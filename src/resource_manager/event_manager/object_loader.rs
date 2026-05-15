//! Custom AOT object-file loader — replaces `libloading`.
//!
//! Loads a PIC `.o` produced by `langc` / `langcd`, maps it into executable
//! memory, applies ELF / MachO / COFF relocations via the `object` crate, and
//! exposes entry-point function pointers by name.  The engine never calls
//! `dlopen` or any system linker — eliminating the 23 ms link step and the
//! ~514 ns PLT-per-call overhead documented in the §8 benchmarks.
//!
//! ## Safety contract
//! Function pointers obtained via [`LoadedObject::fn_ptr`] remain valid for the
//! lifetime of the owning `LoadedObject`.  Callers must not invoke them after
//! the object is dropped.

#![allow(clippy::missing_safety_doc)]

use std::sync::Arc;
use thin_vec::ThinVec;
use object::{
    Object, ObjectSection, ObjectSymbol,
    RelocationKind, RelocationTarget, SectionKind, SymbolSection,
};

// ── OS memory primitives ──────────────────────────────────────────────────────
//
// We need RW pages (to copy and patch code) that we can later flip to RX.
// Using raw libc calls avoids pulling in a new crate — mmap/mprotect/munmap
// are stable POSIX interfaces available on every supported Unix target.

#[cfg(unix)]
mod sys {
    use core::ffi::{c_int, c_void};

    unsafe extern "C" {
        pub fn mmap(
            addr:   *mut c_void,
            len:    usize,
            prot:   c_int,
            flags:  c_int,
            fd:     c_int,
            offset: i64,
        ) -> *mut c_void;
        pub fn mprotect(addr: *mut c_void, len: usize, prot: c_int) -> c_int;
        pub fn munmap(addr: *mut c_void, len: usize) -> c_int;

        // Standard C memory functions LLVM may emit from intrinsic lowering.
        pub fn memset(s: *mut c_void, c: c_int, n: usize) -> *mut c_void;
        pub fn memcpy(dst: *mut c_void, src: *const c_void, n: usize) -> *mut c_void;
        pub fn memmove(dst: *mut c_void, src: *const c_void, n: usize) -> *mut c_void;
    }

    pub const PROT_READ:  c_int = 1;
    pub const PROT_WRITE: c_int = 2;
    pub const PROT_EXEC:  c_int = 4;
    pub const MAP_PRIVATE: c_int = 2;

    #[cfg(target_os = "linux")]
    pub const MAP_ANONYMOUS: c_int = 0x20;
    #[cfg(target_os = "macos")]
    pub const MAP_ANONYMOUS: c_int = 0x1000;
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    pub const MAP_ANONYMOUS: c_int = 0x20; // conservative fallback

    pub const MAP_FAILED: *mut c_void = usize::MAX as *mut c_void;
}

// ── Executable memory region ──────────────────────────────────────────────────

struct MmapRegion {
    ptr: *mut u8,
    len: usize,
}

unsafe impl Send for MmapRegion {}
unsafe impl Sync for MmapRegion {}

impl MmapRegion {
    #[cfg(unix)]
    fn alloc(size: usize) -> Option<Self> {
        debug_assert!(size > 0);
        let len = align_up(size, 4096);
        let ptr = unsafe {
            sys::mmap(
                core::ptr::null_mut(),
                len,
                sys::PROT_READ | sys::PROT_WRITE,
                sys::MAP_PRIVATE | sys::MAP_ANONYMOUS,
                -1,
                0,
            )
        };
        if ptr == sys::MAP_FAILED || ptr.is_null() {
            return None;
        }
        Some(MmapRegion { ptr: ptr as *mut u8, len })
    }

    #[cfg(unix)]
    fn make_exec(&self) -> bool {
        unsafe {
            sys::mprotect(
                self.ptr as _,
                self.len,
                sys::PROT_READ | sys::PROT_EXEC,
            ) == 0
        }
    }
}

impl Drop for MmapRegion {
    fn drop(&mut self) {
        #[cfg(unix)]
        if !self.ptr.is_null() && self.len > 0 {
            unsafe { sys::munmap(self.ptr as _, self.len); }
        }
    }
}

// ── Public API ────────────────────────────────────────────────────────────────

/// A PIC `.o` file loaded into executable memory with all relocations applied.
///
/// Drop this only after all in-flight calls to its function pointers return.
pub struct LoadedObject {
    _region: MmapRegion,
    /// `(name, absolute address)` — sorted ascending by name for binary-search.
    symbols: ThinVec<(Arc<str>, usize)>,
}

unsafe impl Send for LoadedObject {}
unsafe impl Sync for LoadedObject {}

#[derive(Debug)]
pub enum LoadError {
    Io(Arc<str>),
    Parse(Arc<str>),
    Relocation(Arc<str>),
    UndefinedSymbol(Arc<str>),
    Mmap,
}

impl core::fmt::Display for LoadError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            LoadError::Io(s)              => write!(f, "io: {s}"),
            LoadError::Parse(s)           => write!(f, "parse: {s}"),
            LoadError::Relocation(s)      => write!(f, "relocation: {s}"),
            LoadError::UndefinedSymbol(s) => write!(f, "undefined symbol `{s}`"),
            LoadError::Mmap               => write!(f, "mmap failed"),
        }
    }
}

impl LoadedObject {
    pub fn from_file(path: &std::path::Path) -> Result<Self, LoadError> {
        let bytes = std::fs::read(path)
            .map_err(|e| LoadError::Io(Arc::<str>::from(format!("{e}").as_str())))?;
        Self::from_bytes(&bytes)
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, LoadError> {
        let obj = object::File::parse(bytes)
            .map_err(|e| LoadError::Parse(Arc::<str>::from(format!("{e}").as_str())))?;
        load_impl(&obj)
    }

    /// Return the runtime address of `name`, or `None` if not found.
    pub fn symbol(&self, name: &str) -> Option<usize> {
        let i = self.symbols.partition_point(|(n, _)| n.as_ref() < name);
        self.symbols.get(i)
            .filter(|(n, _)| n.as_ref() == name)
            .map(|(_, addr)| *addr)
    }

    /// Transmute the address of `name` to a function pointer of type `F`.
    ///
    /// # Safety
    /// `F` must match the actual function signature, and the `LoadedObject`
    /// must outlive any call through the returned pointer.
    pub unsafe fn fn_ptr<F: Copy>(&self, name: &str) -> Option<F> {
        self.symbol(name).map(|addr| {
            let p = addr as *const ();
            unsafe { core::mem::transmute_copy::<*const (), F>(&p) }
        })
    }
}

// ── Core loader ───────────────────────────────────────────────────────────────

fn load_impl(obj: &object::File<'_>) -> Result<LoadedObject, LoadError> {
    // Step 1: collect loadable sections and compute memory layout.
    // layout[i] = (section_index.0, offset_in_region, data_size)
    let mut layout: ThinVec<(usize, usize, usize)> = ThinVec::new();
    let mut region_size = 0usize;

    for section in obj.sections() {
        if !is_loadable(section.kind()) {
            continue;
        }
        let size = section.size() as usize;
        if size == 0 {
            continue;
        }
        let align = (section.align() as usize).max(16);
        region_size = align_up(region_size, align);
        layout.push((section.index().0, region_size, size));
        region_size += size;
    }

    if region_size == 0 {
        return Ok(LoadedObject {
            _region:  MmapRegion { ptr: core::ptr::null_mut(), len: 0 },
            symbols: ThinVec::new(),
        });
    }

    // Step 2: allocate a single RW anonymous region for all loadable sections.
    let region = MmapRegion::alloc(region_size).ok_or(LoadError::Mmap)?;

    // Step 3: copy section data into the region (BSS stays zero from mmap).
    for section in obj.sections() {
        if !is_loadable(section.kind()) {
            continue;
        }
        let Some(&(_, offset, _)) = layout_entry(&layout, section.index().0) else {
            continue;
        };
        let dst = unsafe { region.ptr.add(offset) };
        if let Ok(data) = section.data() {
            if !data.is_empty() {
                unsafe { core::ptr::copy_nonoverlapping(data.as_ptr(), dst, data.len()) };
            }
        }
    }

    // Step 4: apply relocations for every loadable section.
    for section in obj.sections() {
        if !is_loadable(section.kind()) {
            continue;
        }
        let Some(&(_, sec_offset, _)) = layout_entry(&layout, section.index().0) else {
            continue;
        };
        let sec_base = unsafe { region.ptr.add(sec_offset) };

        for (rel_off, reloc) in section.relocations() {
            let patch = unsafe { sec_base.add(rel_off as usize) };

            let sym_addr_opt = resolve_target(obj, &reloc, &layout, &region)?;
            let Some(sym_addr) = sym_addr_opt else {
                // Relocation targets an unloaded section (e.g. .eh_frame) — skip.
                continue;
            };

            let value = (sym_addr as i64).wrapping_add(reloc.addend());
            apply_reloc(patch, value, reloc.kind(), reloc.size())?;
        }
    }

    // Step 5: flip the region to read+execute.
    if !region.make_exec() {
        return Err(LoadError::Mmap);
    }

    // Step 6: build the exported-symbol table (global + weak, defined symbols).
    let mut symbols: ThinVec<(Arc<str>, usize)> = ThinVec::new();
    for sym in obj.symbols() {
        if sym.is_local() || sym.is_undefined() {
            continue;
        }
        let name = match sym.name() {
            Ok(n) if !n.is_empty() => n,
            _ => continue,
        };
        let addr = match sym.section() {
            SymbolSection::Section(s_idx) => {
                let Some(&(_, off, _)) = layout_entry(&layout, s_idx.0) else {
                    continue;
                };
                region.ptr as usize + off + sym.address() as usize
            }
            SymbolSection::Absolute => sym.address() as usize,
            _ => continue,
        };
        let ins = symbols.partition_point(|(n, _)| n.as_ref() < name);
        symbols.insert(ins, (Arc::from(name), addr));
    }

    Ok(LoadedObject { _region: region, symbols })
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn is_loadable(kind: SectionKind) -> bool {
    matches!(
        kind,
        SectionKind::Text
            | SectionKind::Data
            | SectionKind::ReadOnlyData
            | SectionKind::ReadOnlyDataWithRel
            | SectionKind::UninitializedData
    )
}

fn layout_entry<'a>(
    layout: &'a ThinVec<(usize, usize, usize)>,
    idx: usize,
) -> Option<&'a (usize, usize, usize)> {
    layout.iter().find(|(i, _, _)| *i == idx)
}

fn resolve_target(
    obj: &object::File<'_>,
    reloc: &object::Relocation,
    layout: &ThinVec<(usize, usize, usize)>,
    region: &MmapRegion,
) -> Result<Option<usize>, LoadError> {
    match reloc.target() {
        RelocationTarget::Symbol(sym_idx) => {
            let sym = match obj.symbol_by_index(sym_idx) {
                Ok(s) => s,
                Err(e) => {
                    return Err(LoadError::Relocation(
                        Arc::<str>::from(format!("symbol lookup: {e}").as_str()),
                    ));
                }
            };
            if sym.is_undefined() {
                let name = sym.name().unwrap_or("?");
                resolve_external(name).map(Some)
            } else {
                match sym.section() {
                    SymbolSection::Section(s_idx) => {
                        match layout_entry(layout, s_idx.0) {
                            Some(&(_, off, _)) => {
                                Ok(Some(region.ptr as usize + off + sym.address() as usize))
                            }
                            None => Ok(None), // unloaded section
                        }
                    }
                    SymbolSection::Absolute => Ok(Some(sym.address() as usize)),
                    _ => Ok(None),
                }
            }
        }
        RelocationTarget::Section(s_idx) => {
            match layout_entry(layout, s_idx.0) {
                Some(&(_, off, _)) => Ok(Some(region.ptr as usize + off)),
                None => Ok(None),
            }
        }
        _ => Ok(None),
    }
}

fn resolve_external(name: &str) -> Result<usize, LoadError> {
    #[cfg(unix)]
    match name {
        "memset"  => return Ok(sys::memset  as *const () as usize),
        "memcpy"  => return Ok(sys::memcpy  as *const () as usize),
        "memmove" => return Ok(sys::memmove as *const () as usize),
        _ => {}
    }
    Err(LoadError::UndefinedSymbol(Arc::from(name)))
}

fn apply_reloc(
    patch: *mut u8,
    value: i64,
    kind: RelocationKind,
    size: u8,
) -> Result<(), LoadError> {
    match (kind, size) {
        (RelocationKind::Absolute, 64) => {
            unsafe { (patch as *mut u64).write_unaligned(value as u64) };
        }
        (RelocationKind::Absolute, 32) => {
            unsafe { (patch as *mut u32).write_unaligned(value as u32) };
        }
        (RelocationKind::Relative | RelocationKind::PltRelative, 32) => {
            // ELF R_X86_64_PC32 formula: S + A - P.  `value` is already S + A
            // (caller folded in the addend); the assembler set A to absorb the
            // displacement-field offset (typically -4 for RIP-relative).
            let rel = value.wrapping_sub(patch as i64);
            if !(i32::MIN as i64..=i32::MAX as i64).contains(&rel) {
                return Err(LoadError::Relocation(Arc::<str>::from(
                    "PC-relative offset out of i32 range",
                )));
            }
            unsafe { (patch as *mut i32).write_unaligned(rel as i32) };
        }
        // Skip unsupported relocation types (e.g. .eh_frame GOT/TLS entries).
        _ => {}
    }
    Ok(())
}

fn align_up(n: usize, align: usize) -> usize {
    debug_assert!(align.is_power_of_two());
    n.saturating_add(align - 1) & !(align - 1)
}
