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
