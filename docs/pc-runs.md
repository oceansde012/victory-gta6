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
