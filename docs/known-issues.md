# Known rendering issues

| Since | Build | Issue | Notes |
| --- | --- | --- | --- |
| 2026-11-25 | `03e328f` (PR #7) | Wall textures are see-through (prison, by the window, frames 2300-2500) | Owner: 03e328f otherwise looks much better than `1644f2b` (no black hair pixels, clean glass, no pink spots or garbage blocks). Not urgent. Watch upstream PRs about alpha test/blend, depth and wall/opacity. | Candidate fix: `b204a79` (Marvisak `c0228d93`, mesh primitive indices masked to 9 bits). Our base culls a primitive whose index is past the vertex count; with the 10-bit mask a set edge flag (bits 9/19/29) made indices >= 512, so those triangles were culled, which would leave holes. ZSlayerHQ/victory-world-runtime reports its walls fix on an RTX 5090 with this mask plus a multi-view guard (ours has that guard from fxceless `8dd30ab`). To confirm on the PC. |
