//! The card's own time for the parts of a submission, in release builds
//! (`VICTORY_CARD_TIME`, on by default while residents are on,
//! `crate::resident::on`): timestamps from tiny lists of ours put inside
//! the submission's `ExecuteCommandLists` calls, around the upload to
//! residents, the title's lists and the merge. Nothing of the title's lists
//! changes and nothing waits: a submission's stamps are read once the host
//! fence has passed it, when a later submission takes their block.
//!
//! "On the GPU" in the frame line is the submitting thread's wall clock
//! (with the relay it holds the relay's landing too); these are the card's.

use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use windows::Win32::Graphics::Direct3D12::*;
use windows::core::Interface;

/// A stamp's place in its submission's block.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Mark {
    UploadBegin = 0,
    UploadEnd,
    TitleBegin,
    TitleEnd,
    MergeBegin,
    MergeEnd,
}

const MARKS: usize = 6;
/// Submissions whose stamps can be in the air at once; one beyond that
/// goes without (counted).
const BLOCKS: usize = 128;

/// Whether stamps are taken: `VICTORY_CARD_TIME=1` / `0`, else while
/// residents are on.
pub(crate) fn on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| match std::env::var("VICTORY_CARD_TIME").ok().as_deref().map(str::trim) {
        Some("1") => true,
        Some("0") => false,
        _ => crate::resident::on(),
    })
}

struct Block {
    /// The host fence's value after which its stamps can be read (0: free).
    after: u64,
    /// The marks stamped, by bit.
    written: u8,
}

struct Small {
    allocator: ID3D12CommandAllocator,
    list: ID3D12GraphicsCommandList,
    after: u64,
}

struct Ring {
    heap: ID3D12QueryHeap,
    back: ID3D12Resource,
    back_at: *const u64,
    blocks: Vec<Block>,
    lists: Vec<Small>,
    /// Lists of the submission being prepared ([`sent`] gives them their
    /// value).
    out: Vec<Small>,
    /// The block of the submission being prepared.
    current: Option<usize>,
    /// The last stamp read of the submission before, for the time between.
    last_end: Option<u64>,
}

// SAFETY: D3D12 objects and a persistent mapping read under the lock.
unsafe impl Send for Ring {}

static RING: Mutex<Option<Ring>> = Mutex::new(None);

/// Since the last line, in ticks: upload, title lists, merge, the gaps
/// between our calls within a submission, the time from one submission's
/// last stamp to the next one's first (the card idle, other sends:
/// card sync, the relay's fetches); then counts: submissions read,
/// submissions without a free block.
static SUMS: [AtomicU64; 7] = [const { AtomicU64::new(0) }; 7];
static FREQUENCY: AtomicU64 = AtomicU64::new(0);

fn ring() -> std::sync::MutexGuard<'static, Option<Ring>> {
    RING.lock().unwrap_or_else(|e| e.into_inner())
}

fn make(device: &ID3D12Device, queue: &ID3D12CommandQueue) -> windows::core::Result<Ring> {
    // SAFETY: plain object creation; the readback buffer is mapped for its
    // life and only read.
    unsafe {
        let mut heap: Option<ID3D12QueryHeap> = None;
        device.CreateQueryHeap(&D3D12_QUERY_HEAP_DESC { Type: D3D12_QUERY_HEAP_TYPE_TIMESTAMP, Count: (BLOCKS * MARKS) as u32, NodeMask: 0 }, &mut heap)?;
        let heap = heap.ok_or_else(windows::core::Error::empty)?;
        let props = D3D12_HEAP_PROPERTIES { Type: D3D12_HEAP_TYPE_READBACK, ..Default::default() };
        let mut back: Option<ID3D12Resource> = None;
        device.CreateCommittedResource(
            &props,
            D3D12_HEAP_FLAG_NONE,
            &crate::inplace::buffer_desc((BLOCKS * MARKS * 8) as u64, D3D12_RESOURCE_FLAG_NONE),
            D3D12_RESOURCE_STATE_COPY_DEST,
            None,
            &mut back,
        )?;
        let back = back.ok_or_else(windows::core::Error::empty)?;
        let mut at = std::ptr::null_mut();
        back.Map(0, None, Some(&mut at))?;
        FREQUENCY.store(queue.GetTimestampFrequency()?, Ordering::Relaxed);
        Ok(Ring {
            heap,
            back,
            back_at: at.cast(),
            blocks: (0..BLOCKS).map(|_| Block { after: 0, written: 0 }).collect(),
            lists: Vec::new(),
            out: Vec::new(),
            current: None,
            last_end: None,
        })
    }
}

impl Ring {
    /// Adds what the blocks the card is done with hold, and frees them.
    fn harvest(&mut self, completed: u64) {
        // Oldest first, for the time between submissions.
        let mut done: Vec<usize> = (0..BLOCKS).filter(|&b| self.blocks[b].after != 0 && self.blocks[b].after <= completed).collect();
        done.sort_by_key(|&b| self.blocks[b].after);
        for b in done {
            let written = self.blocks[b].written;
            // SAFETY: inside the mapping; the card wrote these before the
            // fence value that has passed.
            let t = |m: Mark| (written & (1 << m as u8) != 0).then(|| unsafe { self.back_at.add(b * MARKS + m as usize).read_volatile() });
            let span = |a: Mark, z: Mark| t(a).zip(t(z)).map_or(0, |(a, z)| z.saturating_sub(a));
            SUMS[0].fetch_add(span(Mark::UploadBegin, Mark::UploadEnd), Ordering::Relaxed);
            SUMS[1].fetch_add(span(Mark::TitleBegin, Mark::TitleEnd), Ordering::Relaxed);
            SUMS[2].fetch_add(span(Mark::MergeBegin, Mark::MergeEnd), Ordering::Relaxed);
            SUMS[3].fetch_add(span(Mark::UploadEnd, Mark::TitleBegin) + span(Mark::TitleEnd, Mark::MergeBegin), Ordering::Relaxed);
            let first = [Mark::UploadBegin, Mark::TitleBegin, Mark::MergeBegin].into_iter().find_map(t);
            let last = [Mark::MergeEnd, Mark::TitleEnd, Mark::UploadEnd].into_iter().find_map(t);
            if let (Some(prev), Some(first)) = (self.last_end, first) {
                SUMS[4].fetch_add(first.saturating_sub(prev), Ordering::Relaxed);
            }
            if last.is_some() {
                self.last_end = last;
            }
            SUMS[5].fetch_add(1, Ordering::Relaxed);
            self.blocks[b] = Block { after: 0, written: 0 };
        }
    }

    /// A tiny list of ours, open, whose last run is done.
    fn small(&mut self, device: &ID3D12Device, completed: u64) -> Option<ID3D12GraphicsCommandList> {
        let small = match self.lists.iter().position(|s| s.after <= completed) {
            Some(at) => self.lists.swap_remove(at),
            // SAFETY: plain object creation.
            None => unsafe {
                let allocator: ID3D12CommandAllocator = device.CreateCommandAllocator(D3D12_COMMAND_LIST_TYPE_DIRECT).ok()?;
                let list: ID3D12GraphicsCommandList = device.CreateCommandList(0, D3D12_COMMAND_LIST_TYPE_DIRECT, &allocator, None).ok()?;
                list.Close().ok()?;
                Small { allocator, list, after: 0 }
            },
        };
        // SAFETY: its last run is done.
        unsafe {
            small.allocator.Reset().ok()?;
            small.list.Reset(&small.allocator, None).ok()?;
        }
        let list = small.list.clone();
        self.out.push(small);
        Some(list)
    }
}

/// The caller checks [`on`] for this and the next two.
///
/// A submission begins: its block is taken (blocks the card is done with
/// are read first). `completed` is the host fence's value.
pub(crate) fn begin(device: &ID3D12Device, queue: &ID3D12CommandQueue, completed: u64) {
    let mut ring = ring();
    if ring.is_none() {
        match make(device, queue) {
            Ok(r) => *ring = Some(r),
            Err(e) => {
                tracing::warn!(error = %e, "no card time: its stamps could not be made (VICTORY_CARD_TIME)");
                return;
            }
        }
    }
    let Some(ring) = ring.as_mut() else { return };
    ring.harvest(completed);
    ring.current = (0..BLOCKS).find(|&b| ring.blocks[b].after == 0 && ring.blocks[b].written == 0);
    if ring.current.is_none() {
        SUMS[6].fetch_add(1, Ordering::Relaxed);
    }
}

/// A list of ours that stamps `mark` of the submission being prepared, to
/// go in its `ExecuteCommandLists` call before or after the lists it
/// times; with `resolve`, it is the submission's last and resolves its
/// stamps for reading.
pub(crate) fn stamp(device: &ID3D12Device, completed: u64, mark: Mark, resolve: bool) -> Option<ID3D12CommandList> {
    let mut ring = ring();
    let ring = ring.as_mut()?;
    let block = ring.current?;
    let list = ring.small(device, completed)?;
    let index = (block * MARKS + mark as usize) as u32;
    ring.blocks[block].written |= 1 << mark as u8;
    // SAFETY: an open list of ours; the query and buffer ranges are the
    // block's, which no other submission in the air uses.
    unsafe {
        list.EndQuery(&ring.heap, D3D12_QUERY_TYPE_TIMESTAMP, index);
        if resolve {
            let written = ring.blocks[block].written;
            for m in 0..MARKS {
                if written & (1 << m) != 0 {
                    let i = (block * MARKS + m) as u32;
                    list.ResolveQueryData(&ring.heap, D3D12_QUERY_TYPE_TIMESTAMP, i, 1, &ring.back, u64::from(i) * 8);
                }
            }
        }
        list.Close().ok()?;
    }
    list.cast().ok()
}

/// The submission is on the queue and the host fence is signalled with
/// `value` behind it.
pub(crate) fn sent(value: u64) {
    if let Some(ring) = ring().as_mut() {
        if let Some(block) = ring.current.take()
            && ring.blocks[block].written != 0
        {
            ring.blocks[block].after = value;
        }
        let out = std::mem::take(&mut ring.out);
        ring.lists.extend(out.into_iter().map(|s| Small { after: value, ..s }));
    }
}

/// The line, with the frame-rate line.
pub(crate) fn line(frames: f64) {
    if !on() {
        return;
    }
    let ticks = FREQUENCY.load(Ordering::Relaxed) as f64;
    if ticks == 0.0 {
        return;
    }
    let s: Vec<u64> = SUMS.iter().map(|s| s.swap(0, Ordering::Relaxed)).collect();
    let ms = |t: u64| t as f64 * 1000.0 / ticks / frames;
    tracing::info!(
        "card time per frame (VICTORY_CARD_TIME): upload to residents {:.1} ms, title lists {:.1} ms, merge {:.1} ms, between our calls in a submission {:.1} ms, \
         from one submission's end to the next one's start {:.1} ms (the card idle, card sync, the relay's fetches); {:.1} submissions read, {} without stamps",
        ms(s[0]),
        ms(s[1]),
        ms(s[2]),
        ms(s[3]),
        ms(s[4]),
        s[5] as f64 / frames,
        s[6],
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    /// On the software adapter: a submission's stamps around a list that
    /// copies 32 MB are read once a later submission begins after the
    /// fence passed it, and give the list's time and none for parts it
    /// lacks.
    #[test]
    fn warp_a_submissions_stamps_are_read_after_its_fence() {
        let Some(device) = crate::inplace::tests::warp() else { return };
        let mut rig = crate::inplace::Rig::new(&device).unwrap();
        let bytes = 32u64 << 20;
        let heap = D3D12_HEAP_PROPERTIES { Type: D3D12_HEAP_TYPE_DEFAULT, ..Default::default() };
        let buffer = |_| {
            let mut out: Option<ID3D12Resource> = None;
            // SAFETY: plain creation.
            unsafe {
                device
                    .CreateCommittedResource(&heap, D3D12_HEAP_FLAG_NONE, &crate::inplace::buffer_desc(bytes, D3D12_RESOURCE_FLAG_NONE), D3D12_RESOURCE_STATE_COMMON, None, &mut out)
                    .unwrap()
            };
            out.unwrap()
        };
        let (a, b) = (buffer(0), buffer(1));
        for s in &SUMS {
            s.store(0, Ordering::Relaxed);
        }
        begin(&device, &rig.queue, 0);
        let first = stamp(&device, 0, Mark::TitleBegin, false).unwrap();
        let last = stamp(&device, 0, Mark::TitleEnd, true).unwrap();
        let fence: ID3D12Fence = unsafe { device.CreateFence(0, D3D12_FENCE_FLAG_NONE).unwrap() };
        // SAFETY: closed lists of ours on the rig's queue, our fence.
        unsafe {
            rig.queue.ExecuteCommandLists(&[Some(first)]);
            rig.run(|list| list.CopyBufferRegion(&a, 0, &b, 0, bytes)).unwrap();
            rig.queue.ExecuteCommandLists(&[Some(last)]);
            rig.queue.Signal(&fence, 1).unwrap();
        }
        sent(1);
        rig.run(|_| {}).unwrap();
        assert_eq!(unsafe { fence.GetCompletedValue() }, 1);
        begin(&device, &rig.queue, 1);
        assert_eq!(SUMS[5].load(Ordering::Relaxed), 1, "one submission read");
        assert!(SUMS[1].load(Ordering::Relaxed) > 0, "the lists took time on the card");
        assert_eq!(SUMS[0].load(Ordering::Relaxed) + SUMS[2].load(Ordering::Relaxed), 0, "no upload, no merge");
        sent(2);
    }
}
