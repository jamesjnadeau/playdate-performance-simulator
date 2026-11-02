//! The game's heap (`pd->system->realloc`), a first-fit allocator over an
//! emulated memory region.

use std::collections::BTreeMap;

/// Allocator bookkeeping. Memory contents live in the emulator.
#[derive(Debug, Clone)]
pub struct Heap {
    base: u32,
    size: u32,
    free: BTreeMap<u32, u32>,
    used: BTreeMap<u32, u32>,
    pub bytes_in_use: u64,
    pub peak: u64,
}

const ALIGN: u32 = 8;

/// Bytes in front of every block. The device's allocator is newlib's, and
/// newlib's `calloc` (which games link) reads the chunk size from the word
/// just before the pointer to know how much to clear, so that word must be
/// mapped and hold a size. The header keeps the pointer 8-byte aligned.
pub const HEADER: u32 = 8;

impl Heap {
    pub fn new(base: u32, size: u32) -> Heap {
        let mut free = BTreeMap::new();
        free.insert(base, size);
        Heap {
            base,
            size,
            free,
            used: BTreeMap::new(),
            bytes_in_use: 0,
            peak: 0,
        }
    }

    /// Allocates `n` bytes; `None` when out of memory. The returned pointer
    /// is preceded by [`HEADER`] bytes; the caller writes
    /// [`Heap::chunk_word`] into the word before it.
    pub fn alloc(&mut self, n: u32) -> Option<u32> {
        let n = n.max(1).div_ceil(ALIGN) * ALIGN;
        let total = n + HEADER;
        let (&addr, &len) = self.free.iter().find(|(_, l)| **l >= total)?;
        self.free.remove(&addr);
        if len > total {
            self.free.insert(addr + total, len - total);
        }
        let ptr = addr + HEADER;
        self.used.insert(ptr, n);
        self.bytes_in_use += n as u64;
        self.peak = self.peak.max(self.bytes_in_use);
        Some(ptr)
    }

    /// The newlib-style chunk size word for the block at `ptr` (usable size
    /// plus the word itself, with the "previous in use" bit set).
    pub fn chunk_word(&self, ptr: u32) -> u32 {
        self.used.get(&ptr).map_or(0, |n| (n + 4) | 1)
    }

    /// Size of the block at `addr`.
    pub fn size_of(&self, addr: u32) -> Option<u32> {
        self.used.get(&addr).copied()
    }

    /// Frees the block at `addr`. Returns false for an unknown pointer.
    pub fn free(&mut self, addr: u32) -> bool {
        let Some(n) = self.used.remove(&addr) else {
            return false;
        };
        self.bytes_in_use -= n as u64;
        let block = addr - HEADER;
        let mut start = block;
        let mut len = n + HEADER;
        if let Some((&pa, &pl)) = self.free.range(..block).next_back() {
            if pa + pl == block {
                self.free.remove(&pa);
                start = pa;
                len += pl;
            }
        }
        if let Some(&nl) = self.free.get(&(block + n + HEADER)) {
            self.free.remove(&(block + n + HEADER));
            len += nl;
        }
        self.free.insert(start, len);
        true
    }

    /// The heap's range.
    pub fn range(&self) -> (u32, u32) {
        (self.base, self.size)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alloc_free_coalesce() {
        let mut h = Heap::new(0x1000, 96);
        let a = h.alloc(10).unwrap();
        let b = h.alloc(16).unwrap();
        let c = h.alloc(16).unwrap();
        assert_eq!((a, b, c), (0x1008, 0x1020, 0x1038));
        assert_eq!(h.chunk_word(a), (16 + 4) | 1);
        assert!(h.alloc(32).is_none());
        assert!(h.free(b));
        assert!(h.free(a));
        assert_eq!(h.alloc(32), Some(0x1008), "freed neighbours merged");
        assert!(!h.free(0x5000));
        assert_eq!(h.bytes_in_use, 48);
    }
}
