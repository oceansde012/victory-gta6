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
