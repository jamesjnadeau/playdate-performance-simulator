//! Loads a Playdate device build (`pdex.elf`) and relocates it.
//!
//! `pdex.elf` is linked at address 0 with `--emit-relocs`. The device loads
//! it elsewhere and patches the absolute relocations; we do the same. Only
//! `R_ARM_ABS32`/`R_ARM_TARGET1` words need patching; branch and PC-relative
//! relocations stay valid when the whole image moves.

use goblin::elf::{section_header, Elf};

const R_ARM_ABS32: u32 = 2;
const R_ARM_TARGET1: u32 = 38;

/// A function symbol.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Symbol {
    /// Start address after relocation (without the Thumb bit).
    pub start: u32,
    pub size: u32,
    pub name: String,
}

/// A relocated image ready to copy into emulator memory.
#[derive(Debug, Clone)]
pub struct Image {
    pub base: u32,
    /// Bytes from `base`, including zeroed `.bss`.
    pub bytes: Vec<u8>,
    /// Entry point (`eventHandlerShim`), with the Thumb bit.
    pub entry: u32,
    /// Function symbols, sorted by address.
    pub symbols: Vec<Symbol>,
    /// End of code (`.text`), for the cost hook range.
    pub text_end: u32,
}

impl Image {
    /// The function containing `addr`.
    pub fn symbol_at(&self, addr: u32) -> Option<usize> {
        let a = addr & !1;
        let i = self.symbols.partition_point(|s| s.start <= a);
        if i == 0 {
            return None;
        }
        let s = &self.symbols[i - 1];
        (a < s.start + s.size.max(2)).then_some(i - 1)
    }
}

/// Loads and relocates `data` to `base`.
pub fn load(data: &[u8], base: u32) -> Result<Image, String> {
    let elf = Elf::parse(data).map_err(|e| format!("not an ELF file: {e}"))?;
    if elf.header.e_machine != goblin::elf::header::EM_ARM || elf.is_64 {
        return Err("not a 32-bit ARM ELF (expected a Playdate device build, pdex.elf)".into());
    }
    // Image extent from allocated sections.
    let mut end = 0u32;
    let mut text_end = 0u32;
    for sh in &elf.section_headers {
        if sh.sh_flags as u32 & section_header::SHF_ALLOC != 0 {
            end = end.max((sh.sh_addr + sh.sh_size) as u32);
            if sh.sh_flags as u32 & section_header::SHF_EXECINSTR != 0 {
                text_end = text_end.max((sh.sh_addr + sh.sh_size) as u32);
            }
        }
    }
    let mut bytes = vec![0u8; end as usize];
    for sh in &elf.section_headers {
        if sh.sh_flags as u32 & section_header::SHF_ALLOC != 0 && sh.sh_type != section_header::SHT_NOBITS {
            let off = sh.sh_offset as usize;
            let len = sh.sh_size as usize;
            let src = data.get(off..off + len).ok_or("section outside file")?;
            bytes[sh.sh_addr as usize..sh.sh_addr as usize + len].copy_from_slice(src);
        }
    }
    // Apply relocations that target allocated sections.
    for (idx, rels) in &elf.shdr_relocs {
        let Some(rel_sh) = elf.section_headers.get(*idx) else {
            continue;
        };
        let Some(target) = elf.section_headers.get(rel_sh.sh_info as usize) else {
            continue;
        };
        if target.sh_flags as u32 & section_header::SHF_ALLOC == 0 {
            continue;
        }
        for r in rels.iter() {
            if r.r_type == R_ARM_ABS32 || r.r_type == R_ARM_TARGET1 {
                // With --emit-relocs the offset is the address of the word.
                let at = r.r_offset as usize;
                if at + 4 <= bytes.len() {
                    let v = u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
                    bytes[at..at + 4].copy_from_slice(&v.wrapping_add(base).to_le_bytes());
                }
            }
        }
    }
    let mut symbols: Vec<Symbol> = elf
        .syms
        .iter()
        .filter(|s| {
            s.st_type() == goblin::elf::sym::STT_FUNC && s.st_value != 0
                || (s.st_type() == goblin::elf::sym::STT_FUNC && s.st_size > 0)
        })
        .filter_map(|s| {
            let name = elf.strtab.get_at(s.st_name)?.to_string();
            Some(Symbol {
                start: (s.st_value as u32 & !1) + base,
                size: s.st_size as u32,
                name,
            })
        })
        .collect();
    symbols.sort_by_key(|s| s.start);
    symbols.dedup_by_key(|s| s.start);
    let entry = (elf.header.e_entry as u32).wrapping_add(base) | 1;
    Ok(Image {
        base,
        bytes,
        entry,
        symbols,
        text_end: text_end + base,
    })
}
