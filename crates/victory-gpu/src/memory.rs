//! Title memory by address, for translated shaders that read and write
//! through raw 40-bit addresses (`victory_shader::emit::SLOT_MEMORY`: the
//! console's ray-tracing structure builder and the programs that walk its
//! structures; the structures themselves lie in plain title memory, in no
//! buffer object).
//!
//! The copies are the ones every other access uses ([`crate::shadow`]: a
//! title buffer's copy, or the 4 MiB window of memory outside buffers), so
//! a root view and a raw address reach the same host bytes. This module
//! keeps what a shader needs to find them:
//!
//! - the **directory**: per 64 KiB page of title address space the number
//!   of the copy that holds the page (0: none) and the page's byte offset
//!   in it; one buffer for all lists, entries only ever go from empty to
//!   filled;
//! - per view heap a run of slots with a raw unordered access view of each
//!   numbered copy (slot 0 of the run: a null view, for pages without a
//!   copy);
//! - the **faults**: the page numbers shaders found empty, read back here
//!   once a frame, so the copies exist from the next frame on. A pointer a
//!   shader takes out of memory cannot be known before it runs; the
//!   addresses a dispatch is given as root views are made ready before it.
//!
//! **A page that is not there when a shader touches it is a lost access**:
//! the read gives 0, the store is dropped, and the fault note only helps
//! the next frame. For the structure builder that is the same as a dropped
//! stage, so before a dispatch the host makes ready everything it can see
//! the dispatch name: the whole copy around each root view, around each
//! pair of root constants that is a title address and around each root
//! constant that is one in 256-byte units (the builder passes structures
//! all three ways), and the 4 MiB after each (a structure may cross a
//! window's end).
//! shortcut: what a stage reaches only through pointers it reads out of
//! memory (InitHeaders' table of structure addresses, LoadInstances'
//! instance records) is not made ready unless something else named it;
//! the fault count in the log says whether that happens. The whole answer
//! is the extent of the title's allocation around an address, which the
//! memory module of the layer knows and this one does not ask yet.
//!
//! shortcut: a copy that is numbered is kept for the rest of the run, and
//! there are at most [`COPIES`] of them (4 MiB windows mostly: 4 GiB at
//! most). Every dispatch of such a shader brings all numbered copies up to
//! date and, if it can store, declares all of them written (the bytes are
//! compared on the CPU afterwards, as for any unordered access view): a
//! dirty-page mask written by the shader would bound both.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use victory_shader::emit::{MEMORY_FAULTS, MEMORY_PAGES};
use windows::Win32::Graphics::Direct3D12::*;

use crate::descriptor::host::HostHeap;
use crate::host::{HostList, host};
use crate::shadow::Shadow;

/// Copies that can be numbered (the run of heap slots is one longer). A
/// listed frame of the opening cutscene names about 50 windows.
const COPIES: u32 = 1023;
/// A window of memory outside buffers (`shadow.rs`).
const CELL: u64 = 4 << 20;
/// A directory page.
const PAGE: u64 = 1 << 16;

struct Memory {
    /// Buffer, its mapping, its GPU address.
    directory: (ID3D12Resource, *mut u32, u64),
    faults: (ID3D12Resource, *mut u32, u64),
    /// Copy `n` is `copies[n - 1]`; `None`: numbered again ([`Memory::retarget`]).
    copies: Vec<Option<Arc<Shadow>>>,
    /// `crate::resident::generation` at the last [`Memory::retarget`].
    generation: u64,
    /// Per host view heap (by address): the first slot of its run, and how
    /// many copies have their view there.
    heaps: HashMap<usize, (u32, u32)>,
    /// The frame the faults were last read in.
    frame: u64,
    /// Pages asked for this frame that are not the title's memory.
    refused: std::collections::HashSet<u64>,
}

// SAFETY: the mappings are of buffers the struct owns; all access is under
// the mutex.
unsafe impl Send for Memory {}

static MEMORY: Mutex<Option<Memory>> = Mutex::new(None);

/// The directory's offset for title address `at` (in `shadow`): its byte
/// offset from the copy's host buffer's first byte. For title memory in
/// place (`crate::inplace`) the buffer begins on the 64 KiB line, or at an
/// arena's base, before the title memory: `Shadow::offset` adds that bias
/// (design §11, bug 1: the offset from the title memory's first byte was
/// off by the bias).
fn entry_offset(shadow: &Shadow, at: u64) -> u32 {
    shadow.offset(at) as u32
}

/// The bytes of a copy's raw view, from the host buffer's first byte: the
/// title memory's end at its offset (with a bias, `len` alone cut the view
/// `bias` bytes short of the title memory's end).
fn view_bytes(shadow: &Shadow) -> u64 {
    let (start, len) = shadow.span();
    shadow.offset(start) + len
}

/// What a draw binds.
pub(crate) struct Ready {
    pub(crate) directory: u64,
    pub(crate) faults: u64,
    /// The view heap slot of the null view; copy `n`'s view is `n` further.
    pub(crate) base: u32,
}

impl Memory {
    fn new() -> Option<Memory> {
        let host = host()?;
        let make = |dwords: u64, what: &str| {
            let (buffer, data) = host.shared_buffer(dwords * 4)?;
            crate::host::name(&buffer, || format!("title memory by address: {what}"));
            // SAFETY: a fresh mapping of at least that many bytes.
            unsafe { std::ptr::write_bytes(data, 0, (dwords * 4) as usize) };
            // SAFETY: a live buffer.
            let gpu = unsafe { buffer.GetGPUVirtualAddress() };
            Some((buffer, data.cast::<u32>(), gpu))
        };
        tracing::info!("title memory by address: a shader reads or writes through raw addresses");
        Some(Memory {
            directory: make(u64::from(MEMORY_PAGES) * 2, "directory")?,
            faults: make(u64::from(MEMORY_FAULTS), "faults")?,
            copies: Vec::new(),
            generation: 0,
            heaps: HashMap::new(),
            frame: u64::MAX,
            refused: Default::default(),
        })
    }

    /// Gives the page of `address` its copy, if it has none and the memory
    /// is the title's.
    fn map(&mut self, address: u64) {
        let page = address / PAGE;
        // The last entry stays empty: addresses past the directory read it.
        if page >= u64::from(MEMORY_PAGES) - 1 {
            return;
        }
        // SAFETY: inside the directory's mapping.
        let entry = unsafe { self.directory.1.add(page as usize * 2) };
        // SAFETY: as above.
        if unsafe { entry.read() } != 0 {
            return;
        }
        let start = page * PAGE;
        // A shader's pointer, or a constant taken for an address, may be
        // anything: only memory the title has.
        if self.refused.contains(&page) {
            return;
        }
        if !crate::texture::committed(start as *const u8, PAGE as usize) {
            self.refused.insert(page);
            return;
        }
        // The copy a root view at that address would get; a buffer that
        // ends inside the page is passed over for what holds all of it.
        let whole = |s: &Arc<Shadow>| s.offset_in(start).is_some_and(|_| s.left(start) >= PAGE);
        let Some(shadow) = crate::shadow::find(start, 1)
            .filter(whole)
            .or_else(|| crate::shadow::find(start, PAGE).filter(whole))
        else {
            return;
        };
        // SAFETY: a live buffer.
        let flags = unsafe { shadow.buffer.GetDesc() }.Flags;
        if flags.0 & D3D12_RESOURCE_FLAG_ALLOW_UNORDERED_ACCESS.0 == 0 {
            return;
        }
        let n = match self.copies.iter().position(|c| c.as_ref().is_some_and(|c| Arc::ptr_eq(c, &shadow))) {
            Some(n) => n,
            None if self.copies.len() < COPIES as usize => {
                self.copies.push(Some(shadow.clone()));
                let n = self.copies.len() - 1;
                // Every page the copy holds whole, not only the one asked
                // for: a structure is more than 64 KiB.
                // shortcut: a page of a title buffer inside this one that
                // has a copy of its own is given this copy too.
                let (first, len) = shadow.span();
                let mut at = first.next_multiple_of(PAGE);
                while at + PAGE <= first + len && at / PAGE < u64::from(MEMORY_PAGES) - 1 {
                    // SAFETY: inside the directory's mapping.
                    let other = unsafe { self.directory.1.add((at / PAGE) as usize * 2) };
                    // SAFETY: as above; the offset first, as below.
                    unsafe {
                        if other.read() == 0 {
                            other.add(1).write_volatile(entry_offset(&shadow, at));
                            other.write_volatile(n as u32 + 1);
                        }
                    }
                    at += PAGE;
                }
                n
            }
            None => {
                static ONCE: std::sync::Once = std::sync::Once::new();
                ONCE.call_once(|| {
                    tracing::warn!(COPIES, "title memory by address: no number left for another copy; its pages read zeros")
                });
                return;
            }
        };
        let offset = entry_offset(&shadow, start);
        // The offset first: a shader that sees the number sees it too.
        // SAFETY: inside the directory's mapping.
        unsafe {
            entry.add(1).write_volatile(offset);
            entry.write_volatile(n as u32 + 1);
        }
    }

    /// After buffers were retargeted (`crate::resident::retargeted`): each
    /// numbered copy that is a buffer no table hands out any more (in a
    /// resident replaced or gone, or a window in place a resident took
    /// over) gets the copy handed out now under a new number, and its pages
    /// in the directory name that one (the offset first, then the number).
    /// The old number is not used again: lists recorded with it may still
    /// run, and the old copy is held until they have (Stage 2 latency
    /// plan, B1: a numbered copy kept its resident for the whole run).
    fn retarget(&mut self) {
        let now = crate::resident::generation();
        if self.generation == now {
            return;
        }
        self.generation = now;
        for n in 0..self.copies.len() {
            let Some(old) = self.copies[n].clone() else { continue };
            let stale = old.in_resident().map_or_else(|| crate::resident::was_replaced(&old.buffer), |r| !crate::resident::registered(r));
            if !stale {
                continue;
            }
            let (first, len) = old.span();
            let Some(new) = crate::shadow::find(first, len.max(1)).filter(|s| !Arc::ptr_eq(s, &old) && s.offset_in(first).is_some() && s.left(first) >= len) else {
                continue;
            };
            let m = match self.copies.iter().position(|c| c.as_ref().is_some_and(|c| Arc::ptr_eq(c, &new))) {
                Some(m) => m,
                None if self.copies.len() < COPIES as usize => {
                    self.copies.push(Some(new.clone()));
                    self.copies.len() - 1
                }
                None => continue,
            };
            let mut at = first & !(PAGE - 1);
            while at < first + len && at / PAGE < u64::from(MEMORY_PAGES) - 1 {
                // SAFETY: inside the directory's mapping.
                let entry = unsafe { self.directory.1.add((at / PAGE) as usize * 2) };
                // SAFETY: as above; the offset first.
                unsafe {
                    if entry.read() == n as u32 + 1 && new.offset_in(at).is_some() {
                        entry.add(1).write_volatile(entry_offset(&new, at));
                        entry.write_volatile(m as u32 + 1);
                    }
                }
                at += PAGE;
            }
            self.copies[n] = None;
            crate::host::retire(old);
            crate::resident::copy_renumbered();
        }
    }

    /// The pages shaders found empty since the last frame.
    fn take_faults(&mut self) {
        let mut pages = Vec::new();
        for i in 1..MEMORY_FAULTS as usize {
            // SAFETY: inside the faults' mapping; the GPU may be writing,
            // a number missed now is written again next frame.
            let at = unsafe { self.faults.1.add(i) };
            // SAFETY: as above.
            let page = unsafe { at.read_volatile() };
            if page != 0 {
                pages.push(page);
                // SAFETY: as above.
                unsafe { at.write_volatile(0) };
            }
        }
        if !pages.is_empty() {
            // Each is an access that read 0 or was dropped.
            static SAID: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
            if SAID.fetch_add(1, std::sync::atomic::Ordering::Relaxed) < 50 {
                tracing::warn!(
                    pages = pages.len(),
                    first = format_args!("{:#x}", u64::from(pages[0]) * PAGE),
                    "title memory by address: shaders touched pages that had no copy (those accesses read 0 or were dropped); the copies are made now"
                );
            }
        }
        for page in pages {
            self.map(u64::from(page) * PAGE);
        }
    }

    /// What a dispatch names: the copy around `address` and the 4 MiB
    /// after it.
    fn name(&mut self, address: u64) {
        self.map(address);
        self.map((address & !(CELL - 1)) + CELL);
    }
}

/// Before a draw whose shader reads title memory by address: what its
/// root arguments name gets its copies (`addresses`: its root views;
/// `constants`: its root constants, see the module), every numbered copy
/// has its view in `heap`'s run and is brought up to date when `list` is
/// submitted (and, with `writes`, copied back after). None if the host
/// cannot hold it; the draw is then dropped.
pub(crate) fn ready(
    list: &HostList,
    heap: &HostHeap,
    addresses: &[u64],
    constants: &[u32],
    writes: bool,
) -> Option<Ready> {
    let mut guard = MEMORY.lock().unwrap_or_else(|e| e.into_inner());
    if guard.is_none() {
        *guard = Memory::new();
    }
    let m = guard.as_mut()?;
    let frame = crate::frame::current();
    if m.frame != frame {
        m.frame = frame;
        m.refused.clear();
        m.take_faults();
    }
    let numbered = m.copies.len();
    if crate::resident::on() {
        m.retarget();
    }
    for &address in addresses {
        m.name(address);
    }
    // A title address in two constants (low dword, then bits 32 to 39), or
    // in one in 256-byte units. A constant that only looks like one names
    // memory nothing reads; one that is not the title's memory is refused.
    for (i, &c) in constants.iter().enumerate() {
        if let Some(&high) = constants.get(i + 1)
            && (1..=0xFF).contains(&high)
            && c >= 0x1_0000
        {
            m.name(u64::from(c) | u64::from(high) << 32);
        }
        if c >= 0x0100_0000 {
            m.name(u64::from(c) << 8);
        }
    }
    let key = std::ptr::from_ref(heap) as usize;
    let (base, made) = match m.heaps.get(&key) {
        Some(&run) => run,
        None => {
            // Held for the heap's life: not given back with a list's slots.
            let base = heap.dynamic(COPIES + 1)?;
            heap.null_raw_uav_at(base);
            (base, 0)
        }
    };
    for (n, shadow) in m.copies.iter().enumerate().skip(made as usize) {
        match shadow {
            Some(shadow) => heap.raw_uav_at(base + 1 + n as u32, &shadow.buffer, 0, view_bytes(shadow)),
            // Numbered again before this heap had it: no page names it.
            None => heap.null_raw_uav_at(base + 1 + n as u32),
        }
    }
    m.heaps.insert(key, (base, m.copies.len() as u32));
    // Once per list, and for copies numbered since: the builder alone
    // dispatches hundreds of times a frame.
    // shortcut: a copy another list numbers meanwhile is brought up to date
    // by that list's submission, which may run after this one's.
    const READ: usize = usize::MAX - 0x6d;
    const WRITE: usize = usize::MAX - 0x6e;
    let first_read = list.first_use(READ);
    let first_write = writes && list.first_use(WRITE);
    for (n, shadow) in m.copies.iter().enumerate() {
        let Some(shadow) = shadow else { continue };
        let (start, len) = shadow.span();
        if first_read || n >= numbered {
            shadow.refresh(list, start, len);
        }
        if writes && (first_write || n >= numbered) {
            shadow.gpu_writes(list, start, len);
        }
    }
    if first_read {
        list.keep(&m.directory.0);
        list.keep(&m.faults.0);
    }
    Some(Ready {
        directory: m.directory.2,
        faults: m.faults.2,
        base,
    })
}

#[cfg(test)]
mod tests {
    use super::{PAGE, entry_offset, view_bytes};
    use windows::Win32::Graphics::Direct3D12::*;
    use windows::core::Interface;
    use windows_sys::Win32::System::Memory as mem;

    /// Design §11, bug 1: a raw address reaches the title's byte in a copy
    /// in place, whose buffer begins `bias` bytes before the title memory;
    /// and the copy's view reaches its last byte. The software adapter
    /// reads what the directory's offset names.
    #[test]
    fn warp_raw_addresses_into_memory_in_place_take_the_bias() {
        let Some(device) = crate::inplace::tests::warp() else { return };
        if device.cast::<ID3D12Device13>().is_err() {
            return;
        }
        // SAFETY: an allocation of the test's own.
        unsafe {
            let base = mem::VirtualAlloc(std::ptr::null(), 8 << 16, mem::MEM_RESERVE | mem::MEM_COMMIT, mem::PAGE_READWRITE).cast::<u8>();
            let (start, len) = (base as u64 + 0x1230, 3 * PAGE + 0x40);
            for i in 0..len as usize {
                base.add(0x1230 + i).write((i as u8).wrapping_mul(13).wrapping_add(i as u8 >> 3));
            }
            let placed = crate::inplace::place_part(&device, start, len).unwrap();
            let bias = placed.bias;
            assert_eq!(bias, 0x1230);
            let shadow = crate::shadow::Shadow::test_in_place(placed, start, len);
            // The first whole directory page in the copy.
            let page = start.next_multiple_of(PAGE);
            let offset = entry_offset(&shadow, page);
            assert_eq!(u64::from(offset), page - start + bias, "the offset from the buffer's first byte");
            assert_eq!(view_bytes(&shadow), len + bias, "the view reaches the title memory's end");
            let mut rig = crate::inplace::Rig::new(&device).unwrap();
            let got = rig.fetch(&shadow.buffer, u64::from(offset), 4096).unwrap();
            assert_eq!(got, std::slice::from_raw_parts(page as *const u8, 4096), "the page's own bytes");
            let last = rig.fetch(&shadow.buffer, view_bytes(&shadow) - 16, 16).unwrap();
            assert_eq!(last, std::slice::from_raw_parts((start + len - 16) as *const u8, 16), "the last bytes are in the view");
            drop(shadow);
            mem::VirtualFree(base.cast(), 0, mem::MEM_RELEASE);
        }
    }
}
