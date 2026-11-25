# Performance picks, round 1 (2026-11-25)

Branch `claude/project-thread-4f4qka`, on top of PR #6
(`claude/project-thread-2l2dcf`). Open upstream PRs aimed at frame time,
GPU memory and GPU stability, each one commit (upstream author kept), so
any of them can be reverted alone if the PC run gets worse.

Checked here: `cargo clippy --workspace --all-targets --all-features
--target x86_64-pc-windows-msvc -- -D warnings`, `cargo test -p
victory-shader` (189 passed). Nothing was run against the title.

## Taken

| Upstream PR | What | Why |
|---|---|---|
| fxceless #705 | Join adjacent title-memory copies by default | fewer copy calls per frame |
| fxceless #709 | `Counting::realloc` race; pipeline wait budget monotonic across frames | stability, no frame-long PSO waits |
| fxceless #715 | No blending on integer render targets | invalid PSO state (GPU stability) |
| fxceless #747 | Failed `card_wrote` poisoning, 2D-array/MinLOD SRV clamp, overflow | GPU stability |
| fxceless #742 | Evict CBV-backed fresh tables on replay across cut lists | stale constants |
| fxceless #743 | THROUGH alias cache keyed by (placed, source) | stale textures |
| fxceless #691 | Windows outside buffers default to the 1 MiB grid | fewer, larger copies |
| fxceless #695 | `VICTORY_WAVE_WIDE_ONLY` (opt-in): wave-wide form for named programs (water volumetrics on NVIDIA) | Wave64 test switch |
| fxceless #733 | Due texture queue not fully sorted each pass (hand-merged test section) | CPU per frame |
| fxceless #685 | O(log R) BlockRuns guard cache (kept our `switch::on` for DRED) | CPU per frame |
| fxceless #740 | Freshly decoded R8 textures uploaded before first GPU use (hand-merged) | missing/black textures |
| Marvisak #2143 | A program's constant table starts after its instructions (hand-merged) | GBs of NVIDIA video memory |
| fxceless #717 (+#687) | F3 overlay of where the frame goes, `overlay.log`; checkerboard null views, PIX captures | profiling on the PC |
| ours | `VICTORY_HEAP_TEXTURE_MS` / `VICTORY_HEAP_TEXTURE_MB` (defaults 250 / 192) | #717 measured the heap texture pass as the biggest part of slow frames on an RTX 5060 Ti |

## Not taken (yet)

Conflict with the fork's GPU rewrite (Marvisak's GPU crate differs from
fxceless'); worth doing one by one after a PC run shows where time goes:

- Marvisak #2111 (one pipeline-wait budget for all recording threads, 5
  conflicts), #2136 (no host copy nothing wrote, 11), #2147 (write ranges
  armed as page runs, 3), #2129 (pipeline library mapped, 14), #2102
  (FxHash, 4), #2099 (descriptor view table growth, 15), #2088 (256 KiB
  copy windows, 6), #2112 / #2120 / #2130 (in-place / upload-heap title
  memory, opt-in, 17+), #2146 (mip drop, high-performance adapter, 16),
  #2118 (BVH builder whole or not at all, 5), #2150 (one instruction per
  value: adds a pass that assumes Marvisak's page table).
- Marvisak #2117, #2126, #2085, #2105 (wave size declarations): conflict
  with the fork's lane-mask work.
- fxceless #688 (64-thread CARD_COMPARE, COPY-queue readback, 13
  conflicts), #676 (full-card queue starvation, 6), #723 (opt-in card
  reserve).

## Lighting

No open upstream PR makes the world lighting / RT shadows work yet; the
tracker (vuyp.github.io/gta6-pc-race, 2026-11-24) lists every project as
unlit or dark in the world. Marvisak #2118 / #2098 touch the BVH path and
are the next candidates once the fps work is measured.
