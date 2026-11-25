# GPU time by pipeline on 03e328f (VICTORY_GPU_TIMING)

Run R9-L, 2026-11-25. Diag build of 03e328f (PR #7 line), `VICTORY_GPU_TIMING=1`, everything else default. Ryzen 7 5700X, RTX 5060 Ti 16 GB, title renders at 3840x2160. Scripted Enter every 10 s, 4 min cap, warm shader cache. Numbers are the last timing report in the log (11:30:18 UTC, in the first gameplay scene).

The card's time for our commands: **231.7 ms a frame**. By first word: indirect 112.6 ms in 5150, dispatch 101.2 ms in 3863, mesh 7.9 ms in 382, indexed 5.6 ms in 116, alias 2.4 ms in 78, draw 1.9 ms in 304, copy 0.2 ms in 7, texture 0.0 ms in 17, read 0.0 ms in 2.

Timing itself slows the run (1.3 fps vs 1.85 without it), so use these as shares, not as absolute frame cost.

## Top 40 by time per frame

| # | ms/frame | commands/frame | longest ms | kind | shaders |
|---|---|---|---|---|---|
| 1 | 13.44 | 1.0 | 13.54 | indirect | compute f22a76fcb967ee59 |
| 2 | 9.36 | 2.0 | 9.96 | dispatch | compute d474a0f15cd0f545 |
| 3 | 8.82 | 200.0 | 0.15 | indirect | mesh ef5409053e14aa82, pixel 94d281151ca58b4f |
| 4 | 8.68 | 1.0 | 8.81 | dispatch | compute 5cf00dc39042edaf |
| 5 | 7.30 | 49.0 | 3.88 | dispatch | compute 4a05640ec16718ad |
| 6 | 7.15 | 360.0 | 0.07 | indirect | mesh db875f50a166bd91 |
| 7 | 6.70 | 39.0 | 7.81 | dispatch | compute 5b6eed17c131f881 |
| 8 | 6.23 | 5.0 | 10.04 | dispatch | compute 006d583a371f22ec |
| 9 | 5.84 | 369.0 | 0.02 | indirect | mesh 6016f52cd61f30f6 |
| 10 | 5.49 | 2.0 | 5.54 | indirect | compute 6d7fe7745ab9c6d8 |
| 11 | 5.45 | 32.0 | 9.64 | dispatch | compute 098fc28e44347a00 |
| 12 | 4.21 | 1.0 | 4.42 | dispatch | compute 14264103296aa43a |
| 13 | 4.04 | 151.0 | 0.11 | indirect | mesh b7a81f5b96f4ca93, pixel 43e24028d4a1faef |
| 14 | 3.67 | 1.0 | 3.92 | dispatch | compute cb6adcc6da2613ff |
| 15 | 3.28 | 1.0 | 3.37 | indexed | vertex a67d0bcde30cead3, pixel 5ad3a82b1c3cc63a |
| 16 | 2.91 | 1.0 | 2.98 | dispatch | compute 0a5ecfee6b79e13b |
| 17 | 2.58 | 200.0 | 0.02 | indirect | mesh cf9ea039f3cdb04d |
| 18 | 2.55 | 69.3 | 0.10 | indirect | mesh df4b2c7798c14c90, pixel d9fcb8a4425fc59b |
| 19 | 2.37 | 78.1 | 0.12 | alias copy |  |
| 20 | 2.08 | 1.0 | 2.15 | dispatch | compute 2f03a2df10503cef |
| 21 | 2.06 | 1.0 | 2.43 | indexed | vertex 7f4b62129572a14d, pixel 5ad3a82b1c3cc63a |
| 22 | 1.96 | 1.0 | 2.01 | dispatch | compute 7c90a988167650a1 |
| 23 | 1.87 | 79.0 | 0.52 | dispatch | compute 4115dd63f99a09d7 |
| 24 | 1.86 | 113.0 | 0.03 | indirect | mesh 30332cf87c4e0eae |
| 25 | 1.79 | 1.0 | 1.84 | dispatch | compute a7eeed0646be1ab7 |
| 26 | 1.77 | 53.0 | 0.06 | indirect | mesh 235e56869c1ea883, pixel 62f761fb777fc811 |
| 27 | 1.59 | 29.0 | 0.55 | indirect | mesh 1c451759513efb4c, pixel 43fba4644854c38e |
| 28 | 1.55 | 1.0 | 1.58 | dispatch | compute 39f0e9aeb156a61c |
| 29 | 1.52 | 1.0 | 1.55 | dispatch | compute 9ef9a9d818b27236 |
| 30 | 1.49 | 1.0 | 1.64 | dispatch | compute aab1b3c7248ee5de |
| 31 | 1.48 | 239.0 | 0.02 | indirect | mesh 8ded0d53f8c51e1f |
| 32 | 1.47 | 40.0 | 0.09 | indirect | mesh a6a6702e5c2a8ba9, pixel 43fc2af1f70920f1 |
| 33 | 1.43 | 14.5 | 0.32 | mesh | mesh f0c7efd2aceb2aaa, pixel 7d85c9cab3c80721 |
| 34 | 1.37 | 30.0 | 1.12 | dispatch | compute 251d57fa0adf0b53 |
| 35 | 1.37 | 1.0 | 1.39 | dispatch | compute 66d8f8f6a74feddb |
| 36 | 1.33 | 1.0 | 1.34 | indirect | compute be6ce5d849d51127 |
| 37 | 1.32 | 31.0 | 0.11 | indirect | mesh 5bd95fdec765c0d6, pixel 398971d1222d8f95 |
| 38 | 1.32 | 1.0 | 1.34 | dispatch | compute bfad8c6743ac5977 |
| 39 | 1.26 | 41.0 | 0.06 | indirect | mesh cf43bdbfc75c9296, pixel 573d1857e903fab5 |
| 40 | 1.26 | 110.0 | 0.02 | indirect | mesh 9f1f95875944d82a, pixel f710ed8f1038035e |

The top 40 add up to 143.2 ms of the 231.7 ms.

## Same run, frame summary

Last 3 min: 1.3 fps median, 789 ms/frame = preparing 263 + GPU 306 + copy-back 80; card about 34 % busy; 17 untranslated shaders, 5 refused compute pipelines (02384a60fda6f2d0, 2cc4c197ee29e291, 2da4beb386509b77, 369cbe906ea72764, 6b82bc7d22016cb5). No crash.

## How to repeat

```
cargo build --profile diag -p victory --features diagnostics
set VICTORY_GPU_TIMING=1
victory run <game exe> --execute
```
Then take the block after the last `the card's time by command (timing run), all N kinds together` line in stderr.
