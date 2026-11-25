//! Arenas (the in-place design's Stage 1, `VICTORY_INPLACE=3`): title
//! memory the GPU reaches, opened once as a host heap with one buffer
//! placed over all of it, so that every title buffer and window in it is
//! that buffer at an offset (`arena.gpu + (addr - arena.base)`) instead of
//! a copy, or a heap of its own per buffer (`inplace::place_part`).
//!
//! Two kinds:
//!
//! - **parts** of private title memory (`XMemVirtualAlloc`'s, reported by
//!   the runtime as committed: [`title_memory_committed`]), opened with
//!   `OpenExistingHeapFromAddress1`. The runtime refuses a part of 4 GiB or
//!   more (probe P1 on the card and on the software adapter: "pAddress must
//!   point to a memory region that is 4GiB or less"), so a part is at most
//!   [`PART`] bytes, and a bigger region has parts every [`STRIDE`] bytes
//!   that overlap (P5: overlapping parts open and see each other's writes);
//!   anything up to `PART - STRIDE` bytes lies wholly in one of them, the
//!   lowest that holds it is taken. Opening locks every page (P2: 1 GiB in
//!   173 ms, 4 GiB in 695 ms, all of it in the working set), so a part is
//!   opened on the first GPU use, on a thread of its own when it is bigger
//!   than [`sync_bytes`]; until it is open that memory gets copies as
//!   before;
//! - **sections** of the physical page pool made with `SEC_COMMIT`
//!   (`VICTORY_INPLACE_POOL=1` in the runtime; a section made reserved is
//!   refused), opened through a view of the whole section of the arena's
//!   own ([`SectionView`]) with `OpenExistingHeapFromAddress1`: an offset in
//!   the heap is the offset in the section, and the title's views come and
//!   go over the same pages. Not with `OpenExistingHeapFromFileMapping`: on
//!   the card's driver a section opened so keeps its commit until the PC
//!   restarts, released in order or not (`inplace::tests::
//!   section_heap_commit_comes_back`: 256 MB kept in every case; opened by
//!   the address of a view, none). View memory is in place where
//!   the range is one run of one section (`inplace::Cover::Run`); the GPU
//!   then reads the section's pages, never through a tile mapping (into an
//!   opened heap one removes the device: probe "tiles"). A section must be
//!   below 4 GiB too (P1, P7).
//!
//! shortcut: while a part is open the title cannot decommit, release or
//! protect its pages (error 87, probe R9). [`title_memory_released`] (the
//! runtime's wrappers of the title's `VirtualFree`/`VirtualProtect` call
//! it first) retires every part over the range and lets go of what is in
//! place in all of it; a list still in the air that holds the arena's
//! buffer keeps the pages locked until it is reset, and the call then
//! fails as before (the R9 line counts failures).

use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use windows::Win32::Graphics::Direct3D12::{ID3D12Device, ID3D12Heap, ID3D12Resource};

use crate::inplace::{Cover, From, K64, cover, open, place, private_whole};

/// The most a part or a section heap holds: the runtime's limit is "4 GiB
/// or less" and the heap is cut on 64 KiB.
pub(crate) const PART: u64 = (4 << 30) - K64 as u64;
/// Parts of a bigger region begin this far apart.
pub(crate) const STRIDE: u64 = 2 << 30;

/// Where a title range is for the GPU.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    /// Private title memory in place (a part).
    InPlace,
    /// View memory in place: a run of a committed section.
    Section,
}

/// An opened heap, and the view of a section it was opened on, if any: the
/// view stays mapped while anything holds the heap this way.
#[derive(Clone)]
pub(crate) struct Heap {
    _heap: ID3D12Heap,
    _view: Option<Arc<SectionView>>,
}

impl std::convert::From<ID3D12Heap> for Heap {
    fn from(heap: ID3D12Heap) -> Heap {
        Heap { _heap: heap, _view: None }
    }
}

/// The arena's own view of a whole section, at its address. When the last
/// holder lets go it is unmapped [`VIEW_GRACE`] later, not at once: a
/// submission still in the air may hold the arena's buffer alone.
pub(crate) struct SectionView(usize);

/// How long a view a section's heap was opened on stays mapped after the
/// heap is let go.
const VIEW_GRACE: std::time::Duration = std::time::Duration::from_secs(10);

static VIEWS_GOING: Mutex<Vec<(usize, std::time::Instant)>> = Mutex::new(Vec::new());

impl Drop for SectionView {
    fn drop(&mut self) {
        VIEWS_GOING.lock().unwrap_or_else(|e| e.into_inner()).push((self.0, std::time::Instant::now()));
    }
}

/// Unmaps the views let go more than [`VIEW_GRACE`] ago.
fn unmap_views_gone() {
    let mut going = VIEWS_GOING.lock().unwrap_or_else(|e| e.into_inner());
    going.retain(|&(at, since)| {
        if since.elapsed() < VIEW_GRACE {
            return true;
        }
        // SAFETY: a view of ours, mapped by `map_section`, that nothing
        // holds a heap on any more.
        unsafe { windows_sys::Win32::System::Memory::UnmapViewOfFile(windows_sys::Win32::System::Memory::MEMORY_MAPPED_VIEW_ADDRESS { Value: at as *mut _ }) };
        false
    });
}

/// A read-write view of the first `bytes` of `section`, at an address of
/// its own.
fn map_section(section: usize, bytes: u64) -> Option<SectionView> {
    use windows_sys::Win32::System::Memory as mem;
    unmap_views_gone();
    // SAFETY: the runtime's section handle, alive while the runtime has the
    // section (it tells `title_memory_section_gone` before closing it).
    let view = unsafe { mem::MapViewOfFile(section as windows_sys::Win32::Foundation::HANDLE, mem::FILE_MAP_WRITE, 0, 0, bytes as usize) };
    (!view.Value.is_null()).then(|| SectionView(view.Value as usize))
}

/// One opened heap and the buffer over all of it.
pub(crate) struct Arena {
    /// The title address of the buffer's first byte (a section: none, 0).
    pub(crate) base: u64,
    pub(crate) bytes: u64,
    pub(crate) heap: Heap,
    pub(crate) buffer: ID3D12Resource,
    pub(crate) gpu: u64,
}

// SAFETY: D3D12 objects are free-threaded.
unsafe impl Send for Arena {}
// SAFETY: as above.
unsafe impl Sync for Arena {}

/// What [`resolve`] gives for a title range.
pub(crate) struct Resolved {
    pub(crate) resource: ID3D12Resource,
    pub(crate) heap: Heap,
    /// The range's first byte in `resource`.
    pub(crate) offset: u64,
    /// Its host GPU address.
    pub(crate) gpu: u64,
    /// Bytes from it to the arena's end.
    pub(crate) left: u64,
    pub(crate) kind: Kind,
    /// With [`Kind::Section`]: the section.
    pub(crate) section: Option<usize>,
}

enum Slot {
    /// Being opened on the arenas' thread; its bytes are held in the
    /// budget from the ask on ([`Arenas::reserve`]).
    Opening(u64),
    Open(Arc<Arena>),
    /// The host refused it, it is not committed whole, or over the budget:
    /// copies, as before.
    Refused(Why),
}

/// Why a range is not in place (counted in [`TURNED`], by this order).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Why {
    /// View memory that is not one run of a section, or of a section not
    /// made committed.
    View = 0,
    /// Not in memory the runtime reported committed (`XMemVirtualAlloc`).
    NotTitle,
    /// It spans pages not committed (a hole between the title's commits).
    NotCommitted,
    /// It spans pages of another protection than read-write.
    Protection,
    /// Its run of committed pages cannot hold it in one part (4 GiB - 64
    /// KiB, from a part's start on the 2 GiB grid of the run).
    Size,
    /// The arenas' budget is used up.
    Budget,
    /// The host refused to open the part or section.
    Host,
    /// Its part or section is being opened on the arenas' thread.
    Opening,
}

const WHY_NAMES: [&str; 8] = [
    "view memory not one run of a committed section",
    "not memory the runtime reported committed",
    "spans pages not committed",
    "spans pages of another protection",
    "longer than a part of its run",
    "over the budget",
    "refused by the host",
    "its arena still being opened",
];

/// Ranges turned down since the start, by [`Why`].
pub(crate) static TURNED: [AtomicU64; 8] = [const { AtomicU64::new(0) }; 8];

/// [`TURNED`] for the log: the kinds that turned something down.
pub(crate) fn turned_down() -> String {
    let parts: Vec<String> = WHY_NAMES
        .iter()
        .zip(&TURNED)
        .filter_map(|(name, n)| match n.load(Ordering::Relaxed) {
            0 => None,
            n => Some(format!("{n} {name}")),
        })
        .collect();
    if parts.is_empty() { "none".to_owned() } else { parts.join(", ") }
}

/// A run of pages with one state, as `VirtualQuery` gives them: start,
/// end, and whether committed private read-write memory (else why not).
type Run = (u64, u64, Result<(), Why>);

#[derive(Default)]
pub(crate) struct Arenas {
    /// Committed private title memory by address: bytes.
    regions: BTreeMap<u64, u64>,
    /// Per region (by its start), its runs of pages as Windows has them,
    /// asked when first needed and forgotten when the region changes.
    runs: HashMap<u64, Vec<Run>>,
    /// Parts by (start, bytes).
    parts: BTreeMap<(u64, u64), Slot>,
    /// Section heaps by the section's handle.
    sections: HashMap<usize, Slot>,
    /// Bytes of open parts and sections, and of those being opened on the
    /// arenas' thread.
    locked: u64,
    /// With [`early`]: parts asked early ([`Arenas::ask_early`]) by
    /// (start, bytes), while being opened.
    early: std::collections::BTreeSet<(u64, u64)>,
    /// With [`early`]: commits reported and not asked yet, and whether
    /// the host is up (a first range was resolved): until then commits
    /// wait here, and the host is not made for them.
    early_waiting: Vec<(u64, u64)>,
    host_up: bool,
}

/// Since the start: parts and sections opened, refused, retired; their
/// bytes opened; ranges resolved in place.
pub(crate) static ARENA_STATS: [AtomicU64; 7] = [const { AtomicU64::new(0) }; 7];

/// The runs of pages of `[from, to)` as Windows has them now: one
/// `VirtualQuery` per run.
fn runs_of(from: u64, to: u64) -> Vec<Run> {
    use windows_sys::Win32::System::Memory as mem;
    let mut runs = Vec::new();
    let mut at = from;
    while at < to {
        // SAFETY: plain data filled in by the query.
        let mut info: mem::MEMORY_BASIC_INFORMATION = unsafe { std::mem::zeroed() };
        // SAFETY: a valid out pointer of the right size.
        let n = unsafe { mem::VirtualQuery(at as *const std::ffi::c_void, &mut info, size_of_val(&info)) };
        if n == 0 || info.RegionSize == 0 {
            runs.push((at, to, Err(Why::NotCommitted)));
            break;
        }
        let end = (info.BaseAddress as u64 + info.RegionSize as u64).min(to);
        let state = if info.State != mem::MEM_COMMIT || info.Type != mem::MEM_PRIVATE {
            Err(Why::NotCommitted)
        } else if info.Protect != mem::PAGE_READWRITE {
            Err(Why::Protection)
        } else {
            Ok(())
        };
        runs.push((at, end, state));
        at = end;
    }
    runs
}

/// Of a run `[lo, hi)` of committed pages (both on 64 KiB), the part that
/// holds `[at, end)`: the lowest of the parts every [`STRIDE`] from `lo`,
/// each at most [`PART`] and within the run, that holds it whole.
pub(crate) fn grid_part(lo: u64, hi: u64, at: u64, end: u64) -> Result<(u64, u64), Why> {
    if at < lo || end > hi {
        return Err(Why::NotCommitted);
    }
    let mut start = lo;
    while start <= at {
        let bytes = (hi - start).min(PART) & !(K64 as u64 - 1);
        if bytes > 0 && end <= start + bytes {
            return Ok((start, bytes));
        }
        start += STRIDE;
    }
    Err(Why::Size)
}

impl Arenas {
    /// Committed private title memory `[at, at + len)`, joined with what
    /// it continues or overlaps. The region's runs are asked anew when next
    /// needed (a commit may join two runs: a range over both then opens a
    /// new, longer part beside the older one, which stays for what was
    /// placed in it).
    pub(crate) fn committed(&mut self, at: u64, len: u64) {
        let Some(to) = at.checked_add(len).and_then(|end| end.checked_next_multiple_of(4096)) else {
            return;
        };
        let (mut from, mut to) = (at & !(K64 as u64 - 1), to);
        let touching: Vec<u64> = self.regions.range(..=to).filter(|&(&b, &n)| b + n >= from).map(|(&b, _)| b).collect();
        for base in touching {
            if let Some(n) = self.regions.remove(&base) {
                self.runs.remove(&base);
                from = from.min(base);
                to = to.max(base + n);
            }
        }
        self.regions.insert(from, to - from);
    }

    /// `[at, at + len)` is no longer committed private title memory (or is
    /// about to be protected): the regions forget it, every part over it is
    /// retired. Returns the parts' ranges (what is in place there must go).
    pub(crate) fn released(&mut self, at: u64, len: u64) -> Vec<(u64, u64)> {
        let end = at.saturating_add(len.max(1));
        let hit: Vec<(u64, u64)> = self.regions.range(..end).filter(|&(&b, &n)| b + n > at).map(|(&b, &n)| (b, n)).collect();
        for (base, n) in hit {
            self.regions.remove(&base);
            self.runs.remove(&base);
            if base < at {
                self.regions.insert(base, at - base);
            }
            if base + n > end {
                self.regions.insert(end, base + n - end);
            }
        }
        let parts: Vec<(u64, u64)> = self.parts.keys().filter(|&&(s, n)| s < end && s + n > at).copied().collect();
        for key in &parts {
            self.early.remove(key);
            match self.parts.remove(key) {
                Some(Slot::Open(arena)) => {
                    self.locked -= arena.bytes;
                    ARENA_STATS[2].fetch_add(1, Ordering::Relaxed);
                }
                // What the thread opens now stays closed (`landed`).
                Some(Slot::Opening(bytes)) => self.locked -= bytes,
                _ => {}
            }
        }
        parts
    }

    /// With [`early`]: committed private title memory `[at, at + len)`,
    /// just reported, is asked of the arenas' thread to open now, not at
    /// its first GPU use (a part of 2.2 GB takes 365 ms to open; in run
    /// inplace3-pool 1,320 placements were turned down meanwhile). The
    /// parts are cut from the commit itself, every [`STRIDE`] from its
    /// start up to [`PART`] each (as [`grid_part`] cuts a run), so that
    /// commits made in steps do not open parts over each other; a range
    /// over two commits gets the part of its run at its first use, as
    /// before. A commit of `smallest` bytes or less is opened at its first
    /// use (on the asking thread), and what an open part or one asked
    /// early holds is not asked again. The budget is held from the ask on
    /// ([`Arenas::reserve`]); over it, nothing is asked.
    fn ask_early(&mut self, at: u64, len: u64, smallest: u64, most: u64, request: &mut dyn FnMut(Request)) {
        let k = K64 as u64;
        let (lo, hi) = (at.next_multiple_of(k), at.saturating_add(len) & !(k - 1));
        if hi <= lo || hi - lo <= smallest {
            return;
        }
        let mut start = lo;
        loop {
            let bytes = (hi - start).min(PART);
            let key = (start, bytes);
            if self.open_part(start, start + bytes).is_none()
                && !self.early_opening(start, start + bytes)
                && !self.parts.contains_key(&key)
                && self.reserve(bytes, most).is_ok()
            {
                self.parts.insert(key, Slot::Opening(bytes));
                self.early.insert(key);
                request(Request::Part(start, bytes));
                tracing::info!(
                    start = format_args!("{start:#x}"),
                    mb = bytes >> 20,
                    "arena: committed graphics memory asked of the arenas' thread as the runtime reported it (VICTORY_ARENA_EARLY=1)"
                );
            }
            if start + bytes >= hi {
                break;
            }
            start += STRIDE;
        }
    }

    /// With [`early`], once the host is up: the commits waiting are asked
    /// ([`Arenas::ask_early`]).
    fn ask_waiting(&mut self, smallest: u64, most: u64, request: &mut dyn FnMut(Request)) {
        for (at, len) in std::mem::take(&mut self.early_waiting) {
            self.ask_early(at, len, smallest, most, request);
        }
    }

    /// Whether a part asked early that holds `[at, end)` is being opened.
    fn early_opening(&self, at: u64, end: u64) -> bool {
        self.early.iter().any(|&(s, n)| s <= at && end <= s + n && matches!(self.parts.get(&(s, n)), Some(Slot::Opening(_))))
    }

    /// Whether a part asked early that holds any of `[at, end)` is being
    /// opened.
    fn early_opening_over(&self, at: u64, end: u64) -> bool {
        self.early.iter().any(|&(s, n)| s < end && at < s + n && matches!(self.parts.get(&(s, n)), Some(Slot::Opening(_))))
    }

    /// Holds `bytes` of the budget for a part or section the arenas'
    /// thread is asked to open. The thread opens outside the arenas' lock,
    /// so the budget is taken here, under it: two asks made while the
    /// first opens cannot both pass it. Given back if the opening fails
    /// ([`Arenas::landed`]) or the part is retired meanwhile.
    fn reserve(&mut self, bytes: u64, most: u64) -> Result<(), Why> {
        match self.locked.checked_add(bytes) {
            Some(locked) if locked <= most => {
                self.locked = locked;
                Ok(())
            }
            _ => {
                note_refused(format_args!("{} MB: over the budget ({} MB locked of {} MB)", bytes >> 20, self.locked >> 20, most >> 20));
                Err(Why::Budget)
            }
        }
    }

    /// The part a range `[at, end)` of private title memory lies in. The
    /// title commits its big regions in steps, with holes and other
    /// protections between (the 3.4 GB region of run inplace3-t3 was not
    /// committed whole): a part is cut from the run of committed read-write
    /// pages that holds the range ([`grid_part`]), not from the region.
    fn part_for(&mut self, at: u64, end: u64) -> Result<(u64, u64), Why> {
        let (&base, &n) = self.regions.range(..=at).next_back().filter(|&(&b, &n)| at < b + n).ok_or(Why::NotTitle)?;
        if end > base + n {
            return Err(Why::NotTitle);
        }
        let runs = self.runs.entry(base).or_insert_with(|| runs_of(base, base + n));
        let i = runs.partition_point(|r| r.1 <= at);
        let &(lo, hi, state) = runs.get(i).ok_or(Why::NotCommitted)?;
        state?;
        if end > hi {
            // Why the range ends outside its run: the next run's state.
            return Err(match runs.get(i + 1) {
                Some(&(_, _, Err(why))) => why,
                _ => Why::NotCommitted,
            });
        }
        grid_part(lo.next_multiple_of(K64 as u64), hi & !(K64 as u64 - 1), at, end)
    }

    /// An open part that holds `[at, end)`, if one is open.
    fn open_part(&self, at: u64, end: u64) -> Option<Arc<Arena>> {
        self.parts.range((at.saturating_sub(PART), 0)..=(at, u64::MAX)).find_map(|(&(s, n), slot)| match slot {
            Slot::Open(arena) if s <= at && end <= s + n => Some(arena.clone()),
            _ => None,
        })
    }

    /// Where title memory `[at, at + len)` is in place, if it is: an open
    /// part or section. A part or section not open yet is opened now if it
    /// is at most `sync` bytes, else `request` is given it to open (and the
    /// answer is `None` until it is).
    pub(crate) fn resolve_with(
        &mut self,
        device: &ID3D12Device,
        at: u64,
        len: u64,
        sync: u64,
        most: u64,
        request: &mut dyn FnMut(Request),
    ) -> Option<Resolved> {
        match self.resolve_why(device, at, len, sync, most, request) {
            Ok(resolved) => Some(resolved),
            Err(why) => {
                TURNED[why as usize].fetch_add(1, Ordering::Relaxed);
                None
            }
        }
    }

    /// [`Arenas::resolve_with`], with why not.
    pub(crate) fn resolve_why(
        &mut self,
        device: &ID3D12Device,
        at: u64,
        len: u64,
        sync: u64,
        most: u64,
        request: &mut dyn FnMut(Request),
    ) -> Result<Resolved, Why> {
        let end = at.checked_add(len.max(1)).ok_or(Why::NotTitle)?;
        // View memory: one run of a committed section.
        let covered = cover(at as usize, len.max(1) as usize);
        if covered != Cover::None && !matches!(covered, Cover::Run(..)) {
            return Err(Why::View);
        }
        if let Cover::Run(section, offset) = covered {
            let bytes = crate::inplace::section_bytes(section).ok_or(Why::View)?;
            let arena = match self.sections.get(&section) {
                Some(Slot::Open(arena)) => arena.clone(),
                Some(Slot::Opening(_)) => return Err(Why::Opening),
                Some(Slot::Refused(why)) => return Err(*why),
                None if bytes > sync => {
                    match section_fits(bytes).and_then(|held| self.reserve(held, most).map(|()| held)) {
                        Ok(held) => {
                            self.sections.insert(section, Slot::Opening(held));
                            request(Request::Section(section, bytes));
                            return Err(Why::Opening);
                        }
                        Err(why) => {
                            ARENA_STATS[1].fetch_add(1, Ordering::Relaxed);
                            self.sections.insert(section, Slot::Refused(why));
                            return Err(why);
                        }
                    }
                }
                None => {
                    let slot = self.open_section(device, section, bytes, most);
                    let arena = match &slot {
                        Slot::Open(arena) => Ok(arena.clone()),
                        Slot::Refused(why) => Err(*why),
                        Slot::Opening(_) => Err(Why::Opening),
                    };
                    self.sections.insert(section, slot);
                    arena?
                }
            };
            if offset + len > arena.bytes {
                return Err(Why::Size);
            }
            ARENA_STATS[6].fetch_add(1, Ordering::Relaxed);
            return Ok(Resolved {
                resource: arena.buffer.clone(),
                heap: arena.heap.clone(),
                offset,
                gpu: arena.gpu + offset,
                left: arena.bytes - offset,
                kind: Kind::Section,
                section: Some(section),
            });
        }
        // Private memory: a part that is open already and holds it (an
        // older, shorter part as well as a newer one), else the part of
        // its run of committed pages.
        let arena = match self.open_part(at, end) {
            Some(arena) => arena,
            // A part asked early that holds it is being opened.
            None if self.early_opening(at, end) => return Err(Why::Opening),
            None => {
                let key = self.part_for(at, end)?;
                match self.parts.get(&key) {
                    Some(Slot::Open(arena)) => arena.clone(),
                    Some(Slot::Opening(_)) => return Err(Why::Opening),
                    Some(Slot::Refused(why)) => return Err(*why),
                    None if key.1 > sync => {
                        if let Err(why) = self.reserve(key.1, most) {
                            ARENA_STATS[1].fetch_add(1, Ordering::Relaxed);
                            self.parts.insert(key, Slot::Refused(why));
                            return Err(why);
                        }
                        self.parts.insert(key, Slot::Opening(key.1));
                        request(Request::Part(key.0, key.1));
                        return Err(Why::Opening);
                    }
                    None => {
                        let slot = self.open_part_now(device, key, most);
                        let arena = match &slot {
                            Slot::Open(arena) => Ok(arena.clone()),
                            Slot::Refused(why) => Err(*why),
                            Slot::Opening(_) => Err(Why::Opening),
                        };
                        self.keep(key, slot);
                        arena?
                    }
                }
            }
        };
        ARENA_STATS[6].fetch_add(1, Ordering::Relaxed);
        let offset = at - arena.base;
        Ok(Resolved {
            resource: arena.buffer.clone(),
            heap: arena.heap.clone(),
            offset,
            gpu: arena.gpu + offset,
            left: arena.bytes - offset,
            kind: Kind::InPlace,
            section: None,
        })
    }

    /// Keeps what opening the part `key` gave. Pages found not committed
    /// whole or of another protection at the opening mean the runs asked
    /// before are out of date: they are asked anew, and nothing is kept,
    /// so the next ask cuts its part from the runs as they are then.
    fn keep(&mut self, key: (u64, u64), slot: Slot) {
        if let Slot::Refused(Why::NotCommitted | Why::Protection) = slot {
            self.parts.remove(&key);
            self.runs.clear();
            return;
        }
        self.parts.insert(key, slot);
    }

    /// Opens a part on the asking thread, under the arenas' lock.
    fn open_part_now(&mut self, device: &ID3D12Device, (start, bytes): (u64, u64), most: u64) -> Slot {
        let slot = match self.reserve(bytes, most) {
            Ok(()) => self.held_or_given_back(bytes, open_part(device, start, bytes)),
            Err(why) => Slot::Refused(why),
        };
        counted(&slot);
        slot
    }

    /// Opens a section on the asking thread, under the arenas' lock.
    fn open_section(&mut self, device: &ID3D12Device, section: usize, bytes: u64, most: u64) -> Slot {
        let slot = match section_fits(bytes).and_then(|held| self.reserve(held, most).map(|()| held)) {
            Ok(held) => self.held_or_given_back(held, open_section(device, section, held)),
            Err(why) => Slot::Refused(why),
        };
        counted(&slot);
        slot
    }

    /// The budget held for an opening stays with what opened, and is
    /// given back if nothing did.
    fn held_or_given_back(&mut self, held: u64, slot: Slot) -> Slot {
        if !matches!(slot, Slot::Open(_)) {
            self.locked -= held;
        }
        slot
    }

    /// What the arenas' thread opened. Its bytes were held from the ask
    /// on: kept if it opened, given back if not. A part or section retired
    /// while it was being opened gave them back then, and stays closed.
    fn landed(&mut self, request: Request, slot: Slot) {
        let held = match request {
            Request::Part(start, bytes) => match self.parts.get(&(start, bytes)) {
                Some(&Slot::Opening(held)) => held,
                _ => return,
            },
            Request::Section(section, _) => match self.sections.get(&section) {
                Some(&Slot::Opening(held)) => held,
                _ => return,
            },
        };
        let slot = self.held_or_given_back(held, slot);
        counted(&slot);
        match request {
            Request::Part(start, bytes) => {
                self.early.remove(&(start, bytes));
                self.keep((start, bytes), slot);
            }
            Request::Section(section, _) => {
                self.sections.insert(section, slot);
            }
        }
    }

    /// The section is gone: its heap goes when what is in place in it does.
    pub(crate) fn section_gone(&mut self, section: usize) {
        unmap_views_gone();
        match self.sections.remove(&section) {
            Some(Slot::Open(arena)) => {
                self.locked -= arena.bytes;
                ARENA_STATS[2].fetch_add(1, Ordering::Relaxed);
            }
            Some(Slot::Opening(held)) => self.locked -= held,
            _ => {}
        }
    }

    /// Whether `[at, at + len)` would resolve in place now, without
    /// opening anything.
    pub(crate) fn ready(&self, at: u64, len: u64) -> bool {
        let end = at.saturating_add(len.max(1));
        if let Cover::Run(section, _) = cover(at as usize, len.max(1) as usize) {
            return matches!(self.sections.get(&section), Some(Slot::Open(_)));
        }
        self.open_part(at, end).is_some()
    }

    /// Open parts and sections, and their bytes.
    pub(crate) fn census(&self) -> (u64, u64, u64) {
        let parts = self.parts.values().filter(|s| matches!(s, Slot::Open(_))).count() as u64;
        let sections = self.sections.values().filter(|s| matches!(s, Slot::Open(_))).count() as u64;
        (parts, sections, self.locked)
    }
}

/// What the arenas' thread is asked to open.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Request {
    Part(u64, u64),
    Section(usize, u64),
}

/// Since the start: openings refused (counted once a slot is had, on
/// whichever thread opened it), or opened and their bytes.
fn counted(slot: &Slot) {
    match slot {
        Slot::Open(arena) => {
            ARENA_STATS[0].fetch_add(1, Ordering::Relaxed);
            ARENA_STATS[3].fetch_add(arena.bytes, Ordering::Relaxed);
        }
        _ => {
            ARENA_STATS[1].fetch_add(1, Ordering::Relaxed);
        }
    }
}

/// The bytes of a section's heap (cut on 64 KiB), if it can be opened.
fn section_fits(bytes: u64) -> Result<u64, Why> {
    let bytes = bytes & !(K64 as u64 - 1);
    if bytes > PART {
        note_refused(format_args!("section of {} MB: 4 GiB or more cannot be opened (VICTORY_POOL_SECTION_MB)", bytes >> 20));
        return Err(Why::Size);
    }
    Ok(bytes)
}

/// Opens a part; the budget is the caller's ([`Arenas::reserve`]).
fn open_part(device: &ID3D12Device, start: u64, bytes: u64) -> Slot {
    // The runs were asked when the part was cut; the title may have
    // changed its pages since.
    if !private_whole(start as usize, bytes as usize) {
        note_refused(format_args!("part {start:#x}+{bytes:#x}: its pages changed since they were asked (not committed whole and read-write now)"));
        return Slot::Refused(Why::NotCommitted);
    }
    let started = std::time::Instant::now();
    let made = open(device, From::Part(start as usize, bytes as usize)).and_then(|heap| place(device, &heap, 0, bytes).map(|b| (heap, b)));
    match made {
        Ok((heap, buffer)) => {
            crate::host::name(&buffer, || format!("arena: title memory {start:#x}+{bytes:#x} in place"));
            // SAFETY: a live buffer.
            let gpu = unsafe { buffer.GetGPUVirtualAddress() };
            tracing::info!(
                start = format_args!("{start:#x}"),
                mb = bytes >> 20,
                ms = format_args!("{:.1}", started.elapsed().as_secs_f64() * 1000.0),
                "arena: a part of the title's private memory opened in place"
            );
            Slot::Open(Arc::new(Arena { base: start, bytes, heap: heap.into(), buffer, gpu }))
        }
        Err(e) => {
            note_refused(format_args!("part {start:#x}+{bytes:#x}: the host refused ({e})"));
            Slot::Refused(Why::Host)
        }
    }
}

/// Opens a section's heap of `bytes` ([`section_fits`]); the budget is the
/// caller's.
fn open_section(device: &ID3D12Device, section: usize, bytes: u64) -> Slot {
    let started = std::time::Instant::now();
    let Some(view) = map_section(section, bytes) else {
        note_refused(format_args!("section of {} MB: no view of it ({})", bytes >> 20, std::io::Error::last_os_error()));
        return Slot::Refused(Why::Host);
    };
    let made = open(device, From::Part(view.0, bytes as usize)).and_then(|heap| place(device, &heap, 0, bytes).map(|b| (heap, b)));
    match made {
        Ok((heap, buffer)) => {
            let heap = Heap { _heap: heap, _view: Some(Arc::new(view)) };
            crate::host::name(&buffer, || format!("arena: a section of physical pages, {} MB", bytes >> 20));
            // SAFETY: a live buffer.
            let gpu = unsafe { buffer.GetGPUVirtualAddress() };
            tracing::info!(
                mb = bytes >> 20,
                ms = format_args!("{:.1}", started.elapsed().as_secs_f64() * 1000.0),
                "arena: a section of the title's physical pages opened in place"
            );
            Slot::Open(Arc::new(Arena { base: 0, bytes, heap, buffer, gpu }))
        }
        Err(e) => {
            note_refused(format_args!("section of {} MB: the host refused ({e}; a section made reserved is refused: VICTORY_INPLACE_POOL=1)", bytes >> 20));
            Slot::Refused(Why::Host)
        }
    }
}

fn note_refused(what: std::fmt::Arguments) {
    static SAID: AtomicU64 = AtomicU64::new(0);
    if SAID.fetch_add(1, Ordering::Relaxed) < 16 {
        tracing::warn!("arena not opened: {what}; copies there, as before");
    }
}

static ARENAS: Mutex<Option<Arenas>> = Mutex::new(None);

fn with<T>(f: impl FnOnce(&mut Arenas) -> T) -> T {
    let mut guard = ARENAS.lock().unwrap_or_else(|e| e.into_inner());
    f(guard.get_or_insert_with(Arenas::default))
}

/// The most bytes of arenas open at once (`VICTORY_INPLACE_MB`; by
/// default a quarter of the PC's memory): opening locks every page.
pub(crate) fn most() -> u64 {
    static MOST: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
    *MOST.get_or_init(|| {
        std::env::var("VICTORY_INPLACE_MB")
            .ok()
            .and_then(|v| v.trim().parse::<u64>().ok())
            .map(|mb| mb << 20)
            .unwrap_or_else(|| crate::inplace_probes::commit_mb().2 << 20 >> 2)
    })
}

/// Parts and sections up to this size are opened on the thread that asks
/// (`VICTORY_ARENA_SYNC_MB`, 64: about 11 ms for private memory), bigger
/// ones on the arenas' thread.
fn sync_bytes() -> u64 {
    static SYNC: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
    *SYNC.get_or_init(|| std::env::var("VICTORY_ARENA_SYNC_MB").ok().and_then(|v| v.trim().parse::<u64>().ok()).unwrap_or(64) << 20)
}

/// The arenas' thread: opens what [`resolve`] asked for.
fn worker() -> &'static std::sync::mpsc::Sender<Request> {
    static SENDER: std::sync::OnceLock<std::sync::mpsc::Sender<Request>> = std::sync::OnceLock::new();
    SENDER.get_or_init(|| {
        let (sender, receiver) = std::sync::mpsc::channel::<Request>();
        let _ = std::thread::Builder::new().name("victory-arenas".into()).spawn(move || {
            for request in receiver {
                // The budget was taken with the ask (`Arenas::reserve`).
                let slot = match (crate::host::host(), request) {
                    (None, _) => Slot::Refused(Why::Host),
                    (Some(host), Request::Part(start, bytes)) => open_part(&host.device, start, bytes),
                    (Some(host), Request::Section(section, bytes)) => match section_fits(bytes) {
                        Ok(bytes) => open_section(&host.device, section, bytes),
                        Err(why) => Slot::Refused(why),
                    },
                };
                with(|a| a.landed(request, slot));
            }
        });
        sender
    })
}

/// Title memory `[at, at + len)` in place in an arena, if it can be
/// (`VICTORY_INPLACE=3`).
pub(crate) fn resolve(at: u64, len: u64) -> Option<Resolved> {
    let host = crate::host::host()?;
    let mut asked = Vec::new();
    let resolved = with(|a| {
        if early() && !a.host_up {
            a.host_up = true;
            a.ask_waiting(sync_bytes(), most(), &mut |r| asked.push(r));
        }
        a.resolve_with(&host.device, at, len, sync_bytes(), most(), &mut |r| asked.push(r))
    });
    for request in asked {
        let _ = worker().send(request);
    }
    resolved
}

/// [`resolve`] for a resident (`crate::resident`, `VICTORY_INPLACE=4`):
/// the arena is opened on this thread if it is not open yet, and one the
/// arenas' thread is opening is waited for (five seconds at most). A big
/// title buffer is made a resident or a card copy once, at its first use,
/// which is often the first use of its region; without the wait it would
/// keep a card copy for the run.
pub(crate) fn resolve_waiting(at: u64, len: u64) -> Option<Resolved> {
    let host = crate::host::host()?;
    let asked = std::time::Instant::now();
    loop {
        match with(|a| a.resolve_why(&host.device, at, len, u64::MAX, most(), &mut |_| ())) {
            Ok(resolved) => return Some(resolved),
            Err(Why::Opening) if asked.elapsed() < std::time::Duration::from_secs(5) => std::thread::sleep(std::time::Duration::from_millis(2)),
            Err(why) => {
                TURNED[why as usize].fetch_add(1, Ordering::Relaxed);
                return None;
            }
        }
    }
}

/// Whether `[at, at + len)` is in an open arena now.
pub(crate) fn ready(at: u64, len: u64) -> bool {
    with(|a| a.ready(at, len))
}

/// Open parts, open sections, bytes locked.
pub(crate) fn census() -> (u64, u64, u64) {
    with(|a| a.census())
}

/// The runtime committed private title memory `[at, at + len)` that the
/// GPU may use (`XMemVirtualAlloc`): it may be opened as an arena on first
/// GPU use, or at once with [`early`]. Only memory told here is ever
/// opened as a part.
pub fn title_memory_committed(at: usize, len: usize) {
    if crate::inplace::level() < 3 || len == 0 {
        return;
    }
    let (at, len) = (at as u64, len as u64);
    let mut asked = Vec::new();
    with(|a| {
        a.committed(at, len);
        if early() {
            a.early_waiting.push((at, len));
            if a.host_up {
                a.ask_waiting(sync_bytes(), most(), &mut |r| asked.push(r));
            }
        }
    });
    for request in asked {
        let _ = worker().send(request);
    }
}

/// Whether committed graphics memory is asked of the arenas' thread to
/// open as soon as the runtime reports it, not at its first GPU use
/// ([`Arenas::ask_early`]; `VICTORY_ARENA_EARLY=1`, off by default; with
/// arenas, `VICTORY_INPLACE=3` or more). Read once.
fn early() -> bool {
    static EARLY: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *EARLY.get_or_init(|| std::env::var_os("VICTORY_ARENA_EARLY").is_some_and(|v| v == "1"))
}

/// The title is about to decommit, release or protect `[at, at + len)`
/// (the runtime's wrappers of its own calls): the parts over it are
/// retired and what is in place in all of them is let go, so that the
/// pages are no longer locked when the call is made (unless a list still
/// holds an arena's buffer).
pub fn title_memory_released(at: usize, len: usize) {
    if crate::inplace::level() < 3 {
        return;
    }
    // A part asked early over it that is being opened is waited for (five
    // seconds at most): while it opens its pages are locked, and the
    // title's call would fail (R9). Without [`early`] there is none.
    let (at, len) = (at as u64, len as u64);
    let (end, wait, asked) = (at.saturating_add(len.max(1)), std::time::Duration::from_secs(5), std::time::Instant::now());
    let parts = loop {
        if let Some(parts) = with(|a| (asked.elapsed() >= wait || !a.early_opening_over(at, end)).then(|| a.released(at, len))) {
            break parts;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    };
    let mut gone = 0;
    for (start, bytes) in parts {
        gone += crate::shadow::in_place_let_go(start, bytes, false);
    }
    crate::inplace::let_go_counted(gone);
}

/// The runtime closed a section of physical pages.
pub fn title_memory_section_gone(section: usize) {
    if crate::inplace::level() < 3 {
        return;
    }
    with(|a| a.section_gone(section));
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::core::Interface;
    use windows_sys::Win32::System::Memory as mem;

    const G: u64 = 1 << 30;

    /// The parts of a run of committed pages: one up to 4 GiB - 64 KiB;
    /// beyond, every 2 GiB, and a range takes the lowest part that holds it
    /// whole. Committed regions join what they continue and are cut where
    /// released.
    #[test]
    fn a_range_takes_the_lowest_part_of_its_run_that_holds_it() {
        let base = 0x10_0000_0000u64;
        let run = |n: u64| (base, base + n);
        let (lo, hi) = run(64 << 20);
        assert_eq!(grid_part(lo, hi, base + 0x1000, base + 0x2000), Ok((base, 64 << 20)));
        assert_eq!(grid_part(lo, hi, base + (63 << 20), base + (65 << 20)), Err(Why::NotCommitted), "past the run");
        assert_eq!(grid_part(lo + K64 as u64, hi, base, base + 10), Err(Why::NotCommitted), "before the run");
        // A run of 9 GiB: parts at 0, 2, 4, 6 GiB.
        let (lo, hi) = run(9 * G);
        assert_eq!(grid_part(lo, hi, base + G, base + 3 * G), Ok((base, PART)));
        assert_eq!(grid_part(lo, hi, base + 3 * G, base + 5 * G), Ok((base + 2 * G, PART)));
        assert_eq!(grid_part(lo, hi, base + 7 * G, base + 9 * G), Ok((base + 6 * G, 3 * G)));
        assert_eq!(grid_part(lo, hi, base + G, base + 5 * G), Err(Why::Size), "4 GiB in no part");
        // Any range of 2 GiB - 64 KiB lies in one.
        for k in 0..64u64 {
            let at = base + k * (7 * G / 64);
            let end = (at + STRIDE - K64 as u64).min(base + 9 * G);
            let (s, n) = grid_part(lo, hi, at, end).unwrap_or_else(|e| panic!("no part for {at:#x}: {e:?}"));
            assert!(s <= at && end <= s + n && n <= PART);
        }
        // Regions: joined, then cut where released.
        let mut a = Arenas::default();
        a.committed(base, 64 << 20);
        a.committed(base + (64 << 20), 64 << 20);
        assert_eq!(a.regions.iter().map(|(&b, &n)| (b, n)).collect::<Vec<_>>(), [(base, 128 << 20)]);
        assert!(a.released(base + (32 << 20), 16 << 20).is_empty(), "none open");
        assert_eq!(a.regions.iter().map(|(&b, &n)| (b, n)).collect::<Vec<_>>(), [(base, 32 << 20), (base + (48 << 20), 80 << 20)]);
        // Not reported: not the title's.
        assert_eq!(a.part_for(0x30_0000_0000, 0x30_0000_0010), Err(Why::NotTitle));
    }

    /// Design §11 follow-up (run inplace3-t3: the title's 3.4 GB region was
    /// committed in steps, and a part over the whole region was refused):
    /// a reservation committed in two runs with a hole between. A buffer in
    /// either run is placed in its run's part; one over the hole is turned
    /// down (a copy), and so is one over a page of another protection.
    #[test]
    fn warp_runs_of_committed_pages_are_parts_of_their_own() {
        let Some(device) = crate::inplace::tests::warp() else { return };
        if device.cast::<windows::Win32::Graphics::Direct3D12::ID3D12Device13>().is_err() {
            return;
        }
        // SAFETY: an allocation of the test's own.
        unsafe {
            let bytes = 32 * K64;
            let base = mem::VirtualAlloc(std::ptr::null(), bytes, mem::MEM_RESERVE | crate::inplace::MEM_WRITE_WATCH, mem::PAGE_NOACCESS).cast::<u8>();
            let b = base as u64;
            let page = |k: u64| b + k * K64 as u64;
            // Run one: pages 0..8; a hole of 4 pages; run two: 12..20, of
            // which page 18 is read-only; 20..32 stay reserved.
            assert!(!mem::VirtualAlloc(base.cast(), 8 * K64, mem::MEM_COMMIT, mem::PAGE_READWRITE).is_null());
            assert!(!mem::VirtualAlloc(base.add(12 * K64).cast(), 8 * K64, mem::MEM_COMMIT, mem::PAGE_READWRITE).is_null());
            let mut old = 0;
            assert_ne!(mem::VirtualProtect(base.add(18 * K64).cast(), K64, mem::PAGE_READONLY, &mut old), 0);
            let mut a = Arenas::default();
            // As the runtime reports them: two commits of one reservation.
            a.committed(b, 8 * K64 as u64);
            a.committed(page(12), 8 * K64 as u64);
            let mut asked = Vec::new();
            let mut resolve = |a: &mut Arenas, at: u64, len: u64| a.resolve_why(&device, at, len, u64::MAX, u64::MAX, &mut |r| asked.push(r));
            let one = resolve(&mut a, b + 0x100, 0x2_0000).expect("in run one");
            assert_eq!((one.offset, one.left), (0x100, 8 * K64 as u64 - 0x100));
            let two = resolve(&mut a, page(13) + 8, 0x1_0000).expect("in run two");
            assert_eq!(two.offset, K64 as u64 + 8, "run two's part begins at its first page");
            assert_ne!(one.resource.as_raw(), two.resource.as_raw(), "a part each");
            assert_eq!(resolve(&mut a, page(7), 2 * K64 as u64).err(), Some(Why::NotTitle), "over the hole, which nothing reported");
            // The hole committed but not reported: still not the title's;
            // reported, the region is one, and a range over the old hole
            // gets a part of the longer run, beside the two older parts.
            assert!(!mem::VirtualAlloc(base.add(8 * K64).cast(), 4 * K64, mem::MEM_COMMIT, mem::PAGE_READWRITE).is_null());
            a.committed(page(8), 4 * K64 as u64);
            let joined = resolve(&mut a, page(7), 2 * K64 as u64).expect("over the joined run");
            assert_eq!(joined.offset, 7 * K64 as u64, "a part from the reservation's start");
            assert_eq!(joined.left, 18 * K64 as u64 - 7 * K64 as u64, "up to the read-only page");
            assert_eq!(resolve(&mut a, page(17), 2 * K64 as u64).err(), Some(Why::Protection), "over the read-only page");
            assert_eq!(resolve(&mut a, page(18), 16).err(), Some(Why::Protection));
            // The older parts serve what lies in them.
            let again = resolve(&mut a, b + 0x200, 16).expect("in place");
            assert_eq!(again.resource.as_raw(), one.resource.as_raw(), "an open part that holds it is taken");
            assert_eq!(a.census().0, 3);
            assert!(asked.is_empty());
            // Through each part the GPU reads the title's bytes.
            let mut rig = crate::inplace::Rig::new(&device).unwrap();
            crate::inplace::fill(base.add(13 * K64 + 8), 64, 9);
            let got = rig.fetch(&two.resource, two.offset, 64).unwrap();
            assert!(crate::inplace::same(base.add(13 * K64 + 8), &got));
            crate::inplace::fill(base.add(7 * K64), 2 * K64, 3);
            let got = rig.fetch(&joined.resource, joined.offset, 2 * K64).unwrap();
            assert!(crate::inplace::same(base.add(7 * K64), &got));
            drop((one, two, joined, again));
            drop(a);
            mem::VirtualFree(base.cast(), 0, mem::MEM_RELEASE);
        }
    }

    /// A part opened on the arenas' thread holds its bytes of the budget
    /// from the ask on, under the arenas' lock: a second ask while it opens
    /// cannot pass the budget as well (before, the thread read the bytes
    /// locked, opened outside the lock, and added them only once open).
    /// Opened, the bytes stay; refused, or retired while it opens, they are
    /// given back.
    #[test]
    fn warp_a_part_asked_of_the_thread_holds_its_budget() {
        let Some(device) = crate::inplace::tests::warp() else { return };
        // SAFETY: allocations of the test's own.
        unsafe {
            let bytes = 4 * K64;
            let region = || {
                let at = mem::VirtualAlloc(std::ptr::null(), bytes, mem::MEM_RESERVE | mem::MEM_COMMIT, mem::PAGE_READWRITE);
                assert!(!at.is_null());
                at as u64
            };
            let (one, two, three) = (region(), region(), region());
            let n = bytes as u64;
            let mut a = Arenas::default();
            for at in [one, two, three] {
                a.committed(at, n);
            }
            let most = n + n / 2;
            let asked = std::cell::Cell::new(0);
            let ask = |a: &mut Arenas, at: u64| a.resolve_why(&device, at + 16, 16, 0, most, &mut |_| asked.set(asked.get() + 1)).err();
            assert_eq!(ask(&mut a, one), Some(Why::Opening));
            assert_eq!(a.locked, n, "held from the ask on");
            assert_eq!(ask(&mut a, two), Some(Why::Budget), "the first one's bytes count while it opens");
            assert_eq!(ask(&mut a, one), Some(Why::Opening), "asked once");
            assert_eq!(a.locked, n);
            // The thread's refusal gives the bytes back.
            a.landed(Request::Part(one, n), Slot::Refused(Why::Host));
            assert_eq!(a.locked, 0);
            assert_eq!(ask(&mut a, one), Some(Why::Host), "a refusal is kept");
            // Retired while it opens: given back then, and what the thread
            // opens later stays closed.
            assert_eq!(ask(&mut a, three), Some(Why::Opening));
            assert_eq!(a.locked, n);
            assert_eq!(a.released(three, K64 as u64), [(three, n)]);
            assert_eq!(a.locked, 0);
            a.landed(Request::Part(three, n), Slot::Refused(Why::Host));
            assert_eq!(a.locked, 0);
            assert_eq!(asked.get(), 2, "each opening asked of the thread once");
            // Opened by the thread: the bytes stay with the open part.
            if device.cast::<windows::Win32::Graphics::Direct3D12::ID3D12Device13>().is_ok() {
                a.committed(three, n);
                assert_eq!(ask(&mut a, three), Some(Why::Opening));
                a.landed(Request::Part(three, n), open_part(&device, three, n));
                assert_eq!(a.census(), (1, 0, n));
                assert!(a.resolve_why(&device, three + 16, 16, 0, most, &mut |_| ()).is_ok());
                assert_eq!(a.released(three, n), [(three, n)]);
            }
            assert_eq!(a.locked, 0);
            // The regions stay reserved to the end of the run: released
            // here, their addresses went to tests running beside this one,
            // and about one run in ten ended in an access violation.
        }
    }

    /// `VICTORY_ARENA_EARLY`: a commit is asked of the thread at once, in
    /// parts cut from it, its bytes held; asked once; a small one, or one
    /// over the budget, is not. A range in a part being opened is turned
    /// down as opening, not asked as a part of its run; a release over it
    /// sees it; landed or released, it is no longer waited for.
    #[test]
    fn a_commit_asked_early_is_opened_in_parts_of_its_own() {
        let base = 0x10_0000_0000u64;
        let mut a = Arenas::default();
        a.committed(base, 5 * G);
        let mut asked = Vec::new();
        let most = 16 * G;
        a.ask_early(base, 5 * G, 64 << 20, most, &mut |r| asked.push(r));
        let parts: Vec<(u64, u64)> = asked
            .iter()
            .map(|r| match *r {
                Request::Part(s, n) => (s, n),
                Request::Section(..) => panic!("a section"),
            })
            .collect();
        assert_eq!(parts, [(base, PART), (base + 2 * G, 3 * G)], "every 2 GiB from its start, up to its end");
        assert_eq!(a.locked, PART + 3 * G, "held from the ask on");
        a.ask_early(base + G, G, 64 << 20, most, &mut |r| asked.push(r));
        a.ask_early(base + 5 * G, 64 << 20, 64 << 20, most, &mut |r| asked.push(r));
        a.ask_early(base + 6 * G, G, 64 << 20, PART + 3 * G, &mut |r| asked.push(r));
        assert_eq!(asked.len(), 2, "held already, small, over the budget: not asked");
        assert!(a.early_opening(base + 16, base + 32) && a.early_opening_over(base + 3 * G, base + 5 * G));
        if let Some(device) = crate::inplace::tests::warp() {
            let why = a.resolve_why(&device, base + 16, 16, 0, most, &mut |r| asked.push(r)).err();
            assert_eq!((why, asked.len()), (Some(Why::Opening), 2), "waits for the part asked early");
        }
        a.landed(Request::Part(base, PART), Slot::Refused(Why::Host));
        assert!(!a.early_opening(base + 16, base + 32));
        assert_eq!(a.locked, 3 * G);
        assert_eq!(a.released(base + 3 * G, 16), [(base, PART), (base + 2 * G, 3 * G)]);
        assert_eq!(a.locked, 0);
        assert!(!a.early_opening_over(base, base + 5 * G) && a.early.is_empty());
    }

    /// On the software adapter: a region opened as one arena; two title
    /// buffers in it resolve to the same buffer at their offsets; the GPU
    /// reads and writes the title's bytes through it; released, it is
    /// retired and the title can decommit the pages again.
    #[test]
    fn warp_title_buffers_read_and_write_their_bytes_through_one_arena() {
        let Some(device) = crate::inplace::tests::warp() else { return };
        through_one_arena(&device);
    }

    /// The same on this PC's card (`cargo test -p victory-gpu --
    /// --ignored arena_on_hardware`).
    #[test]
    #[ignore = "opens a device on the card"]
    fn arena_on_hardware() {
        use windows::Win32::Graphics::Direct3D::D3D_FEATURE_LEVEL_12_0;
        let mut device: Option<ID3D12Device> = None;
        // SAFETY: plain device creation on the default adapter.
        unsafe { windows::Win32::Graphics::Direct3D12::D3D12CreateDevice(None, D3D_FEATURE_LEVEL_12_0, &mut device).unwrap() };
        through_one_arena(&device.unwrap());
    }

    fn through_one_arena(device: &ID3D12Device) {
        let device = device.clone();
        if device.cast::<windows::Win32::Graphics::Direct3D12::ID3D12Device13>().is_err() {
            return;
        }
        // SAFETY: an allocation of the test's own.
        unsafe {
            let bytes = 16 * K64;
            let base = mem::VirtualAlloc(std::ptr::null(), bytes, mem::MEM_RESERVE | mem::MEM_COMMIT | crate::inplace::MEM_WRITE_WATCH, mem::PAGE_READWRITE).cast::<u8>();
            let b = base as u64;
            let mut a = Arenas::default();
            let mut asked = Vec::new();
            assert!(a.resolve_with(&device, b + 0x100, 0x100, u64::MAX, u64::MAX, &mut |r| asked.push(r)).is_none(), "not committed: not the title's");
            a.committed(b, bytes as u64);
            let one = a.resolve_with(&device, b + 0x1234, 0x2000, u64::MAX, u64::MAX, &mut |r| asked.push(r)).expect("in place");
            let two = a.resolve_with(&device, b + 5 * K64 as u64 + 8, 0x100, u64::MAX, u64::MAX, &mut |r| asked.push(r)).expect("in place");
            assert!(asked.is_empty(), "opened at once under the size limit");
            assert_eq!(one.resource.as_raw(), two.resource.as_raw(), "one buffer for the region");
            assert_eq!((one.offset, two.offset), (0x1234, 5 * K64 as u64 + 8));
            assert_eq!(one.gpu + (two.offset - one.offset), two.gpu);
            assert_eq!(one.left, bytes as u64 - 0x1234);
            assert_eq!(one.kind, Kind::InPlace);
            assert_eq!(a.census(), (1, 0, bytes as u64));
            let mut rig = crate::inplace::Rig::new(&device).unwrap();
            crate::inplace::fill(base.add(0x1234), 0x2000, 5);
            let got = rig.fetch(&one.resource, one.offset, 0x2000).unwrap();
            assert!(crate::inplace::same(base.add(0x1234), &got), "the GPU reads the title's bytes");
            rig.store(&two.resource, two.offset, &[0xc3; 256]).unwrap();
            assert!(crate::inplace::same(base.add(5 * K64 + 8), &[0xc3; 256]), "the GPU's write is at the title's address");
            // Locked while open (R9), free again once retired and let go.
            assert_eq!(mem::VirtualFree(base.add(9 * K64).cast(), K64, mem::MEM_DECOMMIT), 0, "locked");
            assert_eq!(a.released(b + 9 * K64 as u64, K64 as u64), [(b, bytes as u64)]);
            assert_eq!(a.census(), (0, 0, 0));
            drop((one, two));
            assert_ne!(mem::VirtualFree(base.add(9 * K64).cast(), K64, mem::MEM_DECOMMIT), 0, "free again");
            assert!(a.resolve_with(&device, b + 9 * K64 as u64, 16, u64::MAX, u64::MAX, &mut |_| ()).is_none(), "decommitted: no longer the title's");
            // Over the size limit for opening at once: asked of the thread.
            let mut later = Vec::new();
            assert!(a.resolve_with(&device, b + 0x10, 16, 0, u64::MAX, &mut |r| later.push(r)).is_none());
            assert!(matches!(later.as_slice(), [Request::Part(s, n)] if *s == b && *n == 9 * K64 as u64));
            mem::VirtualFree(base.cast(), 0, mem::MEM_RELEASE);
        }
    }

    /// On the software adapter: view memory that is one run of a committed
    /// section resolves into the section's arena at the section offset, and
    /// the GPU's write shows through the view.
    #[test]
    fn warp_a_run_of_a_committed_section_is_in_place() {
        let Some(device) = crate::inplace::tests::warp() else { return };
        through_a_section(&device, 8 * K64);
    }

    /// The same on this PC's card, with a section of 256 MB: and once let
    /// go, the section's commit comes back (`cargo test -p victory-gpu
    /// --release -- --ignored section_arena_on_hardware --nocapture`).
    #[test]
    #[ignore = "opens a device on the card"]
    fn section_arena_on_hardware() {
        use windows::Win32::Graphics::Direct3D::D3D_FEATURE_LEVEL_12_0;
        let mut device: Option<ID3D12Device> = None;
        // SAFETY: plain device creation on the default adapter.
        unsafe { windows::Win32::Graphics::Direct3D12::D3D12CreateDevice(None, D3D_FEATURE_LEVEL_12_0, &mut device).unwrap() };
        let device = device.unwrap();
        let before = crate::inplace::tests::shared_commit_mb();
        through_a_section(&device, 256 << 20);
        drop(device);
        std::thread::sleep(VIEW_GRACE + std::time::Duration::from_secs(1));
        unmap_views_gone();
        let after = crate::inplace::tests::shared_commit_mb();
        eprintln!("section arena on the card: shared commit {before} -> {after} MB");
        assert!(after < before + 64, "the section's commit stays: {before} -> {after} MB");
    }

    fn through_a_section(device: &ID3D12Device, size: usize) {
        let device = device.clone();
        // SAFETY: the section, placeholder and view of the test's own.
        unsafe {
            let invalid = windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
            let handle = mem::CreateFileMappingW(invalid, std::ptr::null(), mem::PAGE_READWRITE | mem::SEC_COMMIT, 0, size as u32, std::ptr::null());
            assert!(!handle.is_null());
            let view = mem::MapViewOfFile(handle, mem::FILE_MAP_WRITE, 0, (2 * K64) as u32, 4 * K64);
            let at = view.Value as usize;
            crate::inplace::title_memory_section(handle as usize, size as u64, true);
            crate::inplace::title_memory_view(at, 4 * K64, handle as usize, 2 * K64 as u64);
            let mut a = Arenas::default();
            let r = a.resolve_with(&device, at as u64 + 0x40, 0x1000, u64::MAX, u64::MAX, &mut |_| ()).expect("in place");
            assert_eq!(r.kind, Kind::Section);
            assert_eq!(r.offset, 2 * K64 as u64 + 0x40);
            let mut rig = crate::inplace::Rig::new(&device).unwrap();
            rig.store(&r.resource, r.offset, &[0x7e; 64]).unwrap();
            assert!(crate::inplace::same((at + 0x40) as *const u8, &[0x7e; 64]));
            assert_eq!(a.census().1, 1);
            a.section_gone(handle as usize);
            assert_eq!(a.census().1, 0);
            drop(r);
            crate::inplace::views_gone(at, 4 * K64);
            mem::UnmapViewOfFile(view);
            windows_sys::Win32::Foundation::CloseHandle(handle);
        }
    }
}
