# Baseline on the owner's PC

First measurement of this fork on the machine it is being tuned for. Later
work (frame rate, ray-traced shadows) is compared against this file.

## Machine

| Part | Value |
|------|-------|
| CPU | AMD Ryzen 7 5700X (8 cores, AVX2) |
| GPU | NVIDIA GeForce RTX 5060 Ti 16 GB (desktop Blackwell, 32-lane waves) |
| GPU driver | 621.40 |
| RAM | 32 GB |
| OS | Windows 11 Pro 10.0.26300 |
| Toolchain | Rust 1.98.1 (pinned), Visual Studio Build Tools 2022 |
| Title | own copy, 1.0.2.1, Series X profile |

The README's own test machine is an RTX 4070; Blackwell is untested there.
Closest outside reports on the same GPU family:
fxceless/victory-gta6#7 (RTX 5060 Ti 8 GB: stable 15 min, up to 2.6 fps),
Marvisak/VIctory#821 (RTX 5060 Ti: host-GPU shader tests pass),
Marvisak/VIctory#1586 (RTX 5090: 1.1-1.7 fps, then `DXGI_ERROR_DEVICE_HUNG`).

## Build (2026-11-24, `main` at `b4886e2`)

| Step | Result |
|------|--------|
| `cargo build --release -p victory` | ok |
| `cargo build --profile diag -p victory --features diagnostics` | ok |
| `cargo test --workspace --release --no-fail-fast` | **496 passed, 0 failed, 7 ignored** |

One flaky test: `victory-gpu` `frame::tests::waits_end_on_ticks_and_count_frames`
failed once (`[3, 4, 6, 7]`, a skipped 5 ms tick) while the diag build was
compiling in parallel, and passed on an idle machine. It depends on wall-clock
timing, so expect it to fail under heavy CPU load.

## In-game run

**Pending: the game files on this PC are still being assembled.**

One early attempt (diag build, `VICTORY_D3D12_DRED=1`, scripted Enter presses)
stopped within a second of start, before any frame:

```
WARN victory_hle::modules::dstorage: dstorage: opening a file failed path="G:\shaders.rpf"
     error=The process cannot access the file because it is being used by another process. (os error 32)
VIctory: unhandled exception 0x80000003 at victory-image-…+0x1ce6e13
```

The title's own breakpoint (`0x80000003`) right after a DirectStorage open of
`shaders.rpf` failed, so this is most likely incomplete or locked game data
rather than a GPU problem. To be repeated once the files are in place.

To fill in after that run:

| Measure | Value |
|---------|-------|
| Time to first world frame | |
| World fps (median per minute) | |
| ms/frame split (preparing / GPU / copying back) | |
| Draws per frame, dropped without a pipeline | |
| Shaders that do not translate | |
| Host pipelines refused | |
| Crashes, device loss, TDR | |
| Peak process memory, GPU memory | |
| Blackwell-specific issues | |

## How to repeat

```powershell
copy victory.example.toml victory.toml   # game_dir, cache_dir, data_dir outside the repo
cargo build --profile diag -p victory --features diagnostics
$env:VICTORY_D3D12_DRED = '1'
$env:VICTORY_KEYS = '10:Enter,20:Enter,30:Enter,...,300:Enter'
target\diag\victory.exe run <game dir>\game_scarlett_master.exe --execute 2> run.log
```

The title's executable must sit in the game folder: its DLLs are looked up
next to it. Frame statistics are the `… fps, … ms/frame (…)` lines in the log.
