# Test runs on the RTX 5060 Ti + Ryzen 7 5700X (DexNom base)

Baseline 7933b42: 3.1 fps, copy-back ~225 ms/frame, 2-12 s freezes while
streaming, 27 GB RAM.

Code in this branch on top of it:
- dstorage workers = physical cores (8 instead of 16 on a 5700X; fxceless
  #1044 measured copy-back 5.7 -> 2.4-3.5 ms/MB on an 8c/16t machine).
  Override: `VICTORY_DSTORAGE_WORKERS=N`.

All switches below already exist in DexNom and are off by default. Run each
set at the same spot, note fps, the `copying back` ms of the frame-rate line
and the freezes.

## Run 1: code only
No extra variables.

## Run 2: VItoriaBr preset (tested by that fork on the same GPU/CPU)
```
set VICTORY_TEXTURE_NO_WAIT=1
set VICTORY_CARD_BUFFERS_MB=6144
set VICTORY_WIDE_REFRESH=1
set VICTORY_WIDE_MERGE=1
set VICTORY_JOIN_COPIES=1
```
- WIDE_MERGE: card-written pieces merged into title memory on 8 threads
  (that merge is part of copy-back).
- CARD_BUFFERS_MB=6144: more title buffers compared on the card instead of
  the CPU (default 1024; 16 GB card).
- TEXTURE_NO_WAIT: texture refresh does not wait 1.3-3.7 s on the runtime's
  lock (the streaming freezes).
- WIDE_REFRESH / JOIN_COPIES: the "copies" part of CPU prep.

## Run 3: run 2 + small buffers on the card
```
set VICTORY_CARD_BUFFERS_MIN_MB=1
```
The "shader writes looked for on the CPU" log line says how many MB this
would move to the card.

Keep `VICTORY_INPLACE` unset (no gain on our earlier build).

## Result of 0540763 (runs 1-3)
Copy-back ~210 ms, 3.7-3.8 fps; preset 2 cuts freezes 8-9 -> 5. CPU side:
26 MB in 279 ranges, 0 MB in copies of 1 MiB or more. Those 26 MB are
already joined per range and compared on 8 threads (`hand_back` /
`relay_hand_back`): a few ms, not 210.

Suspect: `Host::relay_card_back`. With the relay (default) the landing
thread fetches the card copies' changed pieces with a list on the **main
queue** and waits for it (`WaitForSingleObject(INFINITE)`). The flights sent
ahead are already in that queue, so the fetch waits for their GPU work too,
and that wait is counted in "copying back". The log line
`the relay per frame ... waited X ms for fetches of card copies' pieces,
and was Y ms landing in all` gives the split.

## Run 4: card copies off (no fetch at all)
Preset 2, plus:
```
set VICTORY_CARD_BUFFERS=0
```
Card-copy compares move to the CPU (more RAM/CPU, no fetch wait).

## Run 5: relay off
Preset 2, plus:
```
set VICTORY_RELAY=0
```
Fetch runs inside the submission, with no flights queued ahead of it.

If run 4 or 5 removes most of the 210 ms, the fix is a fetch on its own
copy queue (needs care with `QUEUE_ORDER`: sends queued before the fetch
must have run before it reads).

## Confirmed (relay line, 0540763 runs 1-3)
Fetch wait 157-172 ms of 202-221 ms landing. 

## Run 6: fetch on its own copy queue (code in this commit, opt-in)
Preset 2, plus:
```
set VICTORY_FETCH_QUEUE=1
```
Log at start: "card copies' pieces are fetched on a copy queue of their
own". Watch: the relay line's fetch ms (expect a few ms), copying back,
fps, and the picture (walls, shadows, water) against run 2: the fetch now
follows only the sends of flights already landed, so a wrong merge would
show as flicker or stale geometry.

## Added on top of 91078ff (fresh fxceless PRs, victory-unified)
- fxceless #955: `VICTORY_PIPELINE_WORKERS`, `VICTORY_PIPELINE_NORMAL`.
- fxceless #959 (most of it): XMA/audio fixes against the 7-11 min pool
  crash (DexNom's audio.rs did not have them), wait for a submission ends
  on device loss / GPU stall, pipeline library never saved over a bad one,
  `VICTORY_NO_WATCHDOG=1` (opt-in), `VICTORY_INPLACE_POOL=1` (opt-in),
  **upload buffers pooled by size class (on by default; `VICTORY_UPLOAD_POOL=0`
  = old behaviour)**: in their run the driver held 5.4 GB of
  write-combined memory for one-off upload buffers.
- victory-unified fb3d60c: `VICTORY_AMBIENT=<level>` constant indirect
  light (Belobug's), off unless set.
- Not taken: fxceless #980 (live RT + in-place VRAM, 25 commits, 23
  conflicting files on DexNom), #979/#965/#964 (files DexNom lacks or
  rewrote).

## Run 7: new default (upload pool on)
Preset 2 + `VICTORY_FETCH_QUEUE=1`. Compare RAM, "RAM" in the HUD, fps
and freezes with run 6.

## Run 8: upload pool off
Run 7 + `set VICTORY_UPLOAD_POOL=0`.

## Run 9: constant ambient
Run 7 + `set VICTORY_AMBIENT=0.6`. Screenshots indoors and in shade
against run 7: brighter shade, but no lighting from the title's probes.

## A/B of two builds (2026-11-25 evening)
Two builds, same spot (prison by the window, warm cache = second launch of
each build), same 1920x1080 window, 3 minutes each after the world is in.

- **A** = branch `claude/project-thread-go0fy1`: this DexNom build (0d3a650)
  plus fxceless #1129 (changed_runs 64/8 bytes), VItoriaBr #34 (divergent
  barriers on 32-lane GPUs), the small parts of fxceless #1117 (heap start
  kept, root constants on the stack), Marvisak #2783 (wave64 mask set by
  halves), Marvisak #2706 (pipeline library per translation), thin LTO +
  one codegen unit in release.
- **B** = branch `claude/vbr-ab-5060ti`: VItoriaBr main 69c665c (fxceless
  085ee7c + their 5060 Ti work) plus our NVIDIA picks (Marvisak #2634,
  #2007, #2143, fxceless #1002, #1044) and fxceless #1129, #1128, #1122,
  Marvisak #2783.

Cache note: A's first launch starts with an empty pipeline library (it now
lives in a folder per translation, the old one stays in the cache root), so
launch A once, quit after the world is in, then measure on the second
launch. B keeps its library in the cache root, A's folders are not touched
by it.

### What to record for every run
- fps, frame time and 1% low: PresentMon (portable exe, no install) on the
  victory process for the 3 minutes, `--output_file run-XX.csv`; avg fps =
  1000 / mean(MsBetweenPresents), 1% low = 1000 / p99(MsBetweenPresents).
- RAM and CPU: `typeperf "\Process(victory)\Working Set - Private"
  "\Processor(_Total)\% Processor Time" -si 1 -sc 180 -o run-XX-cpu.csv`.
- VRAM and GPU load: `nvidia-smi --query-gpu=utilization.gpu,memory.used
  --format=csv -l 1 > run-XX-gpu.csv`.
- the layer's own frame line (copying back, textures, relay) from the log,
  and the count of "no free dynamic descriptor slots" / device removed.
- one screenshot at the window and one in shade, against 7933b42 (walls,
  hair, glass, water, shadows).

## Run 10: A, run-7 settings
Preset 2 + `VICTORY_FETCH_QUEUE=1`.

## Run 11: B, VItoriaBr's own preset
```
set VICTORY_RELAY=2
set VICTORY_QUEUE2=1
set VICTORY_ASYNC_SUBMIT=1
set VICTORY_COPY_PLAIN=1
set VICTORY_INDIRECT_GATHER=1
set VICTORY_TABLE_BUFFERS=1
set VICTORY_TEXTURE_BACK=1
set VICTORY_TEXTURE_NO_WAIT=1
set VICTORY_CARD_BUFFERS_MB=2048
set VICTORY_TEXTURE_BUDGET_MB=10240
set VICTORY_PAGE_WRITES=slots
set VICTORY_AMBIENT=0.6
set VICTORY_UNWATCHED_FRAMES=4
set VICTORY_WIDE_REFRESH=1
set VICTORY_WIDE_MERGE=1
set VICTORY_JOIN_COPIES=1
set VICTORY_DISPLAY_MODE=1920x1080
```
VItoriaBr reports 5.8 fps at 3840x2160 and copying back ~65-78 ms with
this on the same GPU and CPU (their numbers, not ours).

## Run 12: B without ambient
Run 11 with `set VICTORY_AMBIENT=0`: the picture against 7933b42 without
the constant light, to see what B renders by itself.

## Run 13: B, RAM check
Run 11 with `set VICTORY_CARD_BUFFERS_MB=1024`: our RAM is at the limit
(32 GB, 8 GB pagefile); watch the private working set and freezes.
