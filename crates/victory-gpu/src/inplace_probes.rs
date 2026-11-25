//! Stage 0 of the in-place design (`plans/tier3-design.md` §8): what this
//! PC's driver allows beyond what `crate::inplace`'s first probe asks.
//! Each probe is one or a few lines that begin with its name (P1..P7, and
//! R9/tiles for the title's own calls and tile mappings). Nothing here
//! touches the title's memory or the host's queue: memory of the probe's
//! own, a queue of its own (`inplace::Rig`).
//!
//! What the answers decide is in the design's table of risks; the lines
//! say "ok"/"REFUSED"/"false" plainly so that a log can be grepped.

use std::ffi::c_void;
use std::time::Instant;

use windows::Win32::Graphics::Direct3D::Fxc::D3DCompile;
use windows::Win32::Graphics::Direct3D::{D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST, ID3DBlob};
use windows::Win32::Graphics::Direct3D12::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::core::{Interface, PCSTR};
use windows_sys::Win32::System::Memory as mem;

use crate::inplace::{From, K64, MEM_WRITE_WATCH, Rig, buffer_desc, fill, hr, open, place, same, working_set_mb};

const MIB: usize = 1 << 20;
const GIB: usize = 1 << 30;

/// What the system's commit account says, in MiB: (commit total, commit
/// limit, physical total, physical available).
pub(crate) fn commit_mb() -> (u64, u64, u64, u64) {
    use windows_sys::Win32::System::ProcessStatus::{K32GetPerformanceInfo, PERFORMANCE_INFORMATION};
    let mut info = PERFORMANCE_INFORMATION { cb: size_of::<PERFORMANCE_INFORMATION>() as u32, ..Default::default() };
    // SAFETY: a sized structure filled in by the call.
    unsafe { K32GetPerformanceInfo(&mut info, info.cb) };
    let mb = |pages: usize| (pages as u64 * info.PageSize as u64) >> 20;
    (mb(info.CommitTotal), mb(info.CommitLimit), mb(info.PhysicalTotal), mb(info.PhysicalAvailable))
}

fn last_error() -> u32 {
    // SAFETY: a plain query of this thread's last error.
    unsafe { windows_sys::Win32::Foundation::GetLastError() }
}

/// Private memory as `xmem` makes the title's: committed, read-write,
/// write-watched. Released on drop.
struct Private(*mut u8);

impl Private {
    fn new(bytes: usize) -> Option<Private> {
        // SAFETY: a fresh allocation of our own.
        let at = unsafe { mem::VirtualAlloc(std::ptr::null(), bytes, mem::MEM_RESERVE | mem::MEM_COMMIT | MEM_WRITE_WATCH, mem::PAGE_READWRITE) };
        (!at.is_null()).then_some(Private(at.cast()))
    }
}

impl Drop for Private {
    fn drop(&mut self) {
        // SAFETY: our own allocation.
        unsafe { mem::VirtualFree(self.0.cast(), 0, mem::MEM_RELEASE) };
    }
}

/// A pagefile-backed section of our own and, if mapped, a view of all of
/// it. Closed on drop.
struct Section {
    handle: windows_sys::Win32::Foundation::HANDLE,
    view: *mut u8,
    bytes: usize,
}

impl Section {
    fn new(bytes: usize, committed: bool) -> Result<Section, u32> {
        let invalid = windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
        let kind = if committed { mem::SEC_COMMIT } else { mem::SEC_RESERVE };
        // SAFETY: plain section creation.
        let handle = unsafe {
            mem::CreateFileMappingW(invalid, std::ptr::null(), mem::PAGE_READWRITE | kind, (bytes as u64 >> 32) as u32, bytes as u32, std::ptr::null())
        };
        if handle.is_null() {
            return Err(last_error());
        }
        Ok(Section { handle, view: std::ptr::null_mut(), bytes })
    }

    fn map(&mut self) -> bool {
        // SAFETY: our section, mapped whole.
        let view = unsafe { mem::MapViewOfFile(self.handle, mem::FILE_MAP_WRITE, 0, 0, self.bytes) };
        self.view = view.Value.cast();
        !self.view.is_null()
    }
}

impl Drop for Section {
    fn drop(&mut self) {
        // SAFETY: our view and handle.
        unsafe {
            if !self.view.is_null() {
                mem::UnmapViewOfFile(mem::MEMORY_MAPPED_VIEW_ADDRESS { Value: self.view.cast() });
            }
            windows_sys::Win32::Foundation::CloseHandle(self.handle);
        }
    }
}

/// A compute or vertex program and its root signature (in the source:
/// `#define RS "..."` and `[RootSignature(RS)]`), compiled with FXC.
fn compile(device: &ID3D12Device, source: &str, target: &str) -> windows::core::Result<(ID3D12RootSignature, Vec<u8>)> {
    let target = std::ffi::CString::new(target).unwrap_or_default();
    let mut code: Option<ID3DBlob> = None;
    let mut errors: Option<ID3DBlob> = None;
    // SAFETY: the source and names live through the call.
    unsafe {
        let done = D3DCompile(source.as_ptr().cast(), source.len(), PCSTR(c"probe".as_ptr().cast()), None, None, PCSTR(c"main".as_ptr().cast()), PCSTR(target.as_ptr().cast()), 0, 0, &mut code, Some(&mut errors));
        if let Err(e) = done {
            if let Some(errors) = errors {
                let text = std::slice::from_raw_parts(errors.GetBufferPointer().cast::<u8>(), errors.GetBufferSize());
                eprintln!("probe shader: {}", String::from_utf8_lossy(text));
            }
            return Err(e);
        }
        let code = code.ok_or_else(windows::core::Error::empty)?;
        let bytes = std::slice::from_raw_parts(code.GetBufferPointer().cast::<u8>(), code.GetBufferSize()).to_vec();
        let root: ID3D12RootSignature = device.CreateRootSignature(0, &bytes)?;
        Ok((root, bytes))
    }
}

pub(crate) fn compute(device: &ID3D12Device, source: &str) -> windows::core::Result<(ID3D12RootSignature, ID3D12PipelineState)> {
    let (root, bytes) = compile(device, source, "cs_5_1")?;
    // SAFETY: the bytecode lives through the call; the root signature is
    // borrowed for it.
    let pso = unsafe {
        device.CreateComputePipelineState(&D3D12_COMPUTE_PIPELINE_STATE_DESC {
            pRootSignature: std::mem::transmute_copy(&root),
            CS: D3D12_SHADER_BYTECODE { pShaderBytecode: bytes.as_ptr().cast(), BytecodeLength: bytes.len() },
            ..Default::default()
        })?
    };
    Ok((root, pso))
}

/// A buffer of `bytes` in the card's memory, unordered access allowed.
fn video(device: &ID3D12Device, bytes: u64) -> windows::core::Result<ID3D12Resource> {
    let heap = D3D12_HEAP_PROPERTIES { Type: D3D12_HEAP_TYPE_DEFAULT, ..Default::default() };
    let mut out: Option<ID3D12Resource> = None;
    // SAFETY: plain creation.
    unsafe {
        device.CreateCommittedResource(&heap, D3D12_HEAP_FLAG_NONE, &buffer_desc(bytes, D3D12_RESOURCE_FLAG_ALLOW_UNORDERED_ACCESS), D3D12_RESOURCE_STATE_COMMON, None, &mut out)?
    };
    out.ok_or_else(windows::core::Error::empty)
}

fn uav_barrier() -> D3D12_RESOURCE_BARRIER {
    D3D12_RESOURCE_BARRIER {
        Type: D3D12_RESOURCE_BARRIER_TYPE_UAV,
        Flags: D3D12_RESOURCE_BARRIER_FLAG_NONE,
        Anonymous: D3D12_RESOURCE_BARRIER_0 { UAV: std::mem::ManuallyDrop::new(D3D12_RESOURCE_UAV_BARRIER { pResource: std::mem::ManuallyDrop::new(None) }) },
    }
}

/// The debug layer's messages since the last call: (errors and
/// corruptions, warnings, the first error's text). Nothing without the
/// layer.
fn messages(device: &ID3D12Device) -> Option<(u32, u32, String)> {
    let queue = device.cast::<ID3D12InfoQueue>().ok()?;
    let (mut errors, mut warnings, mut first) = (0, 0, String::new());
    // SAFETY: each message read into a buffer of the size the queue gives.
    unsafe {
        for i in 0..queue.GetNumStoredMessages() {
            let mut size = 0;
            if queue.GetMessage(i, None, &mut size).is_err() || size == 0 {
                continue;
            }
            let mut buffer = vec![0u64; size.div_ceil(8)];
            let message = buffer.as_mut_ptr().cast::<D3D12_MESSAGE>();
            if queue.GetMessage(i, Some(message), &mut size).is_ok() {
                let m = &*message;
                if m.Severity.0 <= D3D12_MESSAGE_SEVERITY_ERROR.0 {
                    errors += 1;
                    if first.is_empty() {
                        let text = std::slice::from_raw_parts(m.pDescription, m.DescriptionByteLength);
                        first = String::from_utf8_lossy(text).trim_end_matches('\0').to_owned();
                    }
                } else if m.Severity.0 == D3D12_MESSAGE_SEVERITY_WARNING.0 {
                    warnings += 1;
                }
            }
        }
        queue.ClearStoredMessages();
    }
    Some((errors, warnings, first))
}

fn debug_said(device: &ID3D12Device) -> String {
    match messages(device) {
        None => String::new(),
        Some((0, w, _)) => format!(" [debug layer: no errors, {w} warnings]"),
        Some((e, w, first)) => format!(" [debug layer: {e} ERRORS, {w} warnings; first: {first}]"),
    }
}

/// All the Stage 0 probes. `big`: also the ones that lock or commit
/// gigabytes (P1, P2's 4 GiB, P7); `tiles`: also the tile mapping into an
/// opened heap, which removes the device (only on a device of the probe's
/// own, never the host's).
pub(crate) fn stage0(lines: &mut Vec<String>, rig: &mut Rig, big: bool, tiles: bool) {
    let device = rig.device.clone();
    let _ = messages(&device);
    let mut va = D3D12_FEATURE_DATA_GPU_VIRTUAL_ADDRESS_SUPPORT::default();
    // SAFETY: a plain query that fills our structure.
    let asked = unsafe { device.CheckFeatureSupport(D3D12_FEATURE_GPU_VIRTUAL_ADDRESS_SUPPORT, (&raw mut va).cast(), size_of_val(&va) as u32) };
    let (total, limit, physical, available) = commit_mb();
    lines.push(format!(
        "P1 GPU virtual address support: {} bits per resource, {} per process ({}); commit {total} of {limit} MB, RAM {available} of {physical} MB free",
        va.MaxGPUVirtualAddressBitsPerResource,
        va.MaxGPUVirtualAddressBitsPerProcess,
        hr(&asked)
    ));
    let mut step = |name: &str, lines: &mut Vec<String>, f: &mut dyn FnMut(&mut Vec<String>, &mut Rig)| {
        let started = Instant::now();
        f(lines, rig);
        let removed = rig.removed();
        if let Some(reason) = &removed {
            lines.push(format!("{name}: THE DEVICE WAS REMOVED ({reason})"));
        }
        tracing::debug!("{name} took {:.0} ms", started.elapsed().as_secs_f64() * 1000.0);
        removed.is_none()
    };
    if big && !step("P1", lines, &mut |l, r| p1(l, r)) {
        return;
    }
    if !step("P2", lines, &mut |l, r| p2(l, r, big)) {
        return;
    }
    if !step("P3", lines, &mut p3) {
        return;
    }
    if !step("P4", lines, &mut p4) {
        return;
    }
    if !step("P5", lines, &mut p5) {
        return;
    }
    if !step("P6", lines, &mut p6) {
        return;
    }
    if big && !step("P7", lines, &mut p7) {
        return;
    }
    if !step("R9", lines, &mut r9) {
        return;
    }
    if tiles {
        step("tiles", lines, &mut tile_probe);
    }
}

/// The GPU writes a 64 KiB pattern by a copy, and a shader through a root
/// view, in the last 64 KiB of `buffer` (`bytes` long), and the CPU looks
/// at `memory` (the same bytes' CPU address): (copy, shader).
fn ends_written(rig: &mut Rig, buffer: &ID3D12Resource, memory: *mut u8, bytes: usize) -> (bool, bool) {
    let last = bytes - K64;
    let new: Vec<u8> = (0..K64).map(|i| (i as u8) ^ 0x6c).collect();
    // SAFETY: the caller's memory holds `bytes` bytes.
    let copied = rig.store(buffer, last as u64, &new).is_ok() && unsafe { same(memory.add(last), &new) };
    // SAFETY: as above.
    let shaded = unsafe {
        memory.add(last).cast::<[u32; 3]>().write([40, 2, 0]);
        rig.shade(buffer.GetGPUVirtualAddress() + last as u64).is_ok() && memory.add(last).cast::<[u32; 3]>().read()[2] == 42
    };
    (copied, shaded)
}

/// P1 (R1): the biggest parts and sections the driver opens, and a buffer
/// over all of each, written at its far end.
fn p1(lines: &mut Vec<String>, rig: &mut Rig) {
    let device = rig.device.clone();
    for (name, bytes) in [
        ("2 GiB", 2 * GIB),
        ("4095 MiB", 4095 * MIB),
        ("4 GiB - 64 KiB", 4 * GIB - K64),
        ("4 GiB", 4 * GIB),
        ("6 GiB", 6 * GIB),
    ] {
        let Some(memory) = Private::new(bytes) else {
            lines.push(format!("P1 part {name}: no memory to try with (error {})", last_error()));
            continue;
        };
        let started = Instant::now();
        let opened = open(&device, From::Part(memory.0 as usize, bytes));
        let took = started.elapsed().as_secs_f64() * 1000.0;
        let mut line = format!("P1 part {name}: open {} in {took:.0} ms", hr(&opened));
        if let Ok(heap) = &opened {
            let placed = place(&device, heap, 0, bytes as u64);
            line += &format!("; a buffer over all of it {}", hr(&placed));
            if let Ok(buffer) = &placed {
                let (copied, shaded) = ends_written(rig, buffer, memory.0, bytes);
                line += &format!("; its last 64 KiB written by a copy {copied}, by a shader {shaded}");
            }
            // A buffer of 4 GiB - 64 KiB from offset 2 GiB, as overlapping
            // arenas of a bigger region would place it.
            if bytes >= 4 * GIB {
                let len = (bytes - 2 * GIB).min(4 * GIB - K64);
                let second = place(&device, heap, 2 * GIB as u64, len as u64);
                line += &format!("; a buffer of {} MiB from 2 GiB {}", len / MIB, hr(&second));
            }
        }
        line += &debug_said(&device);
        lines.push(line);
        drop(opened);
        if rig.removed().is_some() {
            return;
        }
    }
    for (name, bytes) in [("2 GiB", 2 * GIB), ("4095 MiB", 4095 * MIB), ("6 GiB", 6 * GIB)] {
        let mut section = match Section::new(bytes, true) {
            Ok(s) => s,
            Err(e) => {
                lines.push(format!("P1 section {name}: SEC_COMMIT refused (error {e})"));
                continue;
            }
        };
        let started = Instant::now();
        let opened = open(&device, From::Section(section.handle as usize));
        let took = started.elapsed().as_secs_f64() * 1000.0;
        let mut line = format!("P1 section {name}: open {} in {took:.1} ms", hr(&opened));
        if let Ok(heap) = &opened {
            let full = place(&device, heap, 0, bytes.min(4 * GIB - K64) as u64);
            line += &format!("; a buffer of {} MiB from 0 {}", bytes.min(4 * GIB - K64) / MIB, hr(&full));
            let tail = (bytes as u64).saturating_sub(GIB as u64);
            let end = place(&device, heap, tail, GIB as u64);
            line += &format!("; 1 GiB from {} MiB {}", tail >> 20, hr(&end));
            if let Ok(buffer) = &end
                && section.map()
            {
                // SAFETY: the view holds all of the section.
                let (copied, shaded) = ends_written(rig, buffer, unsafe { section.view.add(tail as usize) }, GIB);
                line += &format!("; its last 64 KiB written by a copy {copied}, by a shader {shaded}");
            }
        }
        line += &debug_said(&device);
        lines.push(line);
        drop(opened);
        if rig.removed().is_some() {
            return;
        }
    }
}

/// P2 (R2, R3): what opening private memory costs (it locks every page),
/// and whether a section's pages are locked by opening or by the GPU's
/// first use.
fn p2(lines: &mut Vec<String>, rig: &mut Rig, big: bool) {
    let device = rig.device.clone();
    let sizes: &[usize] = if big { &[GIB, 4 * GIB - K64] } else { &[GIB] };
    for &bytes in sizes {
        let Some(memory) = Private::new(bytes) else {
            lines.push(format!("P2 {} MiB: no memory", bytes / MIB));
            continue;
        };
        let (set, commit) = (working_set_mb(), commit_mb().0);
        let started = Instant::now();
        let opened = open(&device, From::Part(memory.0 as usize, bytes));
        let took = started.elapsed().as_secs_f64() * 1000.0;
        let (set_open, commit_open) = (working_set_mb(), commit_mb().0);
        let started = Instant::now();
        drop(opened);
        let closed = started.elapsed().as_secs_f64() * 1000.0;
        lines.push(format!(
            "P2 private {} MiB never touched: opened in {took:.0} ms, working set {set} -> {set_open} MB, commit {commit} -> {commit_open} MB; \
             the heap went in {closed:.0} ms, working set then {} MB",
            bytes / MIB,
            working_set_mb()
        ));
        drop(memory);
    }
    // A section: opened, then the GPU reads 64 KiB of it.
    if let Ok(section) = Section::new(GIB, true) {
        let set = working_set_mb();
        let opened = open(&device, From::Section(section.handle as usize));
        let set_open = working_set_mb();
        let mut read = 0;
        if let Ok(heap) = &opened
            && let Ok(buffer) = place(&device, heap, 0, GIB as u64)
        {
            let _ = rig.fetch(&buffer, 512 * MIB as u64, K64);
            read = working_set_mb();
        }
        lines.push(format!(
            "P2 committed section 1 GiB never touched: open {}; working set {set} -> {set_open} MB opened -> {read} MB after the GPU read 64 KiB of it",
            hr(&opened)
        ));
    }
}

/// P3 (R4): `InterlockedOr`/`InterlockedAnd` by a shader into private
/// memory in place and into a section, checked on the CPU; and whether a
/// CPU writing other bytes of the same dwords meanwhile keeps its bytes.
fn p3(lines: &mut Vec<String>, rig: &mut Rig) {
    const SOURCE: &str = "#define RS \"RootConstants(num32BitConstants=1, b0), UAV(u0)\"\n\
        cbuffer C : register(b0) { uint loops; };\n\
        RWByteAddressBuffer b : register(u0);\n\
        [RootSignature(RS)]\n[numthreads(32, 1, 1)]\n\
        void main(uint3 g : SV_GroupID, uint3 t : SV_GroupThreadID) {\n\
          uint bit = 1u << t.x;\n\
          b.InterlockedOr(g.x * 4, bit);\n\
          b.InterlockedAnd(128 + g.x * 4, ~bit);\n\
          if (t.x == 0) { b.InterlockedAnd(256 + g.x * 4, 0xFFFF00FFu); b.InterlockedOr(256 + g.x * 4, 0x0000AB00u); }\n\
          for (uint i = 0; i < loops; i++) { b.InterlockedOr(512 + g.x * 4, 1u << (16 + (i + t.x) % 16)); }\n\
        }\n";
    let device = rig.device.clone();
    let (root, pso) = match compute(&device, SOURCE) {
        Ok(p) => p,
        Err(e) => {
            lines.push(format!("P3: no shader ({e})"));
            return;
        }
    };
    let try_on =|name: &str, heap: &ID3D12Heap, offset: u64, memory: *mut u8, lines: &mut Vec<String>, rig: &mut Rig| {
        let Ok(buffer) = place(&device, heap, offset, K64 as u64) else {
            lines.push(format!("P3 {name}: no buffer"));
            return;
        };
        // SAFETY: the buffer's 64 KiB at `memory`.
        let words = unsafe { std::slice::from_raw_parts_mut(memory.cast::<u32>(), K64 / 4) };
        words[..32].fill(0);
        words[32..64].fill(u32::MAX);
        words[64..96].fill(0x1122_3344);
        words[128..160].fill(0);
        let gpu = unsafe { buffer.GetGPUVirtualAddress() };
        // The CPU writes byte 0 of dwords 128..160 while the shader ORs
        // bits 16..31 into them, until the GPU is done.
        let stop = std::sync::atomic::AtomicBool::new(false);
        let at = memory as usize;
        let mut last = 0u8;
        let ran = std::thread::scope(|s| {
            let cpu = s.spawn(|| {
                let mut k = 0u8;
                while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                    k = k.wrapping_add(1).max(1);
                    for w in 128..160 {
                        // SAFETY: inside the buffer's 64 KiB.
                        unsafe { std::ptr::write_volatile((at as *mut u8).add(w * 4), k) };
                    }
                }
                k
            });
            let started = Instant::now();
            // SAFETY: an open list of ours; live buffers.
            let ran = rig.run(|list| unsafe {
                list.SetComputeRootSignature(&root);
                list.SetPipelineState(&pso);
                list.SetComputeRoot32BitConstant(0, 50_000, 0);
                list.SetComputeRootUnorderedAccessView(1, gpu);
                list.Dispatch(32, 1, 1);
            });
            let ms = started.elapsed().as_secs_f64() * 1000.0;
            stop.store(true, std::sync::atomic::Ordering::Relaxed);
            last = cpu.join().unwrap_or(0);
            ran.map(|()| ms)
        });
        let ors = words[..32].iter().all(|&w| w == u32::MAX);
        let ands = words[32..64].iter().all(|&w| w == 0);
        let bytes = words[64..96].iter().all(|&w| w == 0x1122_ab44);
        let kept = words[128..160].iter().filter(|&&w| w & 0xff == u32::from(last)).count();
        let bits = words[128..160].iter().filter(|&&w| w >> 16 == 0xffff).count();
        lines.push(format!(
            "P3 {name}: shader atomics ran {} ({}); InterlockedOr {ors}, InterlockedAnd {ands}, a byte replaced by And+Or with the other bytes kept {bytes}; \
             with the CPU writing byte 0 of 32 dwords all along: {kept} of 32 kept the CPU's last byte, {bits} of 32 have all the GPU's bits{}",
            hr(&ran),
            ran.as_ref().map_or("-".to_owned(), |ms| format!("{ms:.1} ms")),
            debug_said(&device)
        ));
    };
    if let Some(memory) = Private::new(4 * K64)
        && let Ok(heap) = open(&device, From::Part(memory.0 as usize, 4 * K64))
    {
        // SAFETY: inside the allocation.
        try_on("private part", &heap, K64 as u64, unsafe { memory.0.add(K64) }, lines, rig);
    } else {
        lines.push("P3 private part: not opened".to_owned());
    }
    if let Ok(mut section) = Section::new(4 * K64, true)
        && section.map()
        && let Ok(heap) = open(&device, From::Section(section.handle as usize))
    {
        // SAFETY: inside the view.
        try_on("committed section", &heap, 2 * K64 as u64, unsafe { section.view.add(2 * K64) }, lines, rig);
    } else {
        lines.push("P3 committed section: not opened".to_owned());
    }
}

/// The pages `GetWriteWatch` reports in `[at, at + bytes)` (and resets).
fn watched(at: *mut u8, bytes: usize) -> Option<Vec<usize>> {
    let mut found = vec![std::ptr::null_mut::<c_void>(); bytes / 4096];
    let (mut count, mut granularity) = (found.len(), 0u32);
    // SAFETY: room for `count` addresses.
    // WRITE_WATCH_FLAG_RESET.
    let ok = unsafe { mem::GetWriteWatch(1, at.cast(), bytes, found.as_mut_ptr(), &mut count, &mut granularity) } == 0;
    ok.then(|| {
        let mut pages: Vec<usize> = found[..count].iter().map(|&p| (p as usize - at as usize) / K64).collect();
        pages.dedup();
        pages
    })
}

/// P4 (R5): with a part opened (its pages locked), CPU writes are still
/// reported by write watching, GPU writes (a copy, a shader) are not.
fn p4(lines: &mut Vec<String>, rig: &mut Rig) {
    let device = rig.device.clone();
    let Some(memory) = Private::new(16 * K64) else {
        return;
    };
    let Ok(heap) = open(&device, From::Part(memory.0 as usize, 16 * K64)) else {
        lines.push("P4: not opened".to_owned());
        return;
    };
    let Ok(buffer) = place(&device, &heap, 0, 16 * K64 as u64) else {
        return;
    };
    let _ = watched(memory.0, 16 * K64);
    // SAFETY: inside the allocation.
    unsafe {
        for page in [1, 5, 9] {
            memory.0.add(page * K64 + 100).write(7);
        }
    }
    let cpu = watched(memory.0, 16 * K64);
    let new = vec![0x5au8; 2 * K64];
    let copied = rig.store(&buffer, 3 * K64 as u64, &new).is_ok();
    // SAFETY: a live buffer.
    let shaded = rig.shade(unsafe { buffer.GetGPUVirtualAddress() } + 12 * K64 as u64).is_ok();
    let gpu = watched(memory.0, 16 * K64);
    // A CPU write after the GPU's, to be sure the asking still works.
    // SAFETY: inside the allocation.
    unsafe { memory.0.add(14 * K64).write(1) };
    let again = watched(memory.0, 16 * K64);
    lines.push(format!(
        "P4 write watching with the part opened: CPU wrote pages [1, 5, 9], reported {cpu:?}; then the GPU wrote pages 3-4 by a copy ({copied}) and 12 by a shader ({shaded}), \
         reported {gpu:?}; then the CPU wrote page 14, reported {again:?}"
    ));
}

/// P5 (R6): many small parts of one allocation, and two overlapping parts
/// of it written through one and read through the other in one list.
fn p5(lines: &mut Vec<String>, rig: &mut Rig) {
    let device = rig.device.clone();
    const N: usize = 512;
    let Some(memory) = Private::new(N * K64) else {
        return;
    };
    let started = Instant::now();
    let mut heaps = Vec::with_capacity(N);
    let mut failed = 0;
    let mut first_error = String::new();
    for i in 0..N {
        // SAFETY: inside the allocation.
        match open(&device, From::Part(unsafe { memory.0.add(i * K64) } as usize, K64)).and_then(|heap| place(&device, &heap, 0, K64 as u64).map(|b| (heap, b))) {
            Ok(pair) => heaps.push(pair),
            Err(e) => {
                failed += 1;
                if first_error.is_empty() {
                    first_error = hr(&Err::<(), _>(e));
                }
            }
        }
    }
    let took = started.elapsed().as_secs_f64() * 1000.0;
    // The GPU reads the last one.
    // SAFETY: inside the allocation.
    unsafe { fill(memory.0.add((N - 1) * K64), K64, 9) };
    let read = heaps.last().is_some_and(|(_, b)| rig.fetch(b, 0, K64).is_ok_and(|got| unsafe { same(memory.0.add((N - 1) * K64), &got) }));
    let started = Instant::now();
    drop(heaps);
    let freed = started.elapsed().as_secs_f64() * 1000.0;
    lines.push(format!(
        "P5 {N} parts of 64 KiB opened with a buffer each in {took:.0} ms ({:.2} ms each), {failed} failed {first_error}; the GPU reads the last {read}; all gone in {freed:.0} ms",
        took / N as f64
    ));
    // Overlap: A = pages 0..4, B = pages 2..6. Written through A at pages
    // 2..3, a UAV barrier, read through B into a buffer of ours.
    const SOURCE: &str = "#define RS \"RootConstants(num32BitConstants=1, b0), UAV(u0), UAV(u1)\"\n\
        cbuffer C : register(b0) { uint mode; };\n\
        RWByteAddressBuffer a : register(u0);\nRWByteAddressBuffer o : register(u1);\n\
        [RootSignature(RS)]\n[numthreads(64, 1, 1)]\n\
        void main(uint3 t : SV_DispatchThreadID) { if (mode == 0) a.Store(t.x * 4, 0xC0DE0000u + t.x); else o.Store(t.x * 4, a.Load(t.x * 4) + 1); }\n";
    let Ok((root, pso)) = compute(&device, SOURCE) else {
        lines.push("P5 overlap: no shader".to_owned());
        return;
    };
    let (a, b) = (open(&device, From::Part(memory.0 as usize, 4 * K64)), open(&device, From::Part(unsafe { memory.0.add(2 * K64) } as usize, 4 * K64)));
    let (Ok(a), Ok(b)) = (a, b) else {
        lines.push("P5 overlap: the second part was REFUSED".to_owned());
        return;
    };
    let (Ok(ab), Ok(bb), Ok(out)) = (place(&device, &a, 0, 4 * K64 as u64), place(&device, &b, 0, 4 * K64 as u64), video(&device, K64 as u64)) else {
        return;
    };
    // SAFETY: live buffers.
    let (ga, gb, go) = unsafe { (ab.GetGPUVirtualAddress() + 2 * K64 as u64, bb.GetGPUVirtualAddress(), out.GetGPUVirtualAddress()) };
    let groups = 1024 / 64;
    let ran = rig.run(|list| unsafe {
        list.SetComputeRootSignature(&root);
        list.SetPipelineState(&pso);
        list.SetComputeRoot32BitConstant(0, 0, 0);
        list.SetComputeRootUnorderedAccessView(1, ga);
        list.SetComputeRootUnorderedAccessView(2, go);
        list.Dispatch(groups, 1, 1);
        list.ResourceBarrier(&[uav_barrier()]);
        list.SetComputeRoot32BitConstant(0, 1, 0);
        list.SetComputeRootUnorderedAccessView(1, gb);
        list.Dispatch(groups, 1, 1);
    });
    let got = rig.fetch(&out, 0, 4096).unwrap_or_default();
    let through = got.len() == 4096 && (0..1024).all(|i| u32::from_le_bytes(got[i * 4..i * 4 + 4].try_into().unwrap_or_default()) == 0xC0DE_0001 + i as u32);
    // SAFETY: inside the allocation.
    let cpu = unsafe { std::slice::from_raw_parts(memory.0.add(2 * K64).cast::<u32>(), 1024) }.iter().enumerate().all(|(i, &w)| w == 0xC0DE_0000 + i as u32);
    lines.push(format!(
        "P5 two overlapping parts: open both ok; written through one, a UAV barrier, read through the other in the same list: {through} ({}); the CPU sees the write {cpu}{}",
        hr(&ran),
        debug_said(&device)
    ));
}

/// P6 (R7): a buffer in an opened heap used every way the title's commands
/// use title memory: vertex and index views, a root constant buffer view,
/// typed and raw views in a table, `ExecuteIndirect` arguments and count,
/// `CopyTextureRegion` both ways, `ResolveQueryData`.
fn p6(lines: &mut Vec<String>, rig: &mut Rig) {
    let device = rig.device.clone();
    let Some(memory) = Private::new(8 * K64) else {
        return;
    };
    let Ok(heap) = open(&device, From::Part(memory.0 as usize, 8 * K64)) else {
        lines.push("P6: not opened".to_owned());
        return;
    };
    let Ok(buffer) = place(&device, &heap, 0, 8 * K64 as u64) else {
        return;
    };
    let Ok(out) = video(&device, K64 as u64) else {
        return;
    };
    // SAFETY: live buffers.
    let (gpu, go) = unsafe { (buffer.GetGPUVirtualAddress(), out.GetGPUVirtualAddress()) };
    let m = memory.0;
    let word = |at: usize| unsafe { m.add(at).cast::<u32>().read_volatile() };
    let put = |at: usize, v: &[u32]| unsafe { std::ptr::copy_nonoverlapping(v.as_ptr(), m.add(at).cast::<u32>(), v.len()) };
    let read_out = |rig: &mut Rig, n: usize| -> Vec<u32> {
        rig.fetch(&out, 0, n * 4).map(|b| b.chunks_exact(4).map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect()).unwrap_or_default()
    };
    let clear = |rig: &mut Rig| rig.store(&out, 0, &[0u8; 64]).is_ok();
    let _ = messages(&device);

    // Vertex and index views: the vertex shader ORs each vertex's word in.
    {
        const VS: &str = "#define RS \"RootFlags(ALLOW_INPUT_ASSEMBLER_INPUT_LAYOUT), UAV(u0)\"\n\
            RWByteAddressBuffer o : register(u0);\n\
            [RootSignature(RS)]\n\
            float4 main(uint id : ID) : SV_Position { o.InterlockedOr(0, id); return float4(0, 0, 0, 1); }\n";
        let line = (|| -> windows::core::Result<bool> {
            let (root, code) = compile(&device, VS, "vs_5_1")?;
            let element = D3D12_INPUT_ELEMENT_DESC {
                SemanticName: PCSTR(c"ID".as_ptr().cast()),
                SemanticIndex: 0,
                Format: DXGI_FORMAT_R32_UINT,
                InputSlot: 0,
                AlignedByteOffset: 0,
                InputSlotClass: D3D12_INPUT_CLASSIFICATION_PER_VERTEX_DATA,
                InstanceDataStepRate: 0,
            };
            let mut desc = D3D12_GRAPHICS_PIPELINE_STATE_DESC {
                // SAFETY: borrowed for the call.
                pRootSignature: unsafe { std::mem::transmute_copy(&root) },
                VS: D3D12_SHADER_BYTECODE { pShaderBytecode: code.as_ptr().cast(), BytecodeLength: code.len() },
                SampleMask: u32::MAX,
                RasterizerState: D3D12_RASTERIZER_DESC { FillMode: D3D12_FILL_MODE_SOLID, CullMode: D3D12_CULL_MODE_NONE, DepthClipEnable: true.into(), ..Default::default() },
                InputLayout: D3D12_INPUT_LAYOUT_DESC { pInputElementDescs: &element, NumElements: 1 },
                PrimitiveTopologyType: D3D12_PRIMITIVE_TOPOLOGY_TYPE_TRIANGLE,
                SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                ..Default::default()
            };
            desc.DepthStencilState.DepthEnable = false.into();
            // SAFETY: a full description; the borrowed pointers outlive it.
            let pso: ID3D12PipelineState = unsafe { device.CreateGraphicsPipelineState(&desc)? };
            // Vertices at 64 KiB: words 0x100, 1, 0x200, 2, 0x400, 4;
            // indices at 64 KiB + 4 KiB pick 1, 3, 5.
            put(K64, &[0x100, 1, 0x200, 2, 0x400, 4]);
            put(K64 + 4096, &[1, 3, 5]);
            clear(rig);
            rig.run(|list| unsafe {
                list.SetGraphicsRootSignature(&root);
                list.SetPipelineState(&pso);
                list.SetGraphicsRootUnorderedAccessView(0, go);
                list.IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
                list.IASetVertexBuffers(0, Some(&[D3D12_VERTEX_BUFFER_VIEW { BufferLocation: gpu + K64 as u64, SizeInBytes: 24, StrideInBytes: 4 }]));
                list.IASetIndexBuffer(Some(&D3D12_INDEX_BUFFER_VIEW { BufferLocation: gpu + K64 as u64 + 4096, SizeInBytes: 12, Format: DXGI_FORMAT_R32_UINT }));
                list.RSSetViewports(&[D3D12_VIEWPORT { Width: 4.0, Height: 4.0, MaxDepth: 1.0, ..Default::default() }]);
                list.RSSetScissorRects(&[windows::Win32::Foundation::RECT { left: 0, top: 0, right: 4, bottom: 4 }]);
                list.DrawIndexedInstanced(3, 1, 0, 0, 0);
            })?;
            Ok(read_out(rig, 1).first() == Some(&7))
        })();
        lines.push(format!("P6 vertex and index buffer views in the opened heap: {}{}", ok_or(&line), debug_said(&device)));
    }

    // A root constant buffer view, and typed and raw views in a table.
    {
        const CS: &str = "#define RS \"CBV(b0), UAV(u0), DescriptorTable(SRV(t0), UAV(u1))\"\n\
            cbuffer C : register(b0) { uint4 v; };\n\
            RWByteAddressBuffer o : register(u0);\nBuffer<uint> s : register(t0);\nRWByteAddressBuffer r : register(u1);\n\
            [RootSignature(RS)]\n[numthreads(1, 1, 1)]\n\
            void main() { o.Store4(0, v); o.Store(16, s[3]); o.Store(20, r.Load(8)); r.Store(12, 0xFEED); }\n";
        let line = (|| -> windows::core::Result<bool> {
            let (root, pso) = compute(&device, CS)?;
            // SAFETY: plain creation.
            let table: ID3D12DescriptorHeap = unsafe {
                device.CreateDescriptorHeap(&D3D12_DESCRIPTOR_HEAP_DESC {
                    Type: D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV,
                    NumDescriptors: 2,
                    Flags: D3D12_DESCRIPTOR_HEAP_FLAG_SHADER_VISIBLE,
                    NodeMask: 0,
                })?
            };
            let step = unsafe { device.GetDescriptorHandleIncrementSize(D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV) } as usize;
            let cpu = unsafe { table.GetCPUDescriptorHandleForHeapStart() };
            // Typed view from word 1024 (offset 4 KiB), raw view from 8 KiB.
            let srv = D3D12_SHADER_RESOURCE_VIEW_DESC {
                Format: DXGI_FORMAT_R32_UINT,
                ViewDimension: D3D12_SRV_DIMENSION_BUFFER,
                Shader4ComponentMapping: D3D12_DEFAULT_SHADER_4_COMPONENT_MAPPING,
                Anonymous: D3D12_SHADER_RESOURCE_VIEW_DESC_0 { Buffer: D3D12_BUFFER_SRV { FirstElement: 1024, NumElements: 16, ..Default::default() } },
            };
            let uav = D3D12_UNORDERED_ACCESS_VIEW_DESC {
                Format: DXGI_FORMAT_R32_TYPELESS,
                ViewDimension: D3D12_UAV_DIMENSION_BUFFER,
                Anonymous: D3D12_UNORDERED_ACCESS_VIEW_DESC_0 { Buffer: D3D12_BUFFER_UAV { FirstElement: 2048, NumElements: 16, Flags: D3D12_BUFFER_UAV_FLAG_RAW, ..Default::default() } },
            };
            // SAFETY: two slots of our table.
            unsafe {
                device.CreateShaderResourceView(&buffer, Some(&srv), cpu);
                device.CreateUnorderedAccessView(&buffer, None::<&ID3D12Resource>, Some(&uav), D3D12_CPU_DESCRIPTOR_HANDLE { ptr: cpu.ptr + step });
            }
            put(2 * K64, &[11, 22, 33, 44]);
            put(4096 + 12, &[0x5151]);
            put(8192 + 8, &[0x6262]);
            clear(rig);
            rig.run(|list| unsafe {
                list.SetDescriptorHeaps(&[Some(table.clone())]);
                list.SetComputeRootSignature(&root);
                list.SetPipelineState(&pso);
                list.SetComputeRootConstantBufferView(0, gpu + 2 * K64 as u64);
                list.SetComputeRootUnorderedAccessView(1, go);
                list.SetComputeRootDescriptorTable(2, table.GetGPUDescriptorHandleForHeapStart());
                list.Dispatch(1, 1, 1);
            })?;
            let got = read_out(rig, 6);
            Ok(got == [11, 22, 33, 44, 0x5151, 0x6262] && word(8192 + 12) == 0xFEED)
        })();
        lines.push(format!(
            "P6 root constant buffer view, typed view and raw view (written too) in a table, all in the opened heap: {}{}",
            ok_or(&line),
            debug_said(&device)
        ));
    }

    // ExecuteIndirect: arguments and count in the opened heap.
    {
        const CS: &str = "#define RS \"UAV(u0)\"\nRWByteAddressBuffer o : register(u0);\n[RootSignature(RS)]\n[numthreads(1, 1, 1)]\nvoid main() { o.InterlockedAdd(0, 1); }\n";
        let line = (|| -> windows::core::Result<bool> {
            let (root, pso) = compute(&device, CS)?;
            let argument = D3D12_INDIRECT_ARGUMENT_DESC { Type: D3D12_INDIRECT_ARGUMENT_TYPE_DISPATCH, ..Default::default() };
            let mut signature: Option<ID3D12CommandSignature> = None;
            // SAFETY: a description of one dispatch.
            unsafe {
                device.CreateCommandSignature(
                    &D3D12_COMMAND_SIGNATURE_DESC { ByteStride: 12, NumArgumentDescs: 1, pArgumentDescs: &argument, NodeMask: 0 },
                    None::<&ID3D12RootSignature>,
                    &mut signature,
                )?
            };
            let signature = signature.ok_or_else(windows::core::Error::empty)?;
            put(3 * K64, &[5, 1, 1, 7, 1, 1]);
            put(3 * K64 + 256, &[2]);
            clear(rig);
            rig.run(|list| unsafe {
                list.SetComputeRootSignature(&root);
                list.SetPipelineState(&pso);
                list.SetComputeRootUnorderedAccessView(0, go);
                list.ExecuteIndirect(&signature, 4, &buffer, 3 * K64 as u64, &buffer, 3 * K64 as u64 + 256);
            })?;
            Ok(read_out(rig, 1).first() == Some(&12))
        })();
        lines.push(format!("P6 ExecuteIndirect with arguments and count in the opened heap: {}{}", ok_or(&line), debug_said(&device)));
    }

    // CopyTextureRegion from the heap into a texture and back.
    {
        let line = (|| -> windows::core::Result<bool> {
            let desc = D3D12_RESOURCE_DESC {
                Dimension: D3D12_RESOURCE_DIMENSION_TEXTURE2D,
                Width: 64,
                Height: 4,
                DepthOrArraySize: 1,
                MipLevels: 1,
                Format: DXGI_FORMAT_R8G8B8A8_UINT,
                SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                Layout: D3D12_TEXTURE_LAYOUT_UNKNOWN,
                ..Default::default()
            };
            let mut texture: Option<ID3D12Resource> = None;
            let props = D3D12_HEAP_PROPERTIES { Type: D3D12_HEAP_TYPE_DEFAULT, ..Default::default() };
            // SAFETY: plain creation.
            unsafe { device.CreateCommittedResource(&props, D3D12_HEAP_FLAG_NONE, &desc, D3D12_RESOURCE_STATE_COPY_DEST, None, &mut texture)? };
            let texture = texture.ok_or_else(windows::core::Error::empty)?;
            // SAFETY: inside the allocation.
            unsafe { fill(m.add(4 * K64), 1024, 33) };
            let footprint = |offset: u64| D3D12_TEXTURE_COPY_LOCATION {
                // SAFETY: borrowed for the call.
                pResource: unsafe { std::mem::transmute_copy(&buffer) },
                Type: D3D12_TEXTURE_COPY_TYPE_PLACED_FOOTPRINT,
                Anonymous: D3D12_TEXTURE_COPY_LOCATION_0 {
                    PlacedFootprint: D3D12_PLACED_SUBRESOURCE_FOOTPRINT {
                        Offset: offset,
                        Footprint: D3D12_SUBRESOURCE_FOOTPRINT { Format: DXGI_FORMAT_R8G8B8A8_UINT, Width: 64, Height: 4, Depth: 1, RowPitch: 256 },
                    },
                },
            };
            let sub = D3D12_TEXTURE_COPY_LOCATION {
                // SAFETY: borrowed for the call.
                pResource: unsafe { std::mem::transmute_copy(&texture) },
                Type: D3D12_TEXTURE_COPY_TYPE_SUBRESOURCE_INDEX,
                Anonymous: D3D12_TEXTURE_COPY_LOCATION_0 { SubresourceIndex: 0 },
            };
            // One list: the opened heap's buffer is a copy source, then
            // (after an explicit transition: implicit promotion happens
            // once per list, risk R8) a copy destination.
            rig.run(|list| unsafe {
                list.CopyTextureRegion(&sub, 0, 0, 0, &footprint(4 * K64 as u64), None);
                list.ResourceBarrier(&[
                    crate::host::transition(&texture, D3D12_RESOURCE_STATE_COPY_DEST, D3D12_RESOURCE_STATE_COPY_SOURCE),
                    crate::host::transition(&buffer, D3D12_RESOURCE_STATE_COPY_SOURCE, D3D12_RESOURCE_STATE_COPY_DEST),
                ]);
                list.CopyTextureRegion(&footprint(5 * K64 as u64), 0, 0, 0, &sub, None);
            })?;
            // SAFETY: inside the allocation.
            Ok(unsafe { std::slice::from_raw_parts(m.add(4 * K64), 1024) == std::slice::from_raw_parts(m.add(5 * K64), 1024) })
        })();
        lines.push(format!("P6 CopyTextureRegion from the opened heap into a texture and back into it: {}{}", ok_or(&line), debug_said(&device)));
    }

    // ResolveQueryData into the heap.
    {
        let line = (|| -> windows::core::Result<bool> {
            let mut queries: Option<ID3D12QueryHeap> = None;
            // SAFETY: plain creation.
            unsafe { device.CreateQueryHeap(&D3D12_QUERY_HEAP_DESC { Type: D3D12_QUERY_HEAP_TYPE_TIMESTAMP, Count: 2, NodeMask: 0 }, &mut queries)? };
            let queries = queries.ok_or_else(windows::core::Error::empty)?;
            put(6 * K64, &[0, 0, 0, 0]);
            rig.run(|list| unsafe {
                list.EndQuery(&queries, D3D12_QUERY_TYPE_TIMESTAMP, 0);
                list.EndQuery(&queries, D3D12_QUERY_TYPE_TIMESTAMP, 1);
                list.ResolveQueryData(&queries, D3D12_QUERY_TYPE_TIMESTAMP, 0, 2, &buffer, 6 * K64 as u64);
            })?;
            // SAFETY: inside the allocation.
            let stamps = unsafe { m.add(6 * K64).cast::<[u64; 2]>().read_volatile() };
            Ok(stamps[0] > 0 && stamps[1] >= stamps[0])
        })();
        lines.push(format!("P6 ResolveQueryData (timestamps) into the opened heap: {}{}", ok_or(&line), debug_said(&device)));
    }
}

fn ok_or(result: &windows::core::Result<bool>) -> String {
    match result {
        Ok(true) => "ok".to_owned(),
        Ok(false) => "WRONG RESULT".to_owned(),
        Err(e) => format!("FAILED {:#010x} ({})", e.code().0, e.message().trim()),
    }
}

/// P7 (R10): what a `SEC_COMMIT` page pool costs in commit charge: 6 GiB
/// at once, and as six sections of 1 GiB; and whether one heap opens over
/// a 6 GiB section.
fn p7(lines: &mut Vec<String>, rig: &mut Rig) {
    let device = rig.device.clone();
    let (before, limit, ..) = commit_mb();
    let reserved = Section::new(6 * GIB, false);
    let (with_reserved, ..) = commit_mb();
    drop(reserved);
    let started = Instant::now();
    let whole = Section::new(6 * GIB, true);
    let took = started.elapsed().as_secs_f64() * 1000.0;
    let (with_whole, ..) = commit_mb();
    let mut line = format!(
        "P7 commit before {before} of {limit} MB (headroom {} MB); a 6 GiB SEC_RESERVE section adds {} MB; a 6 GiB SEC_COMMIT section {} in {took:.1} ms, adds {} MB",
        limit.saturating_sub(before),
        with_reserved as i64 - before as i64,
        match &whole {
            Ok(_) => "made".to_owned(),
            Err(e) => format!("REFUSED (error {e})"),
        },
        with_whole as i64 - before as i64
    );
    if let Ok(section) = &whole {
        let opened = open(&device, From::Section(section.handle as usize));
        let (after_open, ..) = commit_mb();
        line += &format!("; opened as one heap {} (commit then {} MB more, working set {} MB)", hr(&opened), after_open as i64 - with_whole as i64, working_set_mb());
        if let Ok(heap) = &opened {
            let high = place(&device, heap, 5 * GIB as u64, GIB as u64);
            line += &format!("; a buffer at 5 GiB in it {}", hr(&high));
            if let Ok(buffer) = &high {
                line += &format!(", the GPU writes it {}", rig.store(buffer, 0, &[1u8; 4096]).is_ok());
            }
        }
    }
    drop(whole);
    let started = Instant::now();
    let parts: Vec<_> = (0..6).map(|_| Section::new(GIB, true)).collect();
    let took = started.elapsed().as_secs_f64() * 1000.0;
    let (with_parts, ..) = commit_mb();
    let made = parts.iter().filter(|p| p.is_ok()).count();
    line += &format!("; six SEC_COMMIT sections of 1 GiB: {made} made in {took:.1} ms, adding {} MB", with_parts as i64 - before as i64);
    lines.push(line);
}

/// R9 (and limit (c)): what the title's own calls do on pages of an opened
/// part, and when the pages are free again.
fn r9(lines: &mut Vec<String>, rig: &mut Rig) {
    let device = rig.device.clone();
    let Some(memory) = Private::new(16 * K64) else {
        return;
    };
    let at = |page: usize| unsafe { memory.0.add(page * K64) }.cast::<c_void>();
    let Ok(heap) = open(&device, From::Part(at(4) as usize, 8 * K64)) else {
        lines.push("R9: not opened".to_owned());
        return;
    };
    let placed = place(&device, &heap, 0, 8 * K64 as u64).ok();
    let call = |f: &dyn Fn() -> bool| if f() { "ok".to_owned() } else { format!("FAILS (error {})", last_error()) };
    let mut old = 0u32;
    let old = &raw mut old;
    // SAFETY: pages of our own allocation, inside and outside the part.
    let (protect, same_protect, decommit, reset, release, outside) = unsafe {
        (
            call(&|| mem::VirtualProtect(at(5), K64, mem::PAGE_READONLY, old) != 0),
            call(&|| mem::VirtualProtect(at(5), K64, mem::PAGE_READWRITE, old) != 0),
            call(&|| mem::VirtualFree(at(6), K64, mem::MEM_DECOMMIT) != 0),
            call(&|| !mem::VirtualAlloc(at(7), K64, mem::MEM_RESET, mem::PAGE_READWRITE).is_null()),
            call(&|| mem::VirtualFree(memory.0.cast(), 0, mem::MEM_RELEASE) != 0),
            call(&|| mem::VirtualFree(at(13), K64, mem::MEM_DECOMMIT) != 0),
        )
    };
    let released_early = !release.starts_with("FAILS");
    // The heap's last reference goes with the buffer placed in it.
    drop(heap);
    // SAFETY: as above.
    let with_buffer = call(&|| unsafe { mem::VirtualFree(at(8), K64, mem::MEM_DECOMMIT) } != 0);
    drop(placed);
    // SAFETY: as above.
    let after = call(&|| unsafe { mem::VirtualFree(at(9), K64, mem::MEM_DECOMMIT) } != 0);
    lines.push(format!(
        "R9 on pages of an opened part: VirtualProtect read-only {protect}, VirtualProtect read-write (unchanged) {same_protect}, MEM_DECOMMIT {decommit}, \
         MEM_RESET {reset}, MEM_RELEASE of the allocation {release}; MEM_DECOMMIT outside the part {outside}; \
         with the heap released but a buffer in it alive {with_buffer}; with the buffer gone too {after}"
    ));
    if released_early {
        // Released already: the memory is not ours to free again.
        std::mem::forget(memory);
    }
    let _ = rig;
}

/// Limit (b): a reserved buffer's tiles mapped into an opened heap (on
/// the software adapter this removes the device), and the route the
/// design takes instead: tiles of a heap in the card's memory.
fn tile_probe(lines: &mut Vec<String>, rig: &mut Rig) {
    let device = rig.device.clone();
    let reserved = |device: &ID3D12Device| -> windows::core::Result<ID3D12Resource> {
        let mut out: Option<ID3D12Resource> = None;
        // SAFETY: plain creation.
        unsafe { device.CreateReservedResource(&buffer_desc(4 * K64 as u64, D3D12_RESOURCE_FLAG_ALLOW_UNORDERED_ACCESS), D3D12_RESOURCE_STATE_COMMON, None, &mut out)? };
        out.ok_or_else(windows::core::Error::empty)
    };
    let map = |rig: &mut Rig, resource: &ID3D12Resource, heap: &ID3D12Heap| {
        let start = D3D12_TILED_RESOURCE_COORDINATE::default();
        let size = D3D12_TILE_REGION_SIZE { NumTiles: 1, ..Default::default() };
        let (flags, offset, count) = (D3D12_TILE_RANGE_FLAG_NONE, 0u32, 1u32);
        // SAFETY: one region of one tile, one range of one tile.
        unsafe {
            rig.queue.UpdateTileMappings(resource, 1, Some(&start), Some(&size), heap, 1, Some(&flags), Some(&offset), Some(&count), D3D12_TILE_MAPPING_FLAG_NONE);
        }
        rig.run(|_| ())
    };
    // The route first: a heap in the card's memory.
    let ours = (|| -> windows::core::Result<()> {
        let mut heap: Option<ID3D12Heap> = None;
        // SAFETY: plain creation.
        unsafe {
            device.CreateHeap(
                &D3D12_HEAP_DESC {
                    SizeInBytes: 4 * K64 as u64,
                    Properties: D3D12_HEAP_PROPERTIES { Type: D3D12_HEAP_TYPE_DEFAULT, ..Default::default() },
                    Alignment: 0,
                    Flags: D3D12_HEAP_FLAG_ALLOW_ONLY_BUFFERS,
                },
                &mut heap,
            )?
        };
        let heap = heap.ok_or_else(windows::core::Error::empty)?;
        let resource = reserved(&device)?;
        map(rig, &resource, &heap)?;
        rig.store(&resource, 0, &[3u8; 256])
    })();
    lines.push(format!("tiles: a reserved buffer's tile mapped onto a heap in the card's memory and written: {}{}", hr(&ours), debug_said(&device)));
    let Some(memory) = Private::new(4 * K64) else {
        return;
    };
    let Ok(heap) = open(&device, From::Part(memory.0 as usize, 4 * K64)) else {
        return;
    };
    let theirs = reserved(&device).and_then(|resource| map(rig, &resource, &heap));
    lines.push(format!(
        "tiles: a reserved buffer's tile mapped into an opened heap: {}; the device {}{}",
        hr(&theirs),
        rig.removed().map_or("stands".to_owned(), |r| format!("WAS REMOVED ({r})")),
        debug_said(&device)
    ));
}

/// The probes on a device of their own on the default adapter (the card),
/// for `victory inplace-probe`: the first probe's lines, then Stage 0's.
/// `debug` turns the D3D12 debug layer on first (it must be before the
/// device is made).
pub fn run_alone(mb: usize, big: bool, tiles: bool, debug: bool) -> Vec<String> {
    use windows::Win32::Graphics::Direct3D::D3D_FEATURE_LEVEL_12_0;
    use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory2, DXGI_CREATE_FACTORY_FLAGS, IDXGIAdapter1, IDXGIFactory6, DXGI_GPU_PREFERENCE_HIGH_PERFORMANCE};
    let mut lines = Vec::new();
    // SAFETY: plain DXGI and D3D12 creation.
    let device = unsafe {
        if debug {
            let mut layer: Option<ID3D12Debug> = None;
            match D3D12GetDebugInterface(&mut layer) {
                Ok(()) => layer.iter().for_each(|l| l.EnableDebugLayer()),
                Err(e) => lines.push(format!("no debug layer: {e}")),
            }
        }
        let adapter = CreateDXGIFactory2::<IDXGIFactory6>(DXGI_CREATE_FACTORY_FLAGS(0))
            .and_then(|f| f.EnumAdapterByGpuPreference::<IDXGIAdapter1>(0, DXGI_GPU_PREFERENCE_HIGH_PERFORMANCE));
        let mut device: Option<ID3D12Device> = None;
        match &adapter {
            Ok(adapter) => {
                if let Ok(desc) = adapter.GetDesc1() {
                    let name = String::from_utf16_lossy(&desc.Description);
                    lines.push(format!(
                        "adapter: {} ({} MB of its own memory)",
                        name.trim_end_matches('\0'),
                        desc.DedicatedVideoMemory >> 20
                    ));
                }
                let _ = D3D12CreateDevice(adapter, D3D_FEATURE_LEVEL_12_0, &mut device);
            }
            Err(_) => {
                let _ = D3D12CreateDevice(None, D3D_FEATURE_LEVEL_12_0, &mut device);
            }
        }
        device
    };
    let Some(device) = device else {
        lines.push("no D3D12 device".to_owned());
        return lines;
    };
    let started = Instant::now();
    let probe = crate::inplace::probe_with(&device, &|| None, mb, big, tiles);
    lines.extend(probe.lines);
    lines.push(format!("took {:.0} ms", started.elapsed().as_secs_f64() * 1000.0));
    lines
}
