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

## In-game runs (2026-11-25)

Same scene each time: boot with scripted Enter presses every 10 s, through
the opening into the start of the first gameplay scene (the prologue yard),
camera at rest. Diagnostics build (`--profile diag --features diagnostics`,
release code generation; needed for `VICTORY_KEYS`), `VICTORY_D3D12_DRED=1`,
no speed switches unless listed. The title renders at **3840x2160**.
One run per row, so treat differences under about 10 % as noise.

Steady state is the median of the `fps` log lines over the last 3 minutes of
each run. "Preparing" is CPU work before submission (copies, textures);
"copy-back" is reading shader-written memory back for the title.

| Run | Commit | What | Length | fps | ms/frame | preparing (copies, textures) | GPU | copy-back | draws / dropped | Stable |
|-----|--------|------|--------|-----|----------|------------------------------|-----|-----------|-----------------|--------|
| A | `55293f6` | main + upstream sync (#4) | 15 min, cold cache | 1.9 | 532 | 158 (59, 17) | 245 | 78 | 4974 / 355 | yes |
| B | `fd3bcad` | #5: A + 7 open fxceless PRs | 15 min | **2.2** | **456** | 132 (55, 4) | 240 | 72 | 4790 / 366 | yes |
| C1 | `405b488` | #6 stage 1: #5 + fxceless main + 108 Marvisak commits | 8 min | 2.0 | 512 | 156 (64, 6) | 254 | 86 | 5019 / 386 | yes |
| C2 | `af55858` | #6 head: C1 + 22 open fxceless PRs + descriptor fix | 8 min, pipelines still compiling | 1.4 | 723 | 225 (60, 21) | 223 | 152 | 3081 / 1234 | yes |
| C2b | `af55858` | same, cache warm | 8 min | 2.1 | 486 | 132 (48, 5) | 251 | 88 | 4999 / 381 | yes |
| C3 | `af55858` | `VICTORY_INDIRECT_CLAMP=0` | crashed at 196 s | 0.45 | 11052 | 330 | **10294** | 127 | 2914 / 1406 | **no** |
| D-A | `f651a58` | #7: #6 + F3 overlay + heap texture budgets (defaults) | 8 min | 1.9 | 519 | 160 (66, 4) | 250 | 84 | 5016 / 370 | yes |
| D-B | `f651a58` | `VICTORY_HEAP_TEXTURE_MS=60` | 8 min | 2.1 | 474 | 141 (55, 11) | 243 | 72 | 4946 / 366 | yes |
| D-C | `f651a58` | `VICTORY_HEAP_TEXTURE_MS=60 VICTORY_UNWATCHED_FRAMES=4` | 8 min | 2.1 | 467 | 137 (54, 4) | 237 | 71 | 4874 / 368 | yes |
| D-D | `f651a58` | `VICTORY_WAVE_WIDE_ONLY=02384a60,2cc4c197,2da4beb3,369cbe90,5dc3b259,6f225b64` | 8 min | 2.1 | 468 | 138 (55, 9) | 241 | 70 | 4886 / 364 | yes |

Frame-time spread from the #7 overlay (`overlay.log`, last 240 frames at the
end of each run):

| Run | median | p95 | worst | video memory | process memory |
|-----|--------|-----|-------|--------------|----------------|
| D-A | 483 ms | 550 ms | 601 ms | 9.3 of 14.9 GB (+4.2 GB shared) | 22.6 GB |
| D-B | 465 ms | 513 ms | 591 ms | 9.4 GB (+4.3) | 22.6 GB |
| D-C | 461 ms | 488 ms | 511 ms | 9.6 GB (+4.3) | 22.8 GB |
| D-D | 460 ms | 484 ms | 498 ms | 9.4 GB (+4.3) | 22.8 GB |

### What this says

- **Every warm build lands at 2.0-2.2 fps, 450-520 ms per frame.** #5, #6 and
  #7 are within noise of each other; #6 head (`af55858`) is not worse than its
  stage 1 (`405b488`) once its pipelines are built (C2b vs C1), so no bisect of
  the 22 merges is needed. C2's 1.4 fps was a cold pipeline cache (1300-1600
  pipelines still being made at the end).
- **The frame is serialized, not GPU-bound.** #7's overlay: "GPU 232 ms, card
  busy 50 %, idle behind us 213 ms"; "submitting thread busy 443 of the
  frame's 464 ms"; "waiting for the card 237 ms; sent ahead 0". `nvidia-smi`
  samples show 35 W board power at 50-65 % utilisation on a 180 W card. The
  submitting thread prepares, submits, then waits for the card, and the card
  then waits for the next submission. Overlapping the two (the existing
  `VICTORY_LATE_WAIT` / `VICTORY_RELAY` switches) is the first thing to try.
- **`VICTORY_INDIRECT_CLAMP=0` is unsafe here**: GPU time went to 10 s per
  frame and the title crashed with an access violation at
  `+0x2215eca`, the address of Marvisak/VIctory#1464. Keep the clamp on.
- **Heap texture budget 60 ms** (D-B, D-C) changes little in steady state
  (texture time is already 4-11 ms per frame once warm) and tightens the
  tail (p95 488-513 vs 550 ms). Its texture queue at the end: 2590 waiting,
  oldest due 908-1042 frames, against 2449 / 700 frames with the default, so
  late textures (pop-in) are somewhat more likely; not visible in the
  captured frames.
- **`VICTORY_WAVE_WIDE_ONLY`** cut refused pipelines from 15 to 13 (two of
  the six named programs now build). In its frame 2048 capture the yard seen
  through the fence shows bright grainy speckles not present in the other
  runs' captures of the same scene; this needs a side-by-side check at a
  fixed frame before the switch is used.
- Same problems in every run: 7 shaders do not translate, 13-15 compute
  pipelines refused (`E_INVALIDARG`, 64-lane programs on a 32-lane card),
  ~360 draws dropped per frame (354 of them the ray-tracing structure
  builder, which is off), everything in shadow black, about 23 GB process
  memory. No device loss or TDR in any run except C3.
- Warm-up: with a warm cache the world reaches steady fps about 4 minutes
  after launch; on a cold cache (A) it took 12 minutes.

### Earlier attempt (2026-11-24)

Before the game files were complete, one run stopped within a second at the
title's breakpoint (`0x80000003`) after `shaders.rpf` failed to open (locked
by the download client). Not a VIctory problem.

### Second opinion

The table was also given to ChatGPT (Codex CLI). It agreed that single runs
cannot separate differences under ~10 %, that the overlay points at
serialized CPU submission and GPU waits rather than a saturated GPU, and
suggested a PIX or Nsight Systems timeline of the queue and fences next.

## How to repeat

```powershell
copy victory.example.toml victory.toml   # game_dir, cache_dir, data_dir outside the repo
cargo build --profile diag -p victory --features diagnostics
$env:VICTORY_D3D12_DRED = '1'
$env:VICTORY_KEYS = '10:Enter,20:Enter,30:Enter,...,300:Enter'
target\diag\victory.exe run <game dir>\game_scarlett_master.exe --execute 2> run.log
python tools/perf/analyze_run.py run.log   # per-minute medians, last 3 minutes, problem counts
```

The title's executable must sit in the game folder: its DLLs are looked up
next to it. Frame statistics are the `… fps, … ms/frame (…)` lines in the log.
