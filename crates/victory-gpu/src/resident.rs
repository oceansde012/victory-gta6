//! Residents (the in-place design's Stage 2, `VICTORY_INPLACE=4`): a big
//! title buffer whose memory lies in an arena (`crate::arena`) is read and
//! written by shaders in a buffer in the card's own memory, `R`, kept in
//! step with the title's memory by the card alone, on the one host queue:
//!
//! - **CPU writes to `R`.** At submission the 64 KiB pages the title wrote
//!   (write watching, the guards: [`Resident::refresh`]) go up from the
//!   arena with `CopyBufferRegion`, in a list of ours run before the
//!   submission's lists ([`prepare`]). Nothing is copied by the CPU, nothing
//!   is staged, nothing is hashed. Whole pages are safe: everything earlier
//!   on the queue, each earlier submission's merge included, has run when
//!   the copy runs, so the arena holds the card's bytes and the title's.
//!   No range is held back because a flight may write it (the #1322 kind of
//!   bug: a page the title wrote always goes up).
//! - **GPU writes to the title.** Every resident has a twin `B` in the
//!   card's memory: what `R` held when the submission's shaders started
//!   (uploads write `R`, then `B` from `R`). After the submission's lists a
//!   compute pass of ours ([`MERGE`]) compares `R` with `B` per dword over
//!   the ranges declared written and stores each dword that differs into
//!   the arena and into `B`: a whole dword as a store, a dword the shaders
//!   changed in part with `InterlockedAnd` and `InterlockedOr` on the
//!   arena (probe P3: bytes the CPU writes meanwhile in the same dword are
//!   kept). It runs in the same submission, before the host fence's
//!   signal, so when the fence is reached the title's memory holds what the
//!   shaders wrote (writes of the card to the write-back pages of an arena
//!   are seen by the CPU once the fence completes). There is no fetch, no
//!   merge on the CPU, no snapshot and no copy-back for a resident.
//!
//! Lists of ours run in their own `ExecuteCommandLists`, before and after
//! the title's: buffers fall back to the common state between them, and no
//! list uses a buffer as copy source and destination without an explicit
//! transition (risk R8; an arena's buffer is only ever a copy source in the
//! upload and only unordered access in the merge).
//!
//! A buffer that does not lie in an arena (or the arena cannot be opened,
//! or the budget is spent) keeps today's copy in the card's memory
//! (`crate::shadow::cards`). Shader writes to residents are declared to the
//! flights as writes in place are (`host::in_place_writes`), so late wait
//! and the relay hold back what reads them until the flight has run.
//!
//! Residents are per address range: a title buffer inside a resident's
//! pages is that resident at an offset, whatever its size, and shares its
//! bytes. Where copies of the same title memory still differ (a buffer in
//! place directly in the arena inside a resident, a resident made before a
//! bigger one), what the card writes to one is merged into the arena and
//! those bytes go up to the others before the next submission
//! ([`prepare`], [`cross_mark`]).
//!
//! shortcut: as with copies in the card's memory, shaders are assumed to
//! write only the ranges declared written; a write outside them stays in
//! `R` until a declared range holds it, or a page the title wrote goes up
//! over it.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak};

use windows::Win32::Graphics::Direct3D12::*;
use windows::core::Interface;

use crate::host::transition;

/// The unit that goes up: the console's page, write watching's unit.
const PAGE: u64 = crate::watch::PAGE as u64;

/// Whether title buffers that would get a copy in the card's memory are
/// residents where their memory lies in an arena (`VICTORY_INPLACE=4`; the
/// levels are cumulative, `crate::inplace::level`).
pub(crate) fn on() -> bool {
    crate::inplace::level() >= 4
}

/// Measurement switches (Stage 2 latency plan, Phase A): each takes one
/// part of the residents' work away, to time what it costs. They make the
/// title's memory wrong; never for play. `VICTORY_RESIDENT_NO_CROSS=1`:
/// nothing goes up again after writes through other copies
/// ([`cross_mark`]); `VICTORY_RESIDENT_NO_MERGE=1`: no merge at all.
fn switch(name: &str) -> bool {
    std::env::var_os(name).is_some_and(|v| v == "1")
}

/// How the merge stores a dword the card changed in part ([`MERGE`]).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Partial {
    /// Two atomics (and, or) that keep every byte the card did not change:
    /// a byte the CPU wrote meanwhile in the same dword stays.
    Atomics,
    /// Stored whole: the card's dword over whatever the CPU wrote in it
    /// meanwhile.
    Whole,
    /// One compare-exchange against `B` (whole if the title's dword is as
    /// it went up), the two atomics only if the CPU changed it.
    Exchange,
    /// The title's dword read and compared with `B`: equal, stored whole;
    /// else the two atomics. Not atomic: a CPU write between the read and
    /// the store is lost.
    Load,
}

/// The merge's ways for ranges of shaders with whole-dword stores only, and
/// for ranges of shaders with narrower ones (`crate::pipeline::StoreWidth`;
/// Stage 2 latency plan, B5). `VICTORY_RESIDENT_PARTIAL`: `flag` (the
/// default: whole stores where the translator saw only whole-dword stores,
/// atomics elsewhere; as on the console, a dword store of the card's
/// replaces the dword, CPU bytes too), `atomics` (atomics everywhere, as
/// before), `cas` ([`Partial::Exchange`] everywhere), `load`
/// ([`Partial::Load`] everywhere), `plain` (whole everywhere: loses CPU
/// bytes next to narrow stores; `VICTORY_RESIDENT_PLAIN_STORES=1` too,
/// measurement only).
fn partial() -> [Partial; 2] {
    static MODE: OnceLock<[Partial; 2]> = OnceLock::new();
    *MODE.get_or_init(|| {
        let named = std::env::var("VICTORY_RESIDENT_PARTIAL").ok();
        let mode = if switch("VICTORY_RESIDENT_PLAIN_STORES") { Some("plain") } else { named.as_deref() };
        match mode {
            None | Some("flag") => [Partial::Whole, Partial::Atomics],
            Some("atomics") => [Partial::Atomics; 2],
            Some("cas") => [Partial::Exchange; 2],
            Some("load") => [Partial::Load; 2],
            Some("plain") => [Partial::Whole; 2],
            Some(other) => {
                tracing::warn!(mode = other, "VICTORY_RESIDENT_PARTIAL: not flag, atomics, cas, load or plain; flag");
                [Partial::Whole, Partial::Atomics]
            }
        }
    })
}

fn no_cross() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| switch("VICTORY_RESIDENT_NO_CROSS"))
}

fn no_merge() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| switch("VICTORY_RESIDENT_NO_MERGE"))
}

/// One owner per title range (Stage 2 latency plan, B1): no resident made
/// beside one over the same memory, and what holds a replaced, forgotten
/// or gone resident's buffer takes the one handed out now
/// ([`retargeted`]); our lists hold nothing once they have run; windows
/// in residents are let go when idle. `VICTORY_RESIDENT_ONE_OWNER=0`: as
/// before (measurement only; slots then go on showing old copies).
pub(crate) fn one_owner() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("VICTORY_RESIDENT_ONE_OWNER").is_none_or(|v| v != "0"))
}

/// A title range in the card's memory, fed from its arena.
pub(crate) struct Resident {
    /// Title memory `[start, start + len)`, on 64 KiB.
    start: u64,
    len: u64,
    /// `R`: what shaders read and write.
    pub(crate) buffer: ID3D12Resource,
    gpu: u64,
    /// `B`: `R`'s bytes as the submission's shaders found them.
    twin: ID3D12Resource,
    /// The arena's buffer, and `start`'s offset and host address in it.
    arena: ID3D12Resource,
    _heap: crate::arena::Heap,
    arena_offset: u64,
    title_gpu: u64,
    /// In a section's arena (view memory): the section.
    section: Option<usize>,
    /// Per page: the write-watch asking after which it last went up (0:
    /// never).
    copied: Mutex<Vec<u64>>,
    /// Offsets of `R` to go up before the next submission's lists, in the
    /// order noted, joined where they touch the last.
    dirty: Mutex<Vec<(u64, u64)>>,
}

// SAFETY: D3D12 objects are free-threaded; the rest is behind locks.
unsafe impl Send for Resident {}
// SAFETY: as above.
unsafe impl Sync for Resident {}

impl Drop for Resident {
    fn drop(&mut self) {
        BYTES.fetch_sub(2 * self.len, Ordering::Relaxed);
        // A slot may still show `R` (it holds the buffer, not us): from
        // here on nothing brings it up to date or merges what is written
        // through it.
        retargeted([&self.buffer]);
    }
}

/// Bumped whenever a host buffer stops being the one the tables hand out
/// for its title memory ([`retargeted`]): what holds such a buffer looks
/// again ([`generation`]).
static GENERATION: AtomicU64 = AtomicU64::new(0);

/// The host buffers (by address) that stopped being handed out for their
/// title memory: `R` of residents replaced, forgotten or gone (taken out
/// when a new resident's `R` is made at the same address). Only those few;
/// a slot that shows one is made again ([`was_replaced`]).
static REPLACED: Mutex<Option<std::collections::HashSet<usize>>> = Mutex::new(None);

/// Since the start: descriptor slots made again because they showed a
/// replaced buffer; copies of title memory by address numbered again;
/// views whose slot showed another buffer than the copy their range finds
/// (read and cleared by the line).
static RETARGETS: [AtomicU64; 5] = [const { AtomicU64::new(0) }; 5];

/// The buffers `gone` stop being handed out for their title memory
/// (Stage 2 latency plan, B1): every table, descriptor slot and copy that
/// holds one takes the buffer handed out now ([`generation`]).
pub(crate) fn retargeted<'a>(gone: impl IntoIterator<Item = &'a ID3D12Resource>) {
    if !one_owner() {
        return;
    }
    let mut set = lock(&REPLACED);
    let set = set.get_or_insert_with(Default::default);
    for buffer in gone {
        set.insert(buffer.as_raw() as usize);
    }
    GENERATION.fetch_add(1, Ordering::Release);
}

/// Changes whenever a buffer is [`retargeted`].
pub(crate) fn generation() -> u64 {
    GENERATION.load(Ordering::Acquire)
}

/// Whether `buffer` is one no table hands out any more ([`retargeted`]).
pub(crate) fn was_replaced(buffer: &ID3D12Resource) -> bool {
    lock(&REPLACED).as_ref().is_some_and(|set| set.contains(&(buffer.as_raw() as usize)))
}

/// Whether `r` is the resident registered for its memory (not replaced,
/// forgotten or gone stale): what a table may hand out.
pub(crate) fn registered(r: &Resident) -> bool {
    lock(&RANGES).get(&r.start).is_some_and(|w| std::ptr::eq(w.as_ptr(), r))
}

/// A descriptor slot was made again: it showed a replaced buffer.
pub(crate) fn slot_remade() {
    RETARGETS[0].fetch_add(1, Ordering::Relaxed);
}

/// A copy of title memory by address was numbered again (`crate::memory`).
pub(crate) fn copy_renumbered() {
    RETARGETS[1].fetch_add(1, Ordering::Relaxed);
}

/// A view's slot showed another buffer than the copy its range finds;
/// `kind`: that copy is in a resident (0), in place (1), or a copy (2).
pub(crate) fn stale_slot(kind: usize) {
    RETARGETS[2 + kind.min(2)].fetch_add(1, Ordering::Relaxed);
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl Resident {
    /// `buffer` and `twin` (`len` bytes each, in the card's memory) for
    /// title memory `[start, start + len)` at `at` in its arena. Everything
    /// goes up before the first submission. The caller has counted the
    /// bytes in [`BYTES`].
    fn new(start: u64, len: u64, buffer: ID3D12Resource, twin: ID3D12Resource, at: crate::arena::Resolved) -> Resident {
        // SAFETY: a live buffer.
        let gpu = unsafe { buffer.GetGPUVirtualAddress() };
        Resident {
            start,
            len,
            buffer,
            gpu,
            twin,
            arena: at.resource,
            _heap: at.heap,
            arena_offset: at.offset,
            title_gpu: at.gpu,
            section: at.section,
            copied: Mutex::new(vec![0; (len / PAGE) as usize]),
            dirty: Mutex::new(vec![(0, len)]),
        }
    }

    /// Whether it holds any of title memory `[at, end)`.
    /// Its title memory's first byte.
    pub(crate) fn start(&self) -> u64 {
        self.start
    }

    /// Its bytes of title memory (`R`'s size).
    pub(crate) fn bytes(&self) -> u64 {
        self.len
    }

    pub(crate) fn overlaps(&self, at: u64, end: u64) -> bool {
        self.start < end && at < self.start + self.len
    }

    /// `R`'s host GPU address.
    pub(crate) fn gpu(&self) -> u64 {
        self.gpu
    }

    /// In a section's arena: the section, and the offset in it of the
    /// title memory at `start + bias`.
    pub(crate) fn section_at(&self, bias: u64) -> Option<(usize, u64)> {
        self.section.map(|s| (s, self.arena_offset + bias))
    }

    /// Whether it still shows the title's memory: view memory the title
    /// mapped anew is no longer the same run of its section.
    fn still_good(&self) -> bool {
        self.section.is_none_or(|s| crate::inplace::still_run(self.start, self.len, s, self.arena_offset))
    }

    /// At submission: the pages of title memory `[from, to)` (cut to the
    /// resident) the title wrote since they last went up are to go up
    /// before the submission's lists, whole. Whether write watching or the
    /// guards could say; if not, every page of the range goes up.
    pub(crate) fn refresh(&self, from: u64, to: u64) -> bool {
        let (from, to) = (from.max(self.start), to.min(self.start + self.len));
        if from >= to {
            return true;
        }
        let (first, last) = ((from - self.start) / PAGE, (to - self.start).div_ceil(PAGE));
        let Some((now, written)) = crate::watch::writes(from as usize, to as usize, true) else {
            self.mark(first * PAGE, last * PAGE);
            return false;
        };
        let base = self.start / PAGE;
        let mut pages = Vec::new();
        {
            let mut copied = lock(&self.copied);
            let mut writes = written.iter().peekable();
            for page in first..last {
                let write = match writes.peek() {
                    Some(&&(p, w)) if p as u64 == base + page => {
                        writes.next();
                        w
                    }
                    _ => 0,
                };
                let at = &mut copied[page as usize];
                if *at == 0 || write > *at {
                    *at = now;
                    pages.push(page);
                }
            }
        }
        for (a, b) in page_runs(&pages) {
            self.mark(a * PAGE, b * PAGE);
        }
        true
    }

    /// Offsets `[from, to)` of `R` go up before the next submission.
    fn mark(&self, from: u64, to: u64) {
        let (from, to) = (from.min(self.len), to.min(self.len));
        if from >= to {
            return;
        }
        let mut dirty = lock(&self.dirty);
        match dirty.last_mut() {
            Some(last) if from <= last.1 && to >= last.0 => *last = (last.0.min(from), last.1.max(to)),
            _ => dirty.push((from, to)),
        }
    }

    /// What is to go up, taken: in order, joined.
    fn take(&self) -> Vec<(u64, u64)> {
        joined(std::mem::take(&mut *lock(&self.dirty)))
    }
}

/// Runs of consecutive page numbers (ascending) as `[first, end)`.
fn page_runs(pages: &[u64]) -> Vec<(u64, u64)> {
    let mut runs: Vec<(u64, u64)> = Vec::new();
    for &page in pages {
        match runs.last_mut() {
            Some(last) if last.1 == page => last.1 += 1,
            _ => runs.push((page, page + 1)),
        }
    }
    runs
}

/// Ranges in order, those that overlap or touch as one.
fn joined(mut ranges: Vec<(u64, u64)>) -> Vec<(u64, u64)> {
    ranges.sort_unstable();
    let mut out: Vec<(u64, u64)> = Vec::with_capacity(ranges.len());
    for (from, to) in ranges {
        match out.last_mut() {
            Some(last) if from <= last.1 => last.1 = last.1.max(to),
            _ => out.push((from, to)),
        }
    }
    out
}

/// Every resident alive (also those only a list of ours or a title list
/// still holds): what the title wrote goes up to each of them.
static ALL: Mutex<Vec<Weak<Resident>>> = Mutex::new(Vec::new());

/// Residents by their first title address, for title buffers in their
/// pages to share them.
static RANGES: Mutex<BTreeMap<u64, Weak<Resident>>> = Mutex::new(BTreeMap::new());

/// Bytes of residents alive, `R` and `B` together.
static BYTES: AtomicU64 = AtomicU64::new(0);

/// Since the last line: bytes that went up from arenas and their ranges;
/// bytes the merge compared and its dispatches; bytes that went up again
/// because another copy of the same memory was written; submissions that
/// had work for residents; of the bytes that went up again, those written
/// in place (buffers and windows in arenas, not residents); of the bytes
/// that went up and of those compared, the ones of residents no longer
/// registered (replaced, forgotten: alive only for what still holds them);
/// of the bytes compared, those of shaders with stores narrower than a
/// dword, and of those the ones of typed stores only.
static STATS: [AtomicU64; 11] = [const { AtomicU64::new(0) }; 11];

/// Since the start: windows in place handed a resident for being hot
/// (`shadow::hot_window`); asks for a new resident that made none (not in
/// an arena, or over a budget: a copy in the card's memory, or in place,
/// as before).
static HOT: [AtomicU64; 2] = [const { AtomicU64::new(0) }; 2];

/// A window in place was handed a resident for being hot.
pub(crate) fn hot_window_made() {
    HOT[0].fetch_add(1, Ordering::Relaxed);
}

/// The most bytes of residents, `R` and `B` together
/// (`VICTORY_RESIDENT_MB`; by default 60% of the card's budget for this
/// process when first asked). Each resident is also made only while the
/// card's memory in use stays under 80% of that budget with it, as copies
/// in the card's memory are (`shadow::card_for`).
fn most() -> u64 {
    static MOST: OnceLock<u64> = OnceLock::new();
    *MOST.get_or_init(|| {
        std::env::var("VICTORY_RESIDENT_MB")
            .ok()
            .and_then(|v| v.trim().parse::<u64>().ok())
            .map(|mb| mb << 20)
            .or_else(|| crate::host::host()?.video_memory().map(|[budget, ..]| budget / 10 * 6))
            .unwrap_or(4 << 30)
    })
}

/// A live resident that holds title memory `[from, to)`, if one does.
/// Residents of view memory the title mapped anew are forgotten here.
fn holder(from: u64, to: u64) -> Option<Arc<Resident>> {
    let mut ranges = lock(&RANGES);
    let mut gone = Vec::new();
    let mut stale = Vec::new();
    let found = ranges.range(..=from).rev().take(64).find_map(|(&start, weak)| {
        let Some(r) = weak.upgrade() else {
            gone.push(start);
            return None;
        };
        if !r.still_good() {
            gone.push(start);
            stale.push(r);
            return None;
        }
        (r.start + r.len >= to).then_some(r)
    });
    for start in gone {
        ranges.remove(&start);
    }
    drop(ranges);
    if !stale.is_empty() {
        retargeted(stale.iter().map(|r| &r.buffer));
    }
    found
}

/// What [`find`] gives a title buffer.
pub(crate) struct Found {
    pub(crate) resident: Arc<Resident>,
    /// The bytes from the resident's first byte to the buffer's.
    pub(crate) bias: u64,
    /// Residents the new one was made over and replaces: the caller lets
    /// go of the buffers in them, whose next ask finds the new one.
    pub(crate) replaced: Vec<Arc<Resident>>,
}

/// The resident title buffer `[start, start + len)` is in: the one that
/// holds its 64 KiB pages if there is one (any buffer); else, with `new`
/// (a buffer that would get a copy in the card's memory), one made for
/// its pages if they lie in an arena and the budget has room ([`make_over`]:
/// over the residents it overlaps too, if it can be). `None`: as before
/// (in place, or a copy).
pub(crate) fn find(start: u64, len: u64, new: bool) -> Option<Found> {
    if !on() {
        return None;
    }
    let from = start & !(PAGE - 1);
    let to = start.checked_add(len.max(1))?.checked_next_multiple_of(PAGE)?;
    if let Some(resident) = holder(from, to) {
        let bias = start - resident.start;
        return Some(Found { resident, bias, replaced: Vec::new() });
    }
    if !new {
        return None;
    }
    let host = crate::host::host()?;
    if !ready(&host.device) {
        return None;
    }
    let made = make_over(from, to, most(), &mut crate::arena::resolve_waiting, &mut |bytes| {
        if let Some([budget, used, ..]) = host.video_memory()
            && used + 2 * bytes > budget / 10 * 8
        {
            return None;
        }
        host.uav_buffer(bytes)
    });
    let Some((resident, replaced)) = made else {
        HOT[1].fetch_add(1, Ordering::Relaxed);
        return None;
    };
    tracing::info!(
        start = format_args!("{:#x}", resident.start),
        mb = resident.len >> 20,
        replaced = replaced.len(),
        "a title buffer is resident in the card's memory, fed from its arena (VICTORY_INPLACE=4)"
    );
    let bias = start - resident.start;
    Some(Found { resident, bias, replaced })
}

/// A resident for title memory `[from, to)` (on 64 KiB) that no resident
/// holds. Residents it overlaps become one with it: the new resident is
/// made over all of them, if that lies in one arena within the budget,
/// and takes their place in the registry; what holds them takes the new
/// one ([`retargeted`]). Else none is made (the buffer stays in place, or
/// gets a copy): no two registered residents hold the same title memory
/// (Stage 2 latency plan, B1; one made over `[from, to)` alone beside
/// them, as before, held 165-181 MB twice and cost cross-marks and
/// merges every submission). With the residents it replaces.
fn make_over(
    from: u64,
    to: u64,
    most: u64,
    resolve: &mut dyn FnMut(u64, u64) -> Option<crate::arena::Resolved>,
    buffer: &mut dyn FnMut(u64) -> Option<ID3D12Resource>,
) -> Option<(Arc<Resident>, Vec<Arc<Resident>>)> {
    let (over, stale): (Vec<Arc<Resident>>, Vec<Arc<Resident>>) =
        lock(&RANGES).range(..to).filter_map(|(_, w)| w.upgrade()).filter(|r| r.overlaps(from, to)).partition(|r| r.still_good());
    // Over memory no longer the same run of its section: out of the
    // registry first.
    if !stale.is_empty() {
        lock(&RANGES).retain(|_, w| !stale.iter().any(|r| std::ptr::eq(w.as_ptr(), Arc::as_ptr(r))));
        retargeted(stale.iter().map(|r| &r.buffer));
    }
    if over.is_empty() {
        return Some((make(from, to - from, most, resolve, buffer)?, Vec::new()));
    }
    let (lo, hi) = over.iter().fold((from, to), |(a, b), r| (a.min(r.start), b.max(r.start + r.len)));
    let Some(union) = make(lo, hi - lo, most, &mut *resolve, &mut *buffer) else {
        // As before B1: one beside them.
        return if one_owner() { None } else { Some((make(from, to - from, most, resolve, buffer)?, Vec::new())) };
    };
    lock(&RANGES).retain(|_, w| !over.iter().any(|r| std::ptr::eq(w.as_ptr(), Arc::as_ptr(r))) || std::ptr::eq(w.as_ptr(), Arc::as_ptr(&union)));
    retargeted(over.iter().map(|r| &r.buffer));
    Some((union, over))
}

/// Whether no two live registered residents hold the same title memory
/// ([`make_over`]'s rule).
#[cfg(test)]
fn disjoint() -> bool {
    let ranges = lock(&RANGES);
    let live: Vec<(u64, u64)> = ranges.values().filter_map(Weak::upgrade).map(|r| (r.start, r.len)).collect();
    overlap_bytes(live.into_iter()) == 0
}

/// A resident for title memory `[from, from + bytes)` (on 64 KiB), if the
/// residents' bytes stay within `most`: `resolve` says where it is in an
/// arena, `buffer` makes `R` and `B`. Registered for buffers in its
/// pages to share.
fn make(
    from: u64,
    bytes: u64,
    most: u64,
    resolve: &mut dyn FnMut(u64, u64) -> Option<crate::arena::Resolved>,
    buffer: &mut dyn FnMut(u64) -> Option<ID3D12Resource>,
) -> Option<Arc<Resident>> {
    // The merge counts in 32 bits from `R`'s first byte.
    if bytes >= crate::arena::PART {
        return None;
    }
    let at = resolve(from, bytes).filter(|r| r.left >= bytes)?;
    if BYTES.fetch_add(2 * bytes, Ordering::Relaxed) + 2 * bytes > most {
        BYTES.fetch_sub(2 * bytes, Ordering::Relaxed);
        return None;
    }
    let (Some(r), Some(b)) = (buffer(bytes), buffer(bytes)) else {
        BYTES.fetch_sub(2 * bytes, Ordering::Relaxed);
        return None;
    };
    crate::host::name(&r, || format!("resident {from:#x}+{bytes:#x}"));
    crate::host::name(&b, || format!("resident {from:#x}+{bytes:#x}, its twin"));
    // A buffer at the address of one retargeted and freed is not that one.
    if let Some(set) = lock(&REPLACED).as_mut() {
        set.remove(&(r.as_raw() as usize));
    }
    let resident = Arc::new(Resident::new(from, bytes, r, b, at));
    lock(&ALL).push(Arc::downgrade(&resident));
    lock(&RANGES).insert(from, Arc::downgrade(&resident));
    Some(resident)
}

/// Title memory `[at, end)` is let go of in place (`shadow::in_place_let_go`:
/// its part retired, or the title maps pages anew): no new title buffer
/// takes a resident over it. With `remap`, residents of view memory stay
/// (they are looked at again when next asked for, [`holder`]). Whether a
/// resident was forgotten (the caller lets go of the buffers in it).
pub(crate) fn forget(at: u64, end: u64, remap: bool) -> bool {
    let mut forgot = Vec::new();
    lock(&RANGES).retain(|_, weak| {
        let Some(r) = weak.upgrade() else {
            return true;
        };
        let keep = !r.overlaps(at, end) || (remap && r.section.is_some());
        if !keep {
            forgot.push(r);
        }
        keep
    });
    if forgot.is_empty() {
        return false;
    }
    retargeted(forgot.iter().map(|r| &r.buffer));
    true
}

/// The merge (a group of 64 threads per 1 KiB of a declared range of `R`,
/// each thread a dword in each of its four 256-byte rows): `from`/`to` are
/// offsets in `R` on 4 bytes; `title` is the arena at `R`'s first byte.
/// A dword of `R` that differs from `B` goes into the title's memory, and
/// into `B`: whole if every byte differs, else as [`Partial`] says (as
/// written here: only the bytes that do, by two atomics, so that the other
/// bytes of the dword stay what they are in the title's memory, which the
/// CPU may write meanwhile; [`merge_source`] puts the others in). `counts`
/// adds, at `slot` (0: ranges of whole-dword stores, 8: narrower), the
/// dwords that differed and how many of them in part.
///
/// A row is 64 dwords side by side, so a group's stores and atomics into
/// the arena go over the bus as runs of whole bytes. With a thread per 16
/// bytes, as the merge was first, each store of a group was 64 dwords 16
/// bytes apart: on the RTX 4090, 16 MB changed took 167 ms to merge whole
/// and 327 ms in part (about 100 and 50 MB/s), 0.7 and 1.6 ms this way
/// (`merge_on_hardware_runs_at_bus_speed`).
pub(crate) const MERGE: &str = r#"
#define RS "RootConstants(num32BitConstants=3, b0), SRV(t0), UAV(u0), UAV(u1), UAV(u2)"
cbuffer Range : register(b0) {
    uint from;
    uint to;
    uint slot;
};
ByteAddressBuffer now : register(t0);
RWByteAddressBuffer twin : register(u0);
RWByteAddressBuffer title : register(u1);
RWByteAddressBuffer counts : register(u2);
groupshared uint changed;
groupshared uint partial;
[RootSignature(RS)]
[numthreads(64, 1, 1)]
void main(uint3 group : SV_GroupID, uint gi : SV_GroupIndex) {
    if (gi == 0u) {
        changed = 0u;
        partial = 0u;
    }
    GroupMemoryBarrierWithGroupSync();
    uint c = 0u;
    uint p = 0u;
    uint at = from + group.x * 1024u + gi * 4u;
    [unroll]
    for (uint k = 0u; k < 4u; k++) {
        uint o = at + k * 256u;
        if (o < to) {
            uint gpu = now.Load(o);
            uint was = twin.Load(o);
            uint diff = gpu ^ was;
            if (diff != 0u) {
                uint mask = ((diff & 0x000000FFu) != 0u ? 0x000000FFu : 0u)
                          | ((diff & 0x0000FF00u) != 0u ? 0x0000FF00u : 0u)
                          | ((diff & 0x00FF0000u) != 0u ? 0x00FF0000u : 0u)
                          | ((diff & 0xFF000000u) != 0u ? 0xFF000000u : 0u);
                if (mask == 0xFFFFFFFFu) {
                    title.Store(o, gpu);
                } else {
                    title.InterlockedAnd(o, gpu | ~mask);
                    title.InterlockedOr(o, gpu & mask);
                    p++;
                }
                twin.Store(o, gpu);
                c++;
            }
        }
    }
    if (c != 0u) InterlockedAdd(changed, c);
    if (p != 0u) InterlockedAdd(partial, p);
    GroupMemoryBarrierWithGroupSync();
    if (gi == 0u && changed != 0u) {
        counts.InterlockedAdd(slot, changed);
        counts.InterlockedAdd(slot + 4u, partial);
    }
}
"#;

/// [`MERGE`]'s store of a dword changed in part, which [`Partial`]
/// replaces.
const ATOMICS: &str = "title.InterlockedAnd(o, gpu | ~mask);
                    title.InterlockedOr(o, gpu & mask);";

/// [`MERGE`] with its partial dwords stored `how`.
fn merge_source(how: Partial) -> String {
    let instead = match how {
        Partial::Atomics => return MERGE.to_owned(),
        Partial::Whole => "title.Store(o, gpu);".to_owned(),
        Partial::Exchange => format!(
            "uint seen;\n                    title.InterlockedCompareExchange(o, was, gpu, seen);\n                    if (seen != was) {{\n                    {ATOMICS}\n                    }}"
        ),
        Partial::Load => format!(
            "if (title.Load(o) == was) {{\n                    title.Store(o, gpu);\n                    }} else {{\n                    {ATOMICS}\n                    }}"
        ),
    };
    assert!(MERGE.contains(ATOMICS));
    MERGE.replace(ATOMICS, &instead)
}

/// The bytes of `R` one dispatch of the merge covers at most (32,768
/// groups of 1 KiB; a dispatch takes 65,535 groups).
const MERGE_CHUNK: u64 = 32 << 20;

/// What goes up to one resident in a submission: offsets of `R`.
pub(crate) struct Upload<'a> {
    pub(crate) resident: &'a Resident,
    pub(crate) ranges: Vec<(u64, u64)>,
}

/// Records the upload: each range from the arena into `R`, then from `R`
/// into `B` (from `R`, not from the arena, so that a CPU write landing
/// between the two copies cannot make them differ). Returns the bytes.
///
/// # Safety
/// `list` is open; the residents are alive until it has run.
pub(crate) unsafe fn record_upload(list: &ID3D12GraphicsCommandList, uploads: &[Upload]) -> u64 {
    let uploads: Vec<&Upload> = uploads.iter().filter(|u| !u.ranges.is_empty()).collect();
    if uploads.is_empty() {
        return 0;
    }
    let mut bytes = 0;
    // SAFETY: the caller's open list; live buffers, the ranges inside them.
    unsafe {
        let to_dest: Vec<D3D12_RESOURCE_BARRIER> =
            uploads.iter().map(|u| transition(&u.resident.buffer, D3D12_RESOURCE_STATE_COMMON, D3D12_RESOURCE_STATE_COPY_DEST)).collect();
        list.ResourceBarrier(&to_dest);
        for u in &uploads {
            let r = u.resident;
            for &(a, b) in &u.ranges {
                list.CopyBufferRegion(&r.buffer, a, &r.arena, r.arena_offset + a, b - a);
                bytes += b - a;
            }
        }
        let to_source: Vec<D3D12_RESOURCE_BARRIER> =
            uploads.iter().map(|u| transition(&u.resident.buffer, D3D12_RESOURCE_STATE_COPY_DEST, D3D12_RESOURCE_STATE_COPY_SOURCE)).collect();
        list.ResourceBarrier(&to_source);
        for u in &uploads {
            for &(a, b) in &u.ranges {
                list.CopyBufferRegion(&u.resident.twin, a, &u.resident.buffer, a, b - a);
            }
        }
    }
    bytes
}

/// What the merge needs beside the residents: its pipeline and the
/// counters (in the card's memory, copied after each merge into memory the
/// CPU reads).
pub(crate) struct Pass {
    /// Per class of range (whole-dword stores, narrower: [`partial`]),
    /// the merge's root signature and pipeline.
    ways: [(ID3D12RootSignature, ID3D12PipelineState); 2],
    counts: ID3D12Resource,
    seen: ID3D12Resource,
    seen_at: *const u32,
}

// SAFETY: D3D12 objects and a persistent mapping that is only read.
unsafe impl Send for Pass {}

impl Pass {
    pub(crate) fn new(device: &ID3D12Device) -> windows::core::Result<Pass> {
        Pass::made(device, partial())
    }

    /// With the ways `ways` for ranges of whole-dword stores and of
    /// narrower ones.
    pub(crate) fn made(device: &ID3D12Device, ways: [Partial; 2]) -> windows::core::Result<Pass> {
        let whole = crate::inplace_probes::compute(device, &merge_source(ways[0]))?;
        let narrow = if ways[1] == ways[0] { whole.clone() } else { crate::inplace_probes::compute(device, &merge_source(ways[1]))? };
        let counts = committed(device, D3D12_HEAP_TYPE_DEFAULT, 256)?;
        let seen = committed(device, D3D12_HEAP_TYPE_CUSTOM, 256)?;
        let mut at = std::ptr::null_mut();
        // SAFETY: a CPU-visible buffer of ours, mapped for its life.
        unsafe { seen.Map(0, None, Some(&mut at))? };
        crate::host::name(&counts, || "residents: the merge's counters".into());
        Ok(Pass { ways: [whole, narrow], counts, seen, seen_at: at.cast() })
    }

    /// The counters as the last merge that ran left them: dwords that
    /// differed, of them those that differed in part, in ranges of
    /// whole-dword stores, then the same in ranges of narrower ones (all
    /// wrap).
    pub(crate) fn counts(&self) -> [u32; 4] {
        // SAFETY: the mapping holds 256 bytes; the card writes them whole.
        unsafe { std::array::from_fn(|i| self.seen_at.add(i).read_volatile()) }
    }
}

/// A buffer of ours of `size` bytes, unordered access allowed: in the
/// card's memory (`DEFAULT`), or in the PC's, write-back, for the CPU to
/// map (`CUSTOM`).
fn committed(device: &ID3D12Device, kind: D3D12_HEAP_TYPE, size: u64) -> windows::core::Result<ID3D12Resource> {
    let heap = if kind == D3D12_HEAP_TYPE_CUSTOM {
        D3D12_HEAP_PROPERTIES {
            Type: D3D12_HEAP_TYPE_CUSTOM,
            CPUPageProperty: D3D12_CPU_PAGE_PROPERTY_WRITE_BACK,
            MemoryPoolPreference: D3D12_MEMORY_POOL_L0,
            ..Default::default()
        }
    } else {
        D3D12_HEAP_PROPERTIES { Type: kind, ..Default::default() }
    };
    let mut out: Option<ID3D12Resource> = None;
    let desc = crate::inplace::buffer_desc(size, D3D12_RESOURCE_FLAG_ALLOW_UNORDERED_ACCESS);
    // SAFETY: plain creation.
    unsafe { device.CreateCommittedResource(&heap, D3D12_HEAP_FLAG_NONE, &desc, D3D12_RESOURCE_STATE_COMMON, None, &mut out)? };
    out.ok_or_else(windows::core::Error::empty)
}

/// What the merge does for one resident in a submission: offsets of `R`,
/// in order, apart, on 4 bytes; whether shaders with stores narrower than
/// a dword may have written them ([`partial`]).
pub(crate) struct Merge<'a> {
    pub(crate) resident: &'a Resident,
    pub(crate) ranges: Vec<(u64, u64)>,
    pub(crate) narrow: bool,
}

/// Records the merge of `merges`. Returns the bytes compared and the
/// dispatches.
///
/// # Safety
/// `list` is open and runs after the lists whose shader writes it merges,
/// in an `ExecuteCommandLists` of its own; the residents are alive until it
/// has run.
pub(crate) unsafe fn record_merge(list: &ID3D12GraphicsCommandList, pass: &Pass, merges: &[Merge]) -> (u64, u64) {
    let (mut bytes, mut dispatches) = (0, 0);
    // SAFETY: the caller's open list; live buffers; the merge's own root
    // signature; every offset lies in `R`, `B` and the arena.
    unsafe {
        list.ResourceBarrier(&[transition(&pass.counts, D3D12_RESOURCE_STATE_COMMON, D3D12_RESOURCE_STATE_UNORDERED_ACCESS)]);
        let mut way = None;
        for m in merges {
            let class = usize::from(m.narrow);
            if way != Some(class) {
                let (root, pso) = &pass.ways[class];
                list.SetComputeRootSignature(root);
                list.SetPipelineState(pso);
                list.SetComputeRootUnorderedAccessView(4, pass.counts.GetGPUVirtualAddress());
                way = Some(class);
            }
            let r = m.resident;
            list.SetComputeRootShaderResourceView(1, r.gpu);
            list.SetComputeRootUnorderedAccessView(2, r.twin.GetGPUVirtualAddress());
            list.SetComputeRootUnorderedAccessView(3, r.title_gpu);
            for &(a, b) in &m.ranges {
                let mut at = a;
                while at < b {
                    let end = (at + MERGE_CHUNK).min(b);
                    let constants = [at as u32, end as u32, 8 * class as u32];
                    list.SetComputeRoot32BitConstants(0, 3, constants.as_ptr().cast(), 0);
                    list.Dispatch((end - at).div_ceil(1024) as u32, 1, 1);
                    dispatches += 1;
                    at = end;
                }
                bytes += b - a;
            }
        }
        list.ResourceBarrier(&[transition(&pass.counts, D3D12_RESOURCE_STATE_UNORDERED_ACCESS, D3D12_RESOURCE_STATE_COPY_SOURCE)]);
        list.CopyBufferRegion(&pass.seen, 0, &pass.counts, 0, 16);
    }
    (bytes, dispatches)
}

/// A list of ours with the residents its run needs alive, and the value
/// of the host's fence after which it may be used again.
struct Spare {
    allocator: ID3D12CommandAllocator,
    list: ID3D12GraphicsCommandList,
    after: u64,
    kept: Vec<Arc<Resident>>,
}

/// What the host keeps for residents.
struct Work {
    pass: Pass,
    /// Lists free once the host's fence passes their value.
    spare: Vec<Spare>,
    /// The lists of the submission being prepared ([`sent`] gives them
    /// their value).
    out: Vec<Spare>,
    /// The counters as the last line read them.
    read: [u32; 4],
}

// SAFETY: D3D12 objects, used under the lock.
unsafe impl Send for Work {}

static WORK: Mutex<Option<Work>> = Mutex::new(None);

/// Whether the merge could be made (at the first ask); a resident is made
/// only if it could.
fn ready(device: &ID3D12Device) -> bool {
    static READY: OnceLock<bool> = OnceLock::new();
    *READY.get_or_init(|| match Pass::new(device) {
        Ok(pass) => {
            *lock(&WORK) = Some(Work { pass, spare: Vec::new(), out: Vec::new(), read: [0; 4] });
            true
        }
        Err(e) => {
            tracing::warn!(error = %e, "no residents: the merge could not be made (VICTORY_INPLACE=4)");
            false
        }
    })
}

impl Work {
    /// A list of ours, open, whose last run is done (`completed` is the
    /// host fence's value), holding `kept` until its next run is done. A
    /// list that cannot be had ends the run: what went up or was written
    /// would be lost.
    fn take(&mut self, device: &ID3D12Device, completed: u64, kept: Vec<Arc<Resident>>) -> ID3D12GraphicsCommandList {
        for spare in &mut self.spare {
            if spare.after <= completed {
                spare.kept.clear();
            }
        }
        let spare = match self.spare.iter().position(|s| s.after <= completed) {
            Some(at) => Ok(self.spare.swap_remove(at)),
            // SAFETY: plain object creation.
            None => unsafe {
                (|| {
                    let allocator: ID3D12CommandAllocator = device.CreateCommandAllocator(D3D12_COMMAND_LIST_TYPE_DIRECT)?;
                    let list: ID3D12GraphicsCommandList = device.CreateCommandList(0, D3D12_COMMAND_LIST_TYPE_DIRECT, &allocator, None)?;
                    list.Close()?;
                    Ok(Spare { allocator, list, after: 0, kept: Vec::new() })
                })()
            },
        };
        // SAFETY: the list's last run is done.
        let spare = spare.and_then(|s| unsafe { s.allocator.Reset().and_then(|()| s.list.Reset(&s.allocator, None)).map(|()| s) });
        match spare {
            Ok(spare) => {
                let list = spare.list.clone();
                self.out.push(Spare { kept, ..spare });
                list
            }
            Err(e) => crate::host::fail(format_args!("no list of ours for residents ({e}, VICTORY_INPLACE=4)")),
        }
    }
}

/// A title range `[from, to)` the shaders of a submission may write in
/// `resident`, and the narrowest store of those shaders.
pub(crate) struct Declared {
    pub(crate) resident: Arc<Resident>,
    pub(crate) from: u64,
    pub(crate) to: u64,
    pub(crate) width: crate::pipeline::StoreWidth,
}

/// A resident's declared ranges as offsets of `R`, on 4 bytes, joined
/// ([`classes`]).
struct Classes {
    resident: Arc<Resident>,
    /// Written by shaders with whole-dword stores only, outside `narrow`.
    whole: Vec<(u64, u64)>,
    /// Written by shaders with narrower stores, or typed ones.
    narrow: Vec<(u64, u64)>,
    /// The bytes of `narrow` that only typed stores make narrow.
    typed: u64,
}

/// `declared` by resident and class: where ranges of both classes meet,
/// the narrow one holds the bytes (a partial dword there may be the CPU's
/// in part).
fn classes(declared: &[Declared]) -> Vec<Classes> {
    use crate::pipeline::StoreWidth;
    let mut by: Vec<(Arc<Resident>, [Vec<(u64, u64)>; 3])> = Vec::new();
    for d in declared {
        let r = &d.resident;
        let (a, b) = (d.from.saturating_sub(r.start).min(r.len) & !3, d.to.saturating_sub(r.start).min(r.len).next_multiple_of(4));
        if a >= b {
            continue;
        }
        let at = match by.iter().position(|(k, _)| Arc::ptr_eq(k, r)) {
            Some(at) => at,
            None => {
                by.push((r.clone(), Default::default()));
                by.len() - 1
            }
        };
        let class = match d.width {
            StoreWidth::Dwords => 0,
            StoreWidth::Typed => 1,
            StoreWidth::Bytes => 2,
        };
        by[at].1[class].push((a, b));
    }
    by.into_iter()
        .map(|(resident, [whole, typed, bytes])| {
            let (whole, narrow, typed) = split(whole, typed, bytes);
            Classes { resident, whole, narrow, typed }
        })
        .collect()
}

/// One resident's ranges of whole-dword, typed and narrow stores, as
/// [`Classes`] holds them: whole outside narrow, narrow (typed with it),
/// joined; and the bytes only typed stores make narrow.
fn split(whole: Vec<(u64, u64)>, typed: Vec<(u64, u64)>, bytes: Vec<(u64, u64)>) -> (Vec<(u64, u64)>, Vec<(u64, u64)>, u64) {
    let bytes = joined(bytes);
    let typed = minus(joined(typed), &bytes);
    let typed_only = typed.iter().map(|(a, b)| b - a).sum();
    let narrow = joined([bytes, typed].concat());
    (minus(joined(whole), &narrow), narrow, typed_only)
}

/// `ranges` without `out` (both in order and apart).
fn minus(ranges: Vec<(u64, u64)>, out: &[(u64, u64)]) -> Vec<(u64, u64)> {
    let mut left = Vec::new();
    for (mut a, b) in ranges {
        for &(c, d) in out {
            if d <= a || c >= b {
                continue;
            }
            if c > a {
                left.push((a, c));
            }
            a = a.max(d);
            if a >= b {
                break;
            }
        }
        if a < b {
            left.push((a, b));
        }
    }
    left
}

/// What [`prepare`] made for a submission: its lists, closed, to run
/// before and after the submission's lists, each in an
/// `ExecuteCommandLists` of its own.
#[derive(Default)]
pub(crate) struct Prepared {
    pub(crate) upload: Option<ID3D12CommandList>,
    pub(crate) merge: Option<ID3D12CommandList>,
}

/// At submission, after its refreshes (under the submission lock): what
/// went up to residents since the last submission is recorded to run
/// before its lists, and the merge of `declared` (title ranges its shaders
/// may write in residents) to run after them. `in_place` are the title
/// ranges its shaders may write in arenas directly (buffers and windows in
/// place); they and `declared` are marked to go up, before the next
/// submission, to every other resident over them. `completed` is the host
/// fence's value; [`sent`] follows with the value signalled after the
/// submission.
pub(crate) fn prepare(device: &ID3D12Device, completed: u64, declared: &[Declared], in_place: &[(u64, u64)]) -> Prepared {
    let mut out = Prepared::default();
    let mut work = lock(&WORK);
    let Some(work) = work.as_mut() else {
        return out;
    };
    // Lists whose run is done hold nothing any more: a resident held only
    // by one of them is not live (it took uploads and cross-marks, and
    // never died).
    for spare in work.spare.iter_mut().filter(|_| one_owner()) {
        if spare.after <= completed {
            spare.kept.clear();
        }
    }
    let live: Vec<Arc<Resident>> = {
        let mut all = lock(&ALL);
        all.retain(|w| w.strong_count() > 0);
        all.iter().filter_map(Weak::upgrade).collect()
    };
    if live.is_empty() {
        return out;
    }
    let registered: Vec<*const Resident> = lock(&RANGES).values().map(Weak::as_ptr).collect();
    let unregistered = |r: &Resident| !registered.contains(&std::ptr::from_ref(r));
    let mut ups: Vec<Upload> = Vec::new();
    let mut up_kept: Vec<Arc<Resident>> = Vec::new();
    for r in &live {
        let ranges = r.take();
        if !ranges.is_empty() {
            ups.push(Upload { resident: r, ranges });
            up_kept.push(r.clone());
        }
    }
    // Per resident, the declared ranges as offsets, joined, on 4 bytes
    // (dispatches declare the same ranges thousands of times), by class.
    let by_class = if no_merge() { Vec::new() } else { classes(declared) };
    let mut merges: Vec<Merge> = Vec::new();
    let mut merge_kept: Vec<Arc<Resident>> = Vec::new();
    let (mut narrow_bytes, mut typed_bytes) = (0, 0);
    for c in &by_class {
        merge_kept.push(c.resident.clone());
        narrow_bytes += c.narrow.iter().map(|(a, b)| b - a).sum::<u64>();
        typed_bytes += c.typed;
    }
    for (c, narrow) in by_class.iter().flat_map(|c| [(c, false), (c, true)]) {
        let ranges = if narrow { c.narrow.clone() } else { c.whole.clone() };
        if !ranges.is_empty() {
            merges.push(Merge { resident: &c.resident, ranges, narrow });
        }
    }
    // Whole first, then narrow: one change of pipeline.
    merges.sort_by_key(|m| m.narrow);
    STATS[9].fetch_add(narrow_bytes, Ordering::Relaxed);
    STATS[10].fetch_add(typed_bytes, Ordering::Relaxed);
    if !ups.is_empty() {
        let list = work.take(device, completed, up_kept);
        // SAFETY: an open list of ours; the residents are kept with it.
        let bytes = unsafe { record_upload(&list, &ups) };
        STATS[0].fetch_add(bytes, Ordering::Relaxed);
        let stale: u64 = ups.iter().filter(|u| unregistered(u.resident)).flat_map(|u| u.ranges.iter().map(|(a, b)| b - a)).sum();
        STATS[7].fetch_add(stale, Ordering::Relaxed);
        STATS[1].fetch_add(ups.iter().map(|u| u.ranges.len() as u64).sum(), Ordering::Relaxed);
        out.upload = Some(close(&list, "the upload to residents"));
    }
    if !merges.is_empty() {
        let list = work.take(device, completed, merge_kept);
        // SAFETY: an open list of ours, run after the submission's lists;
        // the residents are kept with it.
        let (bytes, dispatches) = unsafe { record_merge(&list, &work.pass, &merges) };
        STATS[2].fetch_add(bytes, Ordering::Relaxed);
        STATS[3].fetch_add(dispatches, Ordering::Relaxed);
        let stale: u64 = merges.iter().filter(|m| unregistered(m.resident)).flat_map(|m| m.ranges.iter().map(|(a, b)| b - a)).sum();
        STATS[8].fetch_add(stale, Ordering::Relaxed);
        out.merge = Some(close(&list, "the merge of residents"));
    }
    // What this submission writes reaches the arena by its end (in place,
    // or merged); every other resident over it takes those bytes again
    // before the next submission (this one's upload is taken already).
    let mut written: Vec<(Option<&Resident>, u64, u64)> =
        merges.iter().flat_map(|m| m.ranges.iter().map(|&(a, b)| (Some(m.resident), m.resident.start + a, m.resident.start + b))).collect();
    written.extend(joined(in_place.to_vec()).into_iter().map(|(from, to)| (None, from, to)));
    let [by_place, by_merge] = if no_cross() { [0, 0] } else { cross_mark(&live, &written) };
    STATS[4].fetch_add(by_place + by_merge, Ordering::Relaxed);
    STATS[6].fetch_add(by_place, Ordering::Relaxed);
    if out.upload.is_some() || out.merge.is_some() {
        STATS[5].fetch_add(1, Ordering::Relaxed);
    }
    out
}

/// The unit in which what another copy wrote goes up to a resident
/// ([`cross_mark`]): the shaders' declared ranges, widened to it.
const MARK: u64 = 256;

/// Title ranges `written` (each with the resident whose merge writes it,
/// or none: written in place) go up again to every other resident of
/// `live` that holds some of them, on [`MARK`] bytes. The title wrote none
/// of it (what it writes goes up by whole pages, [`Resident::refresh`]):
/// only the bytes the other copy's shaders may have written differ, so
/// whole pages would send up again what the resident holds already (run
/// inplace4-s2: 196 MB a frame marked, of 202 MB that went up). Returns
/// the bytes, marked after writes in place and after merges of other
/// residents.
fn cross_mark(live: &[Arc<Resident>], written: &[(Option<&Resident>, u64, u64)]) -> [u64; 2] {
    let mut marked = [0; 2];
    for &(writer, from, to) in written {
        for other in live {
            if writer.is_some_and(|w| std::ptr::eq(w, &**other)) || !other.overlaps(from, to) {
                continue;
            }
            let a = (from.max(other.start) - other.start) & !(MARK - 1);
            let b = (to.min(other.start + other.len) - other.start).next_multiple_of(MARK).min(other.len);
            other.mark(a, b);
            marked[usize::from(writer.is_some())] += b - a;
        }
    }
    marked
}

/// Closes a list of ours; a failure ends the run (the residents' bytes
/// would be wrong from here on).
fn close(list: &ID3D12GraphicsCommandList, what: &str) -> ID3D12CommandList {
    // SAFETY: an open list of ours.
    match unsafe { list.Close() }.and_then(|()| list.cast()) {
        Ok(list) => list,
        Err(e) => crate::host::fail(format_args!("{what} could not be recorded ({e})")),
    }
}

/// The submission [`prepare`] was for is on the queue, and the host's
/// fence is signalled with `value` behind it: its lists are free again
/// once the fence has passed it.
pub(crate) fn sent(value: u64) {
    if !on() {
        return;
    }
    if let Some(work) = lock(&WORK).as_mut() {
        let out = std::mem::take(&mut work.out);
        work.spare.extend(out.into_iter().map(|s| Spare { after: value, ..s }));
    }
}

/// The line, with the frame-rate line, while residents are on.
pub(crate) fn line(frames: f64) {
    if !on() {
        return;
    }
    let s: Vec<f64> = STATS.iter().map(|s| s.swap(0, Ordering::Relaxed) as f64 / frames).collect();
    let alive: Vec<Arc<Resident>> = lock(&ALL).iter().filter_map(Weak::upgrade).collect();
    let bytes = alive.iter().map(|r| r.len).sum::<u64>();
    let registered: Vec<*const Resident> = lock(&RANGES).values().map(Weak::as_ptr).collect();
    let zombies: Vec<&Arc<Resident>> = alive.iter().filter(|r| !registered.contains(&Arc::as_ptr(r))).collect();
    let (counts, only_ours) = lock(&WORK).as_mut().map_or(([0; 4], 0), |w| {
        let now = w.pass.counts();
        let delta: [u32; 4] = std::array::from_fn(|i| now[i].wrapping_sub(w.read[i]));
        w.read = now;
        // Unregistered residents nothing holds but our own lists (and
        // `alive` here).
        let ours = |r: &Arc<Resident>| w.spare.iter().chain(&w.out).map(|l| l.kept.iter().filter(|k| Arc::ptr_eq(k, r)).count()).sum::<usize>();
        let only = zombies.iter().filter(|r| Arc::strong_count(r) - 1 == ours(r)).count();
        (delta, only)
    });
    let overlap = overlap_bytes(alive.iter().map(|r| (r.start, r.len)));
    let mb = |b: f64| b / f64::from(1u32 << 20);
    let per = |n: u32| f64::from(n) / frames;
    let vram = crate::host::host().and_then(|h| h.video_memory()).unwrap_or_default();
    let [whole_way, narrow_way] = partial();
    tracing::info!(
        "residents (VICTORY_INPLACE=4): {} alive, {} MB in the card's memory and {} MB of twins (budget {} MB); per frame: \
         {:.1} MB uploaded from arenas in {:.0} ranges, {:.1} MB compared in {:.0} merge passes, {:.2} MB merged into arenas \
         ({:.0} partial dwords), {:.1} MB marked to go up again after writes through other copies, {:.1} submissions with work for residents; \
         since the start {} hot windows in place handed residents and {} asks for a new resident turned down (not in an arena, or over a budget); \
         of the bytes marked per frame, {:.1} MB after writes in place and {:.1} MB after merges of other residents over the same memory; \
         no longer registered: {} residents ({} MB; {} held only by our own lists), {:.1} MB of the upload and {:.1} MB of the compare per frame theirs; \
         title memory held by two or more residents {} MB; since the start {} buffers retargeted ({} slots made again, {} copies by address numbered again), \
         {:.1} views per frame whose slot shows another buffer than their range's copy (of them, the copy in a resident {:.1}, in place {:.1}, a copy {:.1})",
        alive.len(),
        bytes >> 20,
        bytes >> 20,
        most() >> 20,
        mb(s[0]),
        s[1],
        mb(s[2]),
        s[3],
        mb((per(counts[0]) + per(counts[2])) * 4.0),
        per(counts[1]) + per(counts[3]),
        mb(s[4]),
        s[5],
        HOT[0].load(Ordering::Relaxed),
        HOT[1].load(Ordering::Relaxed),
        mb(s[6]),
        mb((s[4] - s[6]).max(0.0)),
        zombies.len(),
        zombies.iter().map(|r| r.len).sum::<u64>() >> 20,
        only_ours,
        mb(s[7]),
        mb(s[8]),
        overlap >> 20,
        generation(),
        RETARGETS[0].load(Ordering::Relaxed),
        RETARGETS[1].load(Ordering::Relaxed),
        {
            let k: [f64; 3] = std::array::from_fn(|i| RETARGETS[2 + i].load(Ordering::Relaxed) as f64 / frames);
            k.iter().sum::<f64>()
        },
        RETARGETS[2].swap(0, Ordering::Relaxed) as f64 / frames,
        RETARGETS[3].swap(0, Ordering::Relaxed) as f64 / frames,
        RETARGETS[4].swap(0, Ordering::Relaxed) as f64 / frames,
    );
    tracing::info!(
        "residents' merge by the shaders' narrowest store (VICTORY_RESIDENT_PARTIAL: whole-dword ranges {whole_way:?}, narrow {narrow_way:?}): per frame \
         {:.1} MB compared in ranges of whole-dword stores, {:.2} MB merged, {:.0} partial dwords; \
         {:.1} MB compared in ranges of narrower stores ({:.1} MB of them typed stores only), {:.2} MB merged, {:.0} partial dwords; \
         card memory used {} of {} MB, shared {} of {} MB",
        mb((s[2] - s[9]).max(0.0)),
        mb(per(counts[0]) * 4.0),
        per(counts[1]),
        mb(s[9]),
        mb(s[10]),
        mb(per(counts[2]) * 4.0),
        per(counts[3]),
        vram[1] >> 20,
        vram[0] >> 20,
        vram[3] >> 20,
        vram[2] >> 20,
    );
}

/// The bytes of title memory that two or more of the ranges (start,
/// length) hold.
fn overlap_bytes(ranges: impl Iterator<Item = (u64, u64)>) -> u64 {
    // A sweep over starts and ends: depth 2 or more is overlap; an end
    // sorts before a start at the same address (touching is not overlap).
    let mut edges: Vec<(u64, i32)> = ranges.flat_map(|(start, len)| [(start, 1), (start + len, -1)]).collect();
    edges.sort_by_key(|&(at, d)| (at, d));
    let (mut depth, mut since, mut total) = (0i32, 0u64, 0u64);
    for (at, d) in edges {
        if depth >= 2 {
            total += at - since;
        }
        depth += d;
        since = at;
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inplace::{From, K64, Rig, open, place};
    use windows_sys::Win32::System::Memory as mem;

    /// Overlap: bytes held by two or more ranges, once however deep.
    /// Where ranges of whole-dword and of narrower stores meet, the narrow
    /// class holds the bytes; typed stores are narrow, counted apart.
    #[test]
    fn narrow_stores_hold_the_bytes_they_share_with_whole_ones() {
        assert_eq!(minus(vec![(0, 100), (200, 300)], &[(50, 60), (90, 210), (280, 400)]), vec![(0, 50), (60, 90), (210, 280)]);
        assert_eq!(minus(vec![(0, 100)], &[]), vec![(0, 100)]);
        assert_eq!(minus(vec![(10, 20)], &[(0, 100)]), vec![]);
        let (whole, narrow, typed) = split(vec![(0, 0x100), (0x200, 0x300)], vec![(0xc0, 0x140)], vec![(0x100, 0x120), (0x280, 0x400)]);
        assert_eq!(narrow, vec![(0xc0, 0x140), (0x280, 0x400)]);
        assert_eq!(whole, vec![(0, 0xc0), (0x200, 0x280)]);
        assert_eq!(typed, 0x60, "0xc0..0x100 and 0x120..0x140: typed, not inside a byte store's range");
        let (whole, narrow, typed) = split(vec![(0, 0x40), (0x40, 0x80)], vec![], vec![]);
        assert_eq!((whole, narrow, typed), (vec![(0, 0x80)], vec![], 0));
    }

    #[test]
    fn overlap_counts_bytes_held_twice_once() {
        assert_eq!(overlap_bytes([(0, 10), (10, 10)].into_iter()), 0, "touching");
        assert_eq!(overlap_bytes([(0, 10), (5, 10)].into_iter()), 5);
        assert_eq!(overlap_bytes([(0, 100), (10, 10), (15, 10), (50, 5)].into_iter()), 20, "10..25 and 50..55");
        assert_eq!(overlap_bytes([(0, 10)].into_iter()), 0);
    }

    /// The byte mask of the bytes in which two dwords differ, as [`MERGE`]
    /// makes it.
    fn byte_mask(diff: u32) -> u32 {
        (0..4).map(|k| 0xffu32 << (8 * k)).filter(|m| diff & m != 0).fold(0, |a, m| a | m)
    }

    /// What the merge leaves in the title's dword `title` for `gpu` (what
    /// `R` holds) against `was` (`B`): the bytes the card changed are the
    /// card's, every other byte the title's.
    fn merged(title: u32, gpu: u32, was: u32) -> u32 {
        let mask = byte_mask(gpu ^ was);
        (title & !mask) | (gpu & mask)
    }

    #[test]
    fn a_dword_takes_only_the_bytes_the_card_changed() {
        assert_eq!(byte_mask(0), 0);
        assert_eq!(byte_mask(0x0000_0100), 0x0000_ff00);
        assert_eq!(byte_mask(0x8000_0001), 0xff00_00ff);
        assert_eq!(byte_mask(0x0101_0101), u32::MAX);
        // As the two atomics do it: And with `gpu | !mask`, Or with
        // `gpu & mask`.
        for (title, gpu, was) in [(0x1122_3344u32, 0x11aa_3344u32, 0x1122_3344u32), (0xdead_beef, 0x0000_0000, 0xffff_ff00), (7, 7, 7)] {
            let mask = byte_mask(gpu ^ was);
            assert_eq!((title & (gpu | !mask)) | (gpu & mask), merged(title, gpu, was));
        }
        assert_eq!(merged(0x1122_33ff, 0x11aa_3344, 0x1122_3344), 0x11aa_33ff, "the title's byte 0 is kept");
    }

    /// Pages noted as runs; ranges joined where they touch.
    #[test]
    fn pages_go_up_in_runs() {
        assert_eq!(page_runs(&[]), []);
        assert_eq!(page_runs(&[2, 3, 4, 7, 9, 10]), [(2, 5), (7, 8), (9, 11)]);
        assert_eq!(joined(vec![(8, 12), (0, 4), (4, 6), (10, 20)]), [(0, 6), (8, 20)]);
    }

    /// Title memory of `pages` 64 KiB pages, write-watched as the
    /// runtime's is, opened as a part on `device`: the memory and where it
    /// is in its arena. It stays reserved to the end of the run: the
    /// write-watch journal (`crate::watch`) remembers its pages, and a
    /// later test given the same addresses would find them written.
    fn title(device: &ID3D12Device, pages: usize) -> Option<(*mut u8, crate::arena::Resolved)> {
        device.cast::<ID3D12Device13>().ok()?;
        let bytes = pages * K64;
        // SAFETY: an allocation of the test's own.
        let base = unsafe {
            mem::VirtualAlloc(std::ptr::null(), bytes, mem::MEM_RESERVE | mem::MEM_COMMIT | crate::inplace::MEM_WRITE_WATCH, mem::PAGE_READWRITE)
        }
        .cast::<u8>();
        assert!(!base.is_null());
        crate::inplace::fill(base, bytes, 0x5a);
        let heap = open(device, From::Part(base as usize, bytes)).ok()?;
        let resource = place(device, &heap, 0, bytes as u64).ok()?;
        // SAFETY: a live buffer.
        let gpu = unsafe { resource.GetGPUVirtualAddress() };
        let at = crate::arena::Resolved { resource, heap: heap.into(), offset: 0, gpu, left: bytes as u64, kind: crate::arena::Kind::InPlace, section: None };
        Some((base, at))
    }

    /// A resident over all of `at`'s title memory, `R` and `B` in the
    /// device's own memory.
    fn resident(device: &ID3D12Device, base: *mut u8, at: crate::arena::Resolved) -> Resident {
        let len = at.left;
        let r = committed(device, D3D12_HEAP_TYPE_DEFAULT, len).unwrap();
        let b = committed(device, D3D12_HEAP_TYPE_DEFAULT, len).unwrap();
        BYTES.fetch_add(2 * len, Ordering::Relaxed);
        Resident::new(base as u64, len, r, b, at)
    }

    /// Test (a): the pages the title wrote, and only those, go up from the
    /// arena into `R`, and into `B` from `R`; nothing is copied by the CPU.
    #[test]
    fn warp_the_pages_the_title_wrote_go_up_from_the_arena() {
        let Some(device) = crate::inplace::tests::warp() else { return };
        let Some((base, at)) = title(&device, 8) else { return };
        let r = resident(&device, base, at);
        let (start, n) = (base as u64, 8 * K64 as u64);
        let k = K64 as u64;
        assert_eq!(r.take(), [(0, n)], "everything goes up at first");
        // Never taken: the first look takes every page it covers.
        assert!(r.refresh(start + 100, start + 3 * k));
        assert_eq!(r.take(), [(0, 3 * k)], "whole pages");
        assert!(r.refresh(start, start + n));
        assert_eq!(r.take(), [(3 * k, n)], "what the first look did not cover");
        assert!(r.refresh(start, start + n));
        assert_eq!(r.take(), [], "nothing written since");
        let mut rig = Rig::new(&device).unwrap();
        // SAFETY: an open list of the rig's; the resident lives through it.
        rig.run(|list| unsafe {
            record_upload(list, &[Upload { resident: &r, ranges: vec![(0, n)] }]);
        })
        .unwrap();
        // SAFETY (each): inside the allocation.
        let bytes = |at: u64, len: usize| unsafe { std::slice::from_raw_parts(base.add(at as usize), len).to_vec() };
        assert_eq!(rig.fetch(&r.buffer, 0, n as usize).unwrap(), bytes(0, n as usize));
        assert_eq!(rig.fetch(&r.twin, 0, n as usize).unwrap(), bytes(0, n as usize));
        // The title writes pages 2, 5 and 6.
        for (page, value) in [(2u64, 1u8), (5, 2), (6, 3)] {
            // SAFETY: inside the allocation.
            unsafe { base.add((page * k + 77) as usize).write_volatile(value) };
        }
        assert!(r.refresh(start, start + n));
        let ranges = r.take();
        assert_eq!(ranges, [(2 * k, 3 * k), (5 * k, 7 * k)]);
        // SAFETY: as above.
        let up = rig.run(|list| unsafe {
            record_upload(list, &[Upload { resident: &r, ranges: ranges.clone() }]);
        });
        up.unwrap();
        assert_eq!(rig.fetch(&r.buffer, 0, n as usize).unwrap(), bytes(0, n as usize));
        assert_eq!(rig.fetch(&r.twin, 0, n as usize).unwrap(), bytes(0, n as usize));
        drop(r);
    }

    /// The merge of a resident's declared range: `gpu` is what the
    /// submission's shaders left in `R`; `cpu` writes byte 0 of dwords
    /// 128..160 of the title's memory all along, and byte 0 of dword 400
    /// once before the merge. The pass runs on `device`, storing partial
    /// dwords `way` (in the counters of narrow ranges unless whole).
    fn merge_with_the_cpu_writing(device: &ID3D12Device, way: Partial) {
        let Some((base, at)) = title(device, 1) else { return };
        let r = resident(device, base, at);
        let n = K64 as u64;
        let mut rig = Rig::new(device).unwrap();
        let pass = Pass::made(device, [way; 2]).unwrap();
        let narrow = way != Partial::Whole;
        // SAFETY: an open list of the rig's; the resident lives through it.
        rig.run(|list| unsafe {
            record_upload(list, &[Upload { resident: &r, ranges: r.take() }]);
        })
        .unwrap();
        // The title's dwords, read as the CPU sees them (the card and a
        // thread of the test write them too).
        // SAFETY (each): inside the allocation.
        let word = |w: usize| unsafe { base.cast::<u32>().add(w).read_volatile() };
        let words = || (0..K64 / 4).map(word).collect::<Vec<u32>>();
        let before = words();
        // What the shaders wrote: dwords 0..16 whole, 64..96 and 128..160
        // in byte 2 only.
        let mut gpu = before.clone();
        for w in 0..16 {
            gpu[w] = !before[w];
        }
        for w in (64..96).chain(128..160).chain([400]) {
            gpu[w] = before[w] ^ 0x00ab_0000;
        }
        let as_bytes: Vec<u8> = gpu.iter().flat_map(|w| w.to_le_bytes()).collect();
        rig.store(&r.buffer, 0, &as_bytes).unwrap();
        // Meanwhile the title writes byte 1 of dword 300, which the card
        // did not touch.
        // SAFETY: inside the allocation.
        unsafe { base.add(300 * 4 + 1).write_volatile(0x77) };
        // And byte 0 of dword 400, which the card changed in byte 2.
        let cpu_400 = (before[400] as u8) ^ 0xff;
        // SAFETY: inside the allocation.
        unsafe { base.add(400 * 4).write_volatile(cpu_400) };
        let stop = std::sync::atomic::AtomicBool::new(false);
        let at = base as usize;
        let last = std::thread::scope(|s| {
            let cpu = s.spawn(|| {
                let mut k = 0u8;
                while !stop.load(Ordering::Relaxed) {
                    k = k.wrapping_add(1).max(1);
                    for w in 128..160 {
                        // SAFETY: inside the allocation.
                        unsafe { std::ptr::write_volatile((at as *mut u8).add(w * 4), k) };
                    }
                }
                k
            });
            // SAFETY: an open list of the rig's, after the "shaders"; the
            // resident lives through it.
            let ran = rig.run(|list| unsafe {
                record_merge(list, &pass, &[Merge { resident: &r, ranges: vec![(0, n)], narrow }]);
            });
            stop.store(true, Ordering::Relaxed);
            ran.unwrap();
            cpu.join().unwrap()
        });
        let words = words();
        for w in 0..16 {
            assert_eq!(words[w], gpu[w], "a whole dword the card wrote, dword {w}");
        }
        for w in 64..96 {
            assert_eq!(words[w], gpu[w], "byte 2 the card's, the rest the title's, dword {w}");
        }
        for w in 128..160 {
            // Stored whole (or after a read that raced the CPU), the
            // CPU's byte 0 may be lost to the card's dword: only the
            // atomics keep it for certain.
            if matches!(way, Partial::Atomics | Partial::Exchange) {
                assert_eq!(words[w] & 0xff, u32::from(last), "the CPU's last byte 0 is kept, dword {w} ({way:?})");
            }
            assert_eq!(words[w] & 0xffff_ff00, gpu[w] & 0xffff_ff00, "the card's byte 2, dword {w} ({way:?})");
        }
        if way == Partial::Whole {
            assert_eq!(words[400], gpu[400], "stored whole, the card's dword over the CPU's byte 0");
        } else {
            assert_eq!(words[400], (gpu[400] & 0xffff_ff00) | u32::from(cpu_400), "the CPU's byte 0 and the card's byte 2 ({way:?})");
        }
        assert_eq!(words[300] & 0xff00, 0x7700, "a byte only the title wrote is the title's");
        let untouched = |w: usize| !(w < 16 || (64..96).contains(&w) || (128..160).contains(&w) || w == 300 || w == 400);
        assert!((0..K64 / 4).filter(|&w| untouched(w)).all(|w| words[w] == before[w]));
        let twin = rig.fetch(&r.twin, 0, K64).unwrap();
        assert_eq!(twin, as_bytes, "B takes what R holds");
        let counted = [16 + 32 + 32 + 1, 65];
        let expected = if narrow { [0, 0, counted[0], counted[1]] } else { [counted[0], counted[1], 0, 0] };
        assert_eq!(pass.counts(), expected, "dwords that differed, of them in part, by class ({way:?})");
        drop(r);
    }

    /// Test (b) on the software adapter, each way of storing partial dwords.
    #[test]
    fn warp_the_merge_stores_only_what_the_card_changed() {
        let Some(device) = crate::inplace::tests::warp() else { return };
        for way in [Partial::Atomics, Partial::Whole, Partial::Exchange, Partial::Load] {
            merge_with_the_cpu_writing(&device, way);
        }
    }

    /// One pass of the default ways: ranges of whole-dword stores go back
    /// whole, narrow ones with atomics, each in its own counters.
    #[test]
    fn warp_a_pass_merges_each_class_its_own_way() {
        let Some(device) = crate::inplace::tests::warp() else { return };
        let Some((base, at)) = title(&device, 1) else { return };
        let r = resident(&device, base, at);
        let mut rig = Rig::new(&device).unwrap();
        let pass = Pass::made(&device, [Partial::Whole, Partial::Atomics]).unwrap();
        // SAFETY: an open list of the rig's; the resident lives through it.
        rig.run(|list| unsafe {
            record_upload(list, &[Upload { resident: &r, ranges: r.take() }]);
        })
        .unwrap();
        // SAFETY (each): inside the allocation.
        let word = |w: usize| unsafe { base.cast::<u32>().add(w).read_volatile() };
        let before: Vec<u32> = (0..K64 / 4).map(word).collect();
        // The card changes byte 2 of dwords 10 (whole range) and 10000
        // (narrow range); the CPU then changes byte 0 of both.
        let mut gpu = before.clone();
        for w in [10, 10000] {
            gpu[w] ^= 0x00ab_0000;
        }
        let as_bytes: Vec<u8> = gpu.iter().flat_map(|w| w.to_le_bytes()).collect();
        rig.store(&r.buffer, 0, &as_bytes).unwrap();
        for w in [10, 10000] {
            // SAFETY: inside the allocation.
            unsafe { base.add(w * 4).write_volatile((before[w] as u8) ^ 0xff) };
        }
        let half = K64 as u64 / 2;
        // SAFETY: an open list of the rig's; the resident lives through it.
        rig.run(|list| unsafe {
            record_merge(list, &pass, &[Merge { resident: &r, ranges: vec![(0, half)], narrow: false }, Merge { resident: &r, ranges: vec![(half, K64 as u64)], narrow: true }]);
        })
        .unwrap();
        assert_eq!(word(10), gpu[10], "a whole-dword range: the card's dword whole");
        assert_eq!(word(10000), (gpu[10000] & 0xffff_ff00) | u32::from((before[10000] as u8) ^ 0xff), "a narrow range: the CPU's byte kept");
        assert_eq!(pass.counts(), [1, 1, 1, 1]);
        drop(r);
    }

    /// Test (b) on this PC's card (`cargo test -p victory-gpu -- --ignored
    /// merge_on_hardware`): the atomics into the arena go over the bus.
    #[test]
    #[ignore = "opens a device on the card"]
    fn merge_on_hardware() {
        use windows::Win32::Graphics::Direct3D::D3D_FEATURE_LEVEL_12_0;
        let mut device: Option<ID3D12Device> = None;
        // SAFETY: plain device creation on the default adapter.
        unsafe { D3D12CreateDevice(None, D3D_FEATURE_LEVEL_12_0, &mut device).unwrap() };
        let device = device.unwrap();
        for way in [Partial::Atomics, Partial::Whole, Partial::Exchange, Partial::Load] {
            merge_with_the_cpu_writing(&device, way);
        }
    }

    /// The merge on this PC's card (`cargo test -p victory-gpu --release --
    /// --ignored merge_on_hardware_runs_at_bus_speed`): 16 MB of `R`
    /// changed in every dword, whole and in one byte, is merged into the
    /// title's memory in well under 20 ms each (0.7 and 1.6 ms on the RTX
    /// 4090; 167 and 327 ms with a thread per 16 bytes, [`MERGE`]).
    #[test]
    #[ignore = "opens a device on the card"]
    fn merge_on_hardware_runs_at_bus_speed() {
        use windows::Win32::Graphics::Direct3D::D3D_FEATURE_LEVEL_12_0;
        let mut device: Option<ID3D12Device> = None;
        // SAFETY: plain device creation on the default adapter.
        unsafe { D3D12CreateDevice(None, D3D_FEATURE_LEVEL_12_0, &mut device).unwrap() };
        let device = device.unwrap();
        let pages = 256;
        let (base, at) = title(&device, pages).unwrap();
        let r = resident(&device, base, at);
        let (n, words) = ((pages * K64) as u64, (pages * K64 / 4) as u32);
        let mut rig = Rig::new(&device).unwrap();
        let pass = Pass::made(&device, [Partial::Atomics; 2]).unwrap();
        for (flip, part) in [(u32::MAX, 0), (0x00ab_0000, words)] {
            // SAFETY (each run): an open list of the rig's; the resident
            // lives through it.
            rig.run(|list| unsafe {
                record_upload(list, &[Upload { resident: &r, ranges: vec![(0, n)] }]);
            })
            .unwrap();
            let twin = rig.fetch(&r.twin, 0, n as usize).unwrap();
            let gpu: Vec<u8> = twin.chunks(4).flat_map(|t| (u32::from_le_bytes(t.try_into().unwrap()) ^ flip).to_le_bytes()).collect();
            rig.store(&r.buffer, 0, &gpu).unwrap();
            let before = pass.counts();
            let started = std::time::Instant::now();
            rig.run(|list| unsafe {
                record_merge(list, &pass, &[Merge { resident: &r, ranges: vec![(0, n)], narrow: false }]);
            })
            .unwrap();
            let took = started.elapsed();
            let after = pass.counts();
            assert_eq!([after[0].wrapping_sub(before[0]), after[1].wrapping_sub(before[1])], [words, part]);
            // SAFETY: inside the allocation.
            assert!(unsafe { std::slice::from_raw_parts(base, n as usize) } == gpu.as_slice(), "the title's memory holds R");
            assert!(took < std::time::Duration::from_millis(20), "16 MB merged in {took:?}");
        }
        drop(r);
    }

    /// The frame's shape on this PC's card, without the game (Stage 2
    /// latency plan, A4; `cargo test -p victory-gpu --release --
    /// --ignored frame_shape_on_hardware --nocapture`): 1 GB of title
    /// memory opened as a part, 16 residents of 64 MB over it; per frame
    /// 25 submissions, each sending up 8 ranges of 1 MB from the arena, a
    /// stand-in for the title's shaders (a copy in the card's memory that
    /// changes about 6% of the dwords of 23 MB of `R`, 30% of those in part)
    /// and the merge of those 23 MB, while a CPU thread writes the title's
    /// pages. Prints the time per frame of each part (each run waited for
    /// on its own): the merge with atomics and with plain stores, the
    /// upload from the arena and from an upload heap. What the game's frame
    /// would cost the residents if nothing overlapped.
    #[test]
    #[ignore = "opens a device on the card; about 2 GB of memory"]
    fn frame_shape_on_hardware() {
        use windows::Win32::Graphics::Direct3D::D3D_FEATURE_LEVEL_12_0;
        const MB: u64 = 1 << 20;
        const EACH: u64 = 64 * MB;
        const RESIDENTS: u64 = 16;
        const SUBMISSIONS: u64 = 25;
        const FRAMES: u64 = 4;
        let mut device: Option<ID3D12Device> = None;
        // SAFETY: plain device creation on the default adapter.
        unsafe { D3D12CreateDevice(None, D3D_FEATURE_LEVEL_12_0, &mut device).unwrap() };
        let device = device.unwrap();
        let pages = (RESIDENTS * EACH / K64 as u64) as usize;
        let (base, at) = title(&device, pages).unwrap();
        let residents: Vec<Resident> = (0..RESIDENTS)
            .map(|k| {
                let offset = k * EACH;
                let part = crate::arena::Resolved {
                    resource: at.resource.clone(),
                    heap: at.heap.clone(),
                    offset,
                    gpu: at.gpu + offset,
                    left: EACH,
                    kind: crate::arena::Kind::InPlace,
                    section: None,
                };
                // SAFETY: inside the allocation.
                resident(&device, unsafe { base.add(offset as usize) }, part)
            })
            .collect();
        // What the "shaders" leave in `R`: two patterns of the arena's bytes
        // (every resident's are the same: `fill` repeats every 256 bytes),
        // each changing a different ~6% of the dwords, 30% of those in one
        // byte only.
        let words = (EACH / 4) as usize;
        // SAFETY: inside the allocation.
        let arena: Vec<u32> = unsafe { std::slice::from_raw_parts(base.cast::<u32>(), words).to_vec() };
        let pattern = |salt: u32| -> Vec<u8> {
            arena
                .iter()
                .enumerate()
                .map(|(i, &w)| {
                    let h = (i as u32 ^ salt).wrapping_mul(0x9e37_79b9) >> 16;
                    match h % 1000 {
                        0..=41 => !w,
                        42..=59 => w ^ 0x0000_ff00,
                        _ => w,
                    }
                })
                .flat_map(u32::to_le_bytes)
                .collect()
        };
        let mut rig = Rig::new(&device).unwrap();
        let vram = |bytes: u64| committed(&device, D3D12_HEAP_TYPE_DEFAULT, bytes).unwrap();
        let patterns = [vram(EACH), vram(EACH)];
        for (k, p) in patterns.iter().enumerate() {
            let bytes = pattern(k as u32 * 0x5bd1_e995 + 1);
            for (i, chunk) in bytes.chunks(16 << 20).enumerate() {
                rig.store(p, (i << 24) as u64, chunk).unwrap();
            }
        }
        // An upload heap of 8 MB, for the upload's comparison.
        let up = {
            let props = D3D12_HEAP_PROPERTIES { Type: D3D12_HEAP_TYPE_UPLOAD, ..Default::default() };
            let mut out: Option<ID3D12Resource> = None;
            // SAFETY: plain creation.
            unsafe {
                device
                    .CreateCommittedResource(&props, D3D12_HEAP_FLAG_NONE, &crate::inplace::buffer_desc(8 * MB, D3D12_RESOURCE_FLAG_NONE), D3D12_RESOURCE_STATE_GENERIC_READ, None, &mut out)
                    .unwrap()
            };
            out.unwrap()
        };
        // Everything up once, as at a resident's birth.
        for r in &residents {
            // SAFETY: an open list of the rig's; the resident lives through it.
            rig.run(|list| unsafe {
                record_upload(list, &[Upload { resident: r, ranges: r.take() }]);
            })
            .unwrap();
        }
        let stop = std::sync::atomic::AtomicBool::new(false);
        let (title_at, title_len) = (base as usize, (RESIDENTS * EACH) as usize);
        let compared = 23 * MB;
        let timed = |rig: &mut Rig, record: &dyn Fn(&ID3D12GraphicsCommandList)| {
            let started = std::time::Instant::now();
            rig.run(record).unwrap();
            started.elapsed().as_secs_f64() * 1000.0
        };
        std::thread::scope(|scope| {
            // The title's CPU, writing a byte in a page here and there.
            scope.spawn(|| {
                let mut k = 0usize;
                while !stop.load(Ordering::Relaxed) {
                    k = (k + 7919 * 4096 + 3) % title_len;
                    // SAFETY: inside the allocation.
                    unsafe { std::ptr::write_volatile((title_at as *mut u8).add(k), k as u8) };
                    std::hint::spin_loop();
                }
            });
            for way in [Partial::Atomics, Partial::Whole, Partial::Exchange, Partial::Load] {
                let pass = Pass::made(&device, [way; 2]).unwrap();
                let before = pass.counts();
                let (mut upload, mut upload_heap, mut shaders, mut merge) = (0.0, 0.0, 0.0, 0.0);
                for frame in 0..FRAMES {
                    for n in 0..SUBMISSIONS {
                        let sub = frame * SUBMISSIONS + n;
                        let target = &residents[(sub % RESIDENTS) as usize];
                        let fed = &residents[((sub + 3) % RESIDENTS) as usize];
                        let ranges: Vec<(u64, u64)> = (0..8).map(|i| ((i * 7 + sub) % 60 * MB, ((i * 7 + sub) % 60 + 1) * MB)).collect();
                        // SAFETY (each run): open lists of the rig's; the
                        // residents and buffers live through them; every
                        // range lies inside them.
                        upload += timed(&mut rig, &|list| unsafe {
                            record_upload(list, &[Upload { resident: fed, ranges: ranges.clone() }]);
                        });
                        upload_heap += timed(&mut rig, &|list| unsafe {
                            for (i, &(a, b)) in ranges.iter().enumerate() {
                                list.CopyBufferRegion(&fed.twin, a, &up, i as u64 % 8 * MB, b - a);
                            }
                        });
                        let from = (sub * 7 * MB) % (EACH - compared);
                        let p = &patterns[(sub % 2) as usize];
                        shaders += timed(&mut rig, &|list| unsafe {
                            list.CopyBufferRegion(&target.buffer, from, p, from, compared);
                        });
                        merge += timed(&mut rig, &|list| unsafe {
                            record_merge(list, &pass, &[Merge { resident: target, ranges: vec![(from, from + compared)], narrow: false }]);
                        });
                    }
                }
                let f = FRAMES as f64;
                let counts = pass.counts();
                let (changed, partial) = (counts[0].wrapping_sub(before[0]), counts[1].wrapping_sub(before[1]));
                eprintln!(
                    "frame shape (dwords changed in part: {way:?}): per frame of {SUBMISSIONS} submissions: upload of 8 MB each from the arena {:.1} ms, the same from an upload heap {:.1} ms, \
                     the stand-in shaders {:.1} ms, the merge of {} MB compared {:.1} ms; {:.1} MB merged per frame, {:.0} partial dwords per frame",
                    upload / f,
                    upload_heap / f,
                    shaders / f,
                    SUBMISSIONS * compared / MB,
                    merge / f,
                    f64::from(changed) * 4.0 / f / MB as f64,
                    f64::from(partial) / f,
                );
            }
            stop.store(true, Ordering::Relaxed);
        });
        drop(residents);
    }

    /// Test (c): submission 1's shaders write a dword of `R` and its merge
    /// puts it into the title's memory; the title then writes another byte
    /// of the same 64 KiB page; submission 2's upload of that page brings
    /// both into `R` and `B`, and its merge finds nothing to do.
    #[test]
    fn warp_a_merge_lands_before_the_next_upload_of_its_page() {
        let Some(device) = crate::inplace::tests::warp() else { return };
        let Some((base, at)) = title(&device, 2) else { return };
        let r = resident(&device, base, at);
        let (start, n) = (base as u64, 2 * K64 as u64);
        let mut rig = Rig::new(&device).unwrap();
        let pass = Pass::new(&device).unwrap();
        // SAFETY (each run): an open list of the rig's; the resident lives
        // through it.
        rig.run(|list| unsafe {
            record_upload(list, &[Upload { resident: &r, ranges: r.take() }]);
        })
        .unwrap();
        assert!(r.refresh(start, start + n));
        let _ = r.take();
        // Submission 1: a shader writes dword 0x40 of page 1; the merge.
        let x = K64 as u64 + 0x100;
        rig.store(&r.buffer, x, &0x1234_5678u32.to_le_bytes()).unwrap();
        rig.run(|list| unsafe {
            record_merge(list, &pass, &[Merge { resident: &r, ranges: vec![(K64 as u64, n)], narrow: true }]);
        })
        .unwrap();
        // SAFETY (each): inside the allocation.
        let word = |at: u64| unsafe { base.add(at as usize).cast::<u32>().read_volatile() };
        assert_eq!(word(x), 0x1234_5678, "merged into the title's memory");
        // The title writes a byte of the same page.
        let y = K64 as u64 + 0x2001;
        // SAFETY: inside the allocation.
        unsafe { base.add(y as usize).write_volatile(0xee) };
        // Submission 2: the page the title wrote goes up, whole.
        assert!(r.refresh(start, start + n));
        let ranges = r.take();
        assert_eq!(ranges, [(K64 as u64, n)]);
        rig.run(|list| unsafe {
            record_upload(list, &[Upload { resident: &r, ranges }]);
        })
        .unwrap();
        for buffer in [&r.buffer, &r.twin] {
            let got = rig.fetch(buffer, K64 as u64, K64).unwrap();
            assert_eq!(&got[0x100..0x104], &0x1234_5678u32.to_le_bytes(), "the card's dword");
            assert_eq!(got[0x2001], 0xee, "the title's byte");
        }
        let counted = pass.counts();
        rig.run(|list| unsafe {
            record_merge(list, &pass, &[Merge { resident: &r, ranges: vec![(0, n)], narrow: true }]);
        })
        .unwrap();
        assert_eq!(pass.counts(), counted, "R and B agree: nothing merged");
        assert_eq!(word(x), 0x1234_5678);
        drop(r);
    }

    /// Test (d): a buffer whose memory is not in an arena gets no resident
    /// (its copy in the card's memory stays, as before), and takes nothing
    /// of the budget; one in an arena gets one, and a buffer inside its
    /// pages shares it at its offset. What is written through one copy of
    /// the same memory goes up to the others, not to the writer.
    #[test]
    fn warp_a_buffer_outside_arenas_keeps_its_copy_and_nested_ones_share() {
        let Some(device) = crate::inplace::tests::warp() else { return };
        let Some((base, at)) = title(&device, 4) else { return };
        let k = K64 as u64;
        let (start, n) = (base as u64, 4 * k);
        let mut arenas = crate::arena::Arenas::default();
        let mut buffer = |bytes| committed(&device, D3D12_HEAP_TYPE_DEFAULT, bytes).ok();
        // Not reported committed: no arena, no resident.
        let none = make(start, n, u64::MAX, &mut |at, len| arenas.resolve_with(&device, at, len, u64::MAX, u64::MAX, &mut |_| ()), &mut buffer);
        assert!(none.is_none());
        assert!(holder(start, start + k).is_none(), "nothing registered");
        // In an arena: a resident, registered.
        arenas.committed(start, n);
        let outer = make(start, n, u64::MAX, &mut |at, len| arenas.resolve_with(&device, at, len, u64::MAX, u64::MAX, &mut |_| ()), &mut buffer).expect("a resident");
        assert_eq!((outer.start, outer.len), (start, n));
        let inner = holder(start + k, start + 3 * k).expect("in its pages");
        assert!(Arc::ptr_eq(&inner, &outer));
        assert!(holder(start, start + n + k).is_none(), "past its pages");
        // A second resident over some of the same pages (one made before
        // a bigger one): writes merged from the outer one go up to it.
        let other = Arc::new(resident(&device, base, at));
        let _ = (outer.take(), other.take());
        let live = [outer.clone(), other.clone()];
        let marked = cross_mark(&live, &[(Some(&*outer), start + k + 8, start + k + 16)]);
        assert_eq!(marked, [0, MARK]);
        assert_eq!(other.take(), [(k, k + MARK)], "the bytes written go up to the other copy, not their page");
        assert_eq!(outer.take(), [], "not to the one that wrote it");
        // Written in place (no resident): up to both; a range across the
        // unit's line takes both units.
        assert_eq!(cross_mark(&live, &[(None, start + 3 * k + MARK - 4, start + 3 * k + MARK + 4)]), [4 * MARK, 0]);
        assert_eq!(outer.take(), [(3 * k, 3 * k + 2 * MARK)]);
        // At the resident's end: not past it.
        assert_eq!(cross_mark(&live, &[(None, start + n - 8, start + n + 8)]), [2 * MARK, 0]);
        assert_eq!(other.take(), [(3 * k, 3 * k + 2 * MARK), (n - MARK, n)]);
        forget(start, start + 1, false);
        assert!(holder(start, start + k).is_none(), "forgotten when let go");
        drop((outer, inner, other, live));
        drop(arenas);
    }

    /// No two registered residents hold the same title memory (Stage 2
    /// latency plan, B1): a union that cannot be made makes nothing (it
    /// made one beside the old, over the same pages); one that is made
    /// takes the old one's place, and the old one's `R` is retargeted;
    /// forgetting retargets too.
    #[test]
    fn warp_no_two_residents_hold_the_same_memory() {
        let Some(device) = crate::inplace::tests::warp() else { return };
        let Some((base, _at)) = title(&device, 4) else { return };
        let (k, start) = (K64 as u64, base as u64);
        let mut arenas = crate::arena::Arenas::default();
        arenas.committed(start, 4 * k);
        let mut buffer = |bytes| committed(&device, D3D12_HEAP_TYPE_DEFAULT, bytes).ok();
        let mut resolve = |at, len| arenas.resolve_with(&device, at, len, u64::MAX, u64::MAX, &mut |_| ());
        let (inner, replaced) = make_over(start + k, start + 2 * k, u64::MAX, &mut resolve, &mut buffer).expect("a resident");
        assert!(replaced.is_empty() && registered(&inner) && !was_replaced(&inner.buffer));
        // The union over 3 pages does not fit (a budget of 4 pages of R
        // and B, 2 of them taken): nothing made, the old one stays.
        let before = generation();
        assert!(make_over(start, start + 3 * k, 4 * k, &mut resolve, &mut buffer).is_none(), "no resident beside the old one");
        assert!(disjoint());
        assert!(Arc::ptr_eq(&holder(start + k, start + 2 * k).unwrap(), &inner));
        assert_eq!(generation(), before, "nothing retargeted");
        // It fits: one over both, the old one retargeted.
        let (outer, replaced) = make_over(start, start + 3 * k, u64::MAX, &mut resolve, &mut buffer).expect("a union");
        assert!(matches!(replaced.as_slice(), [r] if Arc::ptr_eq(r, &inner)));
        assert!(disjoint());
        assert!(registered(&outer) && !registered(&inner));
        assert!(was_replaced(&inner.buffer) && !was_replaced(&outer.buffer));
        assert!(generation() > before);
        // Forgotten: retargeted too.
        assert!(forget(start, start + 1, false));
        assert!(!registered(&outer) && was_replaced(&outer.buffer));
        drop((inner, outer, replaced));
        drop(arenas);
    }

    /// A union made through one table lets go of the windows of the other
    /// that were in the old resident (they kept it alive, taking uploads
    /// and merges); a window of the new one stays.
    #[test]
    fn warp_a_union_lets_go_of_windows_in_the_old_resident() {
        let Some(device) = crate::inplace::tests::warp() else { return };
        let Some((base, _at)) = title(&device, 4) else { return };
        let (k, start) = (K64 as u64, base as u64);
        let mut arenas = crate::arena::Arenas::default();
        arenas.committed(start, 4 * k);
        let mut buffer = |bytes| committed(&device, D3D12_HEAP_TYPE_DEFAULT, bytes).ok();
        let mut resolve = |at, len| arenas.resolve_with(&device, at, len, u64::MAX, u64::MAX, &mut |_| ());
        let (inner, _) = make_over(start + k, start + 2 * k, u64::MAX, &mut resolve, &mut buffer).expect("a resident");
        let old = crate::shadow::Shadow::resident_window(inner.clone(), start + k, k);
        crate::shadow::retarget_tables();
        assert!(crate::shadow::Shadow::in_windows(&old), "registered: stays");
        let (outer, _) = make_over(start, start + 3 * k, u64::MAX, &mut resolve, &mut buffer).expect("a union");
        let new = crate::shadow::Shadow::resident_window(outer.clone(), start + 2 * k, k);
        crate::shadow::retarget_tables();
        assert!(!crate::shadow::Shadow::in_windows(&old), "in the replaced resident: let go");
        assert!(crate::shadow::Shadow::in_windows(&new), "in the registered one: stays");
        forget(start, start + 4 * k, false);
        crate::shadow::retarget_tables();
        assert!(!crate::shadow::Shadow::in_windows(&new), "forgotten: let go");
        drop((old, new, inner, outer));
        drop(arenas);
    }

    /// Residents that overlap become one: a big buffer whose pages hold a
    /// resident made before it (an inner buffer used first) and more gets
    /// one resident over both, which takes the older one's place; one over
    /// the gap between two residents joins all three. The replaced ones
    /// stay alive for what holds them, and in step with the new one.
    #[test]
    fn warp_overlapping_residents_become_one() {
        let Some(device) = crate::inplace::tests::warp() else { return };
        let Some((base, _at)) = title(&device, 8) else { return };
        let (k, start) = (K64 as u64, base as u64);
        let mut arenas = crate::arena::Arenas::default();
        arenas.committed(start, 8 * k);
        let mut resolve = |at, len| arenas.resolve_with(&device, at, len, u64::MAX, u64::MAX, &mut |_| ());
        let mut buffer = |bytes| committed(&device, D3D12_HEAP_TYPE_DEFAULT, bytes).ok();
        let mut over = |from: u64, to: u64| make_over(start + from * k, start + to * k, u64::MAX, &mut resolve, &mut buffer).expect("a resident");
        let (inner, replaced) = over(1, 2);
        assert!(replaced.is_empty());
        let (right, replaced) = over(5, 6);
        assert!(replaced.is_empty());
        let (outer, replaced) = over(0, 3);
        assert_eq!((outer.start, outer.len), (start, 3 * k));
        assert!(matches!(replaced.as_slice(), [r] if Arc::ptr_eq(r, &inner)));
        assert!(Arc::ptr_eq(&holder(start + k, start + 2 * k).unwrap(), &outer), "the older one's pages find the new one");
        let (all, replaced) = over(2, 6);
        assert_eq!((all.start, all.len), (start, 6 * k));
        assert_eq!(replaced.len(), 2);
        assert!(replaced.iter().any(|r| Arc::ptr_eq(r, &outer)) && replaced.iter().any(|r| Arc::ptr_eq(r, &right)));
        assert!(Arc::ptr_eq(&holder(start + 5 * k, start + 6 * k).unwrap(), &all));
        assert!(Arc::ptr_eq(&holder(start, start + k).unwrap(), &all));
        // Written through the new one: the page goes up to a replaced one.
        let live = [all.clone(), inner.clone()];
        let _ = (all.take(), inner.take());
        assert_eq!(cross_mark(&live, &[(Some(&*all), start + k + 4, start + k + 8)]), [0, MARK]);
        assert_eq!(inner.take(), [(0, MARK)]);
        forget(start, start + 8 * k, false);
        drop((inner, right, outer, all, live, replaced));
    }
}
