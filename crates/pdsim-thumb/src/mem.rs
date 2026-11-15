//! Flat memory regions, bounds-checked little-endian access, the block
//! cache of each code region with its write guard, and the write log.

use crate::decode::Block;
use pdsim_cpu::{CpuError, Region};
use std::cell::Cell;
use std::rc::Rc;

/// Granule of `mem_map` and of the block cache's write guard. Unicorn
/// (QEMU for Arm M-profile) uses 1 KiB target pages, and a translation
/// block never starts in one page and runs past the next.
pub const PAGE: u32 = 1024;

/// Decoded blocks of one executable region, indexed by
/// `(addr - base) / 2`, plus one flag per page that says whether any block
/// starts in it (so a store into data that shares nothing with code costs
/// one flag test).
pub struct BlockCache {
    blocks: Vec<Option<Rc<Block>>>,
    page_has_code: Vec<bool>,
}

impl BlockCache {
    fn new(size: u32) -> BlockCache {
        BlockCache {
            blocks: vec![None; (size / 2) as usize],
            page_has_code: vec![false; size.div_ceil(PAGE) as usize],
        }
    }

    /// Drops every block starting in pages `first..=last` (offsets into the
    /// region).
    fn invalidate(&mut self, first: u32, last: u32) {
        for p in first..=last.min(self.page_has_code.len() as u32 - 1) {
            if std::mem::take(&mut self.page_has_code[p as usize]) {
                let at = (p * PAGE / 2) as usize;
                let end = (at + (PAGE / 2) as usize).min(self.blocks.len());
                self.blocks[at..end].fill(None);
            }
        }
    }
}

/// One mapped region.
pub struct Area {
    pub base: u32,
    /// One past the last byte (`base + size`, which can be 2^32).
    pub end: u64,
    pub kind: Region,
    data: Vec<u8>,
    /// Blocks decoded from this region (Code regions only).
    cache: Option<BlockCache>,
}

impl Area {
    fn contains(&self, addr: u32, len: u32) -> bool {
        addr >= self.base && addr as u64 + len as u64 <= self.end
    }
}

/// Why a game access failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fault {
    ReadUnmapped,
    WriteUnmapped,
    WriteProt,
}

impl Fault {
    /// Unicorn's name for it (`uc_error`'s `Debug`).
    pub fn kind(self) -> &'static str {
        match self {
            Fault::ReadUnmapped => "READ_UNMAPPED",
            Fault::WriteUnmapped => "WRITE_UNMAPPED",
            Fault::WriteProt => "WRITE_PROT",
        }
    }

    pub fn at(self, pc: u32) -> CpuError {
        CpuError {
            kind: self.kind().to_string(),
            pc,
        }
    }
}

/// The address space.
#[derive(Default)]
pub struct Mem {
    areas: Vec<Area>,
    /// Index of the area the last lookup found.
    last: Cell<usize>,
    /// Game stores since the last drain, when recording.
    log: Option<Vec<(u32, Vec<u8>)>>,
}

impl Mem {
    pub fn map(&mut self, addr: u32, size: u32, kind: Region) -> Result<(), CpuError> {
        let arg = || CpuError {
            kind: "ARG".to_string(),
            pc: 0,
        };
        if size == 0 || addr % PAGE != 0 || size % PAGE != 0 {
            return Err(arg());
        }
        let end = addr as u64 + size as u64;
        if end > 1 << 32 {
            return Err(arg());
        }
        if self
            .areas
            .iter()
            .any(|a| (addr as u64) < a.end && end > a.base as u64)
        {
            return Err(CpuError {
                kind: "MAP".to_string(),
                pc: 0,
            });
        }
        self.areas.push(Area {
            base: addr,
            end,
            kind,
            data: vec![0; size as usize],
            cache: (kind == Region::Code).then(|| BlockCache::new(size)),
        });
        Ok(())
    }

    /// The area holding `addr`.
    #[inline]
    pub fn find(&self, addr: u32) -> Option<usize> {
        let last = self.last.get();
        if let Some(a) = self.areas.get(last) {
            if a.contains(addr, 1) {
                return Some(last);
            }
        }
        let i = self.areas.iter().position(|a| a.contains(addr, 1))?;
        self.last.set(i);
        Some(i)
    }

    pub fn area(&self, i: usize) -> &Area {
        &self.areas[i]
    }

    /// The kind of region at `addr`.
    pub fn kind_at(&self, addr: u32) -> Option<Region> {
        self.find(addr).map(|i| self.areas[i].kind)
    }

    /// Reads `buf.len()` bytes, which may span adjacent regions.
    pub fn read(&self, addr: u32, buf: &mut [u8]) -> bool {
        let mut done = 0;
        while done < buf.len() {
            let at = addr.wrapping_add(done as u32);
            let Some(i) = self.find(at) else {
                return false;
            };
            let a = &self.areas[i];
            let off = (at - a.base) as usize;
            let n = (a.data.len() - off).min(buf.len() - done);
            buf[done..done + n].copy_from_slice(&a.data[off..off + n]);
            done += n;
        }
        true
    }

    /// A game load of `N` bytes.
    #[inline]
    pub fn load<const N: usize>(&self, addr: u32) -> Result<[u8; N], Fault> {
        let mut b = [0; N];
        if let Some(i) = self.find(addr) {
            let a = &self.areas[i];
            if a.contains(addr, N as u32) {
                let off = (addr - a.base) as usize;
                b.copy_from_slice(&a.data[off..off + N]);
                return Ok(b);
            }
        }
        if self.read(addr, &mut b) {
            Ok(b)
        } else {
            Err(Fault::ReadUnmapped)
        }
    }

    pub fn ld8(&self, addr: u32) -> Result<u32, Fault> {
        Ok(self.load::<1>(addr)?[0] as u32)
    }
    pub fn ld16(&self, addr: u32) -> Result<u32, Fault> {
        Ok(u16::from_le_bytes(self.load(addr)?) as u32)
    }
    pub fn ld32(&self, addr: u32) -> Result<u32, Fault> {
        Ok(u32::from_le_bytes(self.load(addr)?))
    }
    pub fn ld64(&self, addr: u32) -> Result<u64, Fault> {
        Ok(u64::from_le_bytes(self.load(addr)?))
    }

    /// Fetches an instruction halfword from an executable region.
    #[inline]
    pub fn fetch16(&self, addr: u32) -> Option<u16> {
        let i = self.find(addr)?;
        let a = &self.areas[i];
        if a.kind == Region::Data || !a.contains(addr, 2) {
            return None;
        }
        let off = (addr - a.base) as usize;
        Some(u16::from_le_bytes([a.data[off], a.data[off + 1]]))
    }

    /// Checks that a game store of `len` bytes at `addr` may happen.
    fn check_store(&self, addr: u32, len: u32) -> Result<(), Fault> {
        let mut at = addr;
        let end = addr as u64 + len as u64;
        while (at as u64) < end {
            let Some(i) = self.find(at) else {
                return Err(Fault::WriteUnmapped);
            };
            let a = &self.areas[i];
            if a.kind == Region::Trampoline {
                return Err(Fault::WriteProt);
            }
            if a.end >= end {
                return Ok(());
            }
            at = a.end as u32;
        }
        Ok(())
    }

    /// A game store: faults on unmapped and trampoline memory, logs it
    /// when recording, and drops cached blocks it may have changed.
    #[inline]
    pub fn store(&mut self, addr: u32, bytes: &[u8]) -> Result<(), Fault> {
        let n = bytes.len() as u32;
        if let Some(i) = self.find(addr) {
            let a = &mut self.areas[i];
            if a.kind == Region::Data && a.contains(addr, n) {
                let off = (addr - a.base) as usize;
                a.data[off..off + bytes.len()].copy_from_slice(bytes);
                if let Some(log) = &mut self.log {
                    log.push((addr, bytes.to_vec()));
                }
                return Ok(());
            }
        }
        self.check_store(addr, n)?;
        self.write(addr, bytes);
        if let Some(log) = &mut self.log {
            log.push((addr, bytes.to_vec()));
        }
        Ok(())
    }

    pub fn st8(&mut self, addr: u32, v: u32) -> Result<(), Fault> {
        self.store(addr, &[v as u8])
    }
    pub fn st16(&mut self, addr: u32, v: u32) -> Result<(), Fault> {
        self.store(addr, &(v as u16).to_le_bytes())
    }
    pub fn st32(&mut self, addr: u32, v: u32) -> Result<(), Fault> {
        self.store(addr, &v.to_le_bytes())
    }
    pub fn st64(&mut self, addr: u32, v: u64) -> Result<(), Fault> {
        self.store(addr, &v.to_le_bytes())
    }

    /// Writes anywhere mapped (no protection check), dropping cached
    /// blocks. Returns false if part of the range is unmapped; the mapped
    /// part before it is written.
    pub fn write(&mut self, addr: u32, bytes: &[u8]) -> bool {
        let mut done = 0;
        while done < bytes.len() {
            let at = addr.wrapping_add(done as u32);
            let Some(i) = self.find(at) else {
                return false;
            };
            let a = &mut self.areas[i];
            let off = (at - a.base) as usize;
            let n = (a.data.len() - off).min(bytes.len() - done);
            a.data[off..off + n].copy_from_slice(&bytes[done..done + n]);
            if let Some(c) = &mut a.cache {
                let first = (off as u32 / PAGE).saturating_sub(1);
                let last = (off + n - 1) as u32 / PAGE;
                c.invalidate(first, last);
            }
            done += n;
        }
        true
    }

    /// Whether every byte of `[addr, addr + len)` is mapped.
    pub fn mapped(&self, addr: u32, len: u32) -> bool {
        let mut at = addr as u64;
        let end = addr as u64 + len as u64;
        while at < end {
            let Some(i) = self.find(at as u32) else {
                return false;
            };
            at = self.areas[i].end;
        }
        true
    }

    /// The cached block at `addr` in area `i`.
    #[inline]
    pub fn cached(&self, i: usize, addr: u32) -> Option<Rc<Block>> {
        let a = &self.areas[i];
        a.cache.as_ref()?.blocks[((addr - a.base) / 2) as usize].clone()
    }

    pub fn cache(&mut self, i: usize, addr: u32, b: Rc<Block>) {
        let a = &mut self.areas[i];
        if let Some(c) = &mut a.cache {
            let off = addr - a.base;
            c.page_has_code[(off / PAGE) as usize] = true;
            c.blocks[(off / 2) as usize] = Some(b);
        }
    }

    pub fn set_log(&mut self, on: bool) {
        match (on, self.log.is_some()) {
            (true, false) => self.log = Some(Vec::new()),
            (false, true) => self.log = None,
            _ => {}
        }
    }

    pub fn drain_log(&mut self) -> Vec<(u32, Vec<u8>)> {
        self.log.as_mut().map(std::mem::take).unwrap_or_default()
    }
}
