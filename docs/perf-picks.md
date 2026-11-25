# Performance picks, round 1 (2026-11-25)

Branch `claude/project-thread-4f4qka`, rebased onto PR #6 at `fbde9d1`
(fxceless main `a26859f` with the 8dd30ab GPU work restored, plus
fxceless #785, #786, #783). Open upstream PRs for frame time, GPU memory
and GPU stability, one commit each (upstream author kept), so any of them
can be reverted alone.

Checked here: `cargo clippy --workspace --all-targets --all-features
--target x86_64-pc-windows-msvc -- -D warnings`, `cargo test -p
victory-shader`. The full Windows suite runs in CI.

## Taken

| Upstream PR | What | Why |
|---|---|---|
| fxceless #705 | Join adjacent title-memory copies by default | fewer copy calls per frame |
| fxceless #709 | `Counting::realloc` race; pipeline wait budget monotonic across frames | stability |
| fxceless #715 | No blending on integer render targets | invalid PSO state |
| fxceless #691 | Windows outside buffers default to the 1 MiB grid | fewer, larger copies |
| fxceless #742 | Evict CBV-backed fresh tables on replay across cut lists (hand-merged) | stale constants |
| fxceless #743 | THROUGH alias cache keyed by (placed, source) | stale textures |
| fxceless #768 | Texture checks of a submission run several at a time (`VICTORY_WIDE_TEXTURES=1`, opt-in) | CPU per submission |
| Marvisak #2143 | A program's constant table starts after its instructions | GBs of NVIDIA video memory |
| ours | `VICTORY_TEXTURE_STAGE_MS` default 250 -> 60 | PC run: smoother p95, no late textures |

## Dropped in the rebase

- fxceless #747: already in fxceless main.
- fxceless #695 `VICTORY_WAVE_WIDE_ONLY`: speckles the water on the PC.
- fxceless #717 F3 overlay, #733, #685, #740: rewritten code under them in
  fxceless `a26859f` (stats lines, texture queue, write watch, levels on
  demand); several conflicts each for small gains. The log's frame-rate
  lines (`the card's clock per frame`, late flights) give what the PC runs
  need. The overlay's own idea (per-frame split on screen) can come back
  as a port later.
- Our `VICTORY_HEAP_TEXTURE_MS/MB`: fxceless now has the same switches
  (`VICTORY_TEXTURE_STAGE_MS/MB`, with a catch-up for backlogs).

## Not taken (yet)

Conflict with the fork's GPU code; one by one after PC runs show where
time goes: Marvisak #2111, #2136, #2147 (CPU/GPU sync), #2129 (pipeline
library mapped), #2102, #2099, #2088, #2112 / #2120 / #2130 (title memory
in place / upload heaps), #2146, #2118, #2150; wave size declarations
#2117, #2126, #2085, #2105; fxceless #688, #676, #723. New since: Marvisak
#2191 (`image_bvh` as a miss so the lighting computes run), #2162 (depth
twins share scratch, ~280 MB), #2192 (translation and pipeline creation as
a dependency graph).

## PC runs

Round 1 (old base): PR #5, #6, #7 all 2.0-2.2 fps (450-520 ms per frame).
The card is about half busy (~35 W): the CPU prepares a frame, then waits
for the card, and the card waits for the next frame.
`VICTORY_WAVE_WIDE_ONLY` speckles water; `VICTORY_INDIRECT_CLAMP=0`
crashes. Round 2: `docs/pc-runs.md`.
