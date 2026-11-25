# PC runs: CPU/GPU overlap (round 2)

Build: release (or diagnostics) build of `claude/project-thread-4f4qka`.
Same scene and length as round 1, shader cache warm. For each run, from
the log's frame-rate lines: ms per frame (median and worst), "the card's
clock per frame" (card busy %, "the card idle behind us", "the submitting
thread stood waiting for the card"), card power; and whether the picture
changed (flicker, stale or missing geometry) or the run ended early.

The overlap of CPU preparation and card work exists, off by default:
`VICTORY_LATE_WAIT` sends submissions ahead of the wait for the card,
`VICTORY_RELAY` lands their results on a thread of its own as the card
finishes (fxceless README: 240-350 ms a frame with them, 420-450 without).

| Run | Environment |
|---|---|
| A | (defaults; the reference) |
| B | `VICTORY_LATE_WAIT=3` |
| C | `VICTORY_RELAY=2` |
| D | `VICTORY_LATE_WAIT=3 VICTORY_WIDE_BACK=1 VICTORY_CARD_BUFFERS_MIN_MB=1 VICTORY_CARD_BUFFERS_MB=1024` |
| E | `VICTORY_RELAY=2 VICTORY_WIDE_BACK=1 VICTORY_CARD_BUFFERS_MIN_MB=1 VICTORY_CARD_BUFFERS_MB=1024` |
| F | best of B-E plus `VICTORY_CARD_FETCH_INFLIGHT=1 VICTORY_WIDE_TEXTURES=1` |

If C or E is stable and faster with the same picture, those switches
become the defaults in a follow-up commit. If the card is still idle much
of the frame in the best run, take a PIX timing capture there
(`VICTORY_PIX=<dir>` with PIX installed; a file named `capture` in that
folder about 60 s into the world saves the next frames) and send the
`.wpix`.

Stability tip from ztrx's README: error 1455 in `crashes.log` means
Windows commit memory ran out; raise the paging file.

## Round 3: copy-back (2026-11-25)

On the corrected base (PR #6 `fbde9d1`) the default frame is 291 ms, of
which copy-back is 173 ms (the card itself 88 ms). Copy-back is the CPU
waiting for each submission to finish, fetching what shaders wrote in
the card's memory and putting it into title memory before the title may
go on. Ways around it in this tree, all off by default:

- `VICTORY_RELAY=2`: a thread lands flights as the card finishes them, so
  the submitting thread does not wait for the fetch.
- `VICTORY_CARD_FETCH_INFLIGHT=1` (with the relay): the fetch is recorded
  into the flight itself, no extra wait or list.
- `VICTORY_EARLY_SIGNAL=1` (with the relay): flights with nothing to copy
  back tell the title at once instead of waiting for later copy-backs.
- `VICTORY_INPLACE=4` (fxceless #772, residents): big title buffers live
  in the card's memory and the card merges shader writes back itself, in
  the same submission: no fetch, no CPU merge.

| Run | Environment |
|---|---|
| G | `VICTORY_RELAY=2 VICTORY_CARD_FETCH_INFLIGHT=1 VICTORY_EARLY_SIGNAL=1` |
| H | `VICTORY_INPLACE=4` |
| I | `VICTORY_INPLACE=4 VICTORY_RELAY=2 VICTORY_EARLY_SIGNAL=1` |
| J | I plus `VICTORY_CARD_TIME=1` (card time split: uploads, title lists, merge; for the log only) |

For each: ms per frame, the copy-back share (the log's "copied back" /
relay lines), "the card's clock per frame", the residents' line with
`VICTORY_INPLACE=4` (MB uploaded and merged per frame), picture changes,
and whether the run ended early. `VICTORY_INPLACE=4` is new upstream: if
it crashes, note the last lines of the log and `crashes.log`.

## Round 4: lighting vs frame time (2026-11-25)

03e328f against 1644f2b by default: 1.85 fps / 540 ms (card 284 ms)
against 3.5 fps / 283 ms (card 73 ms); 5 refused pipelines instead of 15;
and shadowed places are no longer black but lit with a blue fill. The
suspect is fxceless #799's loose-LDS fallback (on by default): console
wave64 programs that read and write LDS inside one lane-branch region
were refused by a 32-lane host (every dispatch dropped) and now run,
approximately, as two host waves. These look like the lighting computes.

`VICTORY_WAVE_LDS_LOOSE=0` turns it off: the switch between picture
(lighting, default) and frame time. It changes the translation, so the
shader cache translates those programs again on the first run with it.

| Run | Build | Environment | Expect if #799 is the cause |
|---|---|---|---|
| K | 03e328f (or later) | `VICTORY_WAVE_LDS_LOOSE=0` | ~3.5 fps, card ~73 ms, shadows black again |
| L | 03e328f, diagnostics build | `VICTORY_GPU_TIMING=1`, 20 s in the world | per-pipeline card times: which programs take the ~210 ms |

L's log names the slowest pipelines by hash (the lines from
`copystat::command_took`); with those, the expensive programs can be run
at a lower rate or resolution, or tuned, instead of all or nothing.

K result: about 2.1 fps, so #799 is not the cause. The bisect of the six
code commits between `1644f2b` and `03e328f` is in
`docs/upstream-integration.md`. `03e328f` is the picture baseline.

## Round 5: hoisted indirect clamps, walls, Series S (2026-11-25)

Build: PR #7 head (after `315577a`), at the same spot as earlier runs.

| Run | Environment | Look for |
|---|---|---|
| M | defaults | walls (candidate fix `b204a79`), fps vs `03e328f` |
| N | `VICTORY_HOIST_CLAMPS=1` | GPU ms a frame (L: 232, indirect 113), flicker of mesh objects, fps |
| O | `VICTORY_HOIST_CLAMPS=1`, `VICTORY_GPU_TIMING=1` (diagnostics build) | does indirect drop from 113 ms? |
| P | `--console series-s` | fps and picture (klbf-lab and victory-perf measure this way) |

In M, search the log for "no free dynamic descriptor slots" and dropped
draws near walls that vanish.
