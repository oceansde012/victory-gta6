# FPS mix: fxceless/victory-fork plus seven open PRs

Step 3 of the plan (frame rate). Instead of starting from scratch, this branch
reproduces the best public mix known on 2026-11-24, the one the
[AnyXboxX](https://github.com/LooperSalty/AnyXboxX) project (LooperSalty) pins
as its VIctory submodule.

## Source

AnyXboxX README and THIRD_PARTY.md (commit `608cd4b`), submodule
`third_party/victory` = `LooperSalty/victory`, branch `anyxboxx/fork-7b0-lot5`,
commit `fd021bf`.

Their numbers (maintainer, i7-13700KF + RTX 4070 SUPER, Windows 11, VIctory log
after about 2 min in the world):

| Build | fps | Draws / frame | Notes |
|-------|-----|---------------|-------|
| fxceless `d279e69` | ~4 | — | device lost after ~13 min |
| fxceless `7b0f736` + `VICTORY_RELAY=2` | ~4.5 | — | |
| same, launcher settings of AnyXboxX PR #172, no DRED | ~5.5 | ~4,410 | |
| lot 4: `7b0f736` + #63 #71 #32, `VICTORY_RELAY=2` | ~6.6 | 5,986 | startup flicker fixed, scene dark |
| lot 5: `7b0f736` + #63 #71 #32 #82 #83 #81 #77, no `VICTORY_RELAY` | ~5.4 | 6,449 | best image so far, 9 untranslated shaders, 16 pipelines refused, no RT |

`VICTORY_RELAY` is off in lot 5 because fxceless issue #89 reports it makes
character skin disappear and overlaps subtitles.

Launcher settings they use (AnyXboxX PR #172, `tools/lancer_victory.ps1`): no
DRED, loop limit 256, `VICTORY_LATE_WAIT=3`, `VICTORY_WIDE_BACK=1`, card
buffers; `VICTORY_RELAY=2` only for lot 4.

## What this branch contains

Our `main` + fxceless `main` (`2119b0e`) + the seven PRs, merged in the lot 5
order: #63, #71, #32, #82, #83, #81, #77. No conflicts.

Differences from AnyXboxX lot 5:

- Base is newer: AnyXboxX stays on `7b0f736` and skips `1684989` and later,
  which their maintainer judged a regression (not measured in their notes).
  Our base includes `1684989`, `3930989`, `90228a7`, `4e17478`, `2119b0e`.
  If fps on the PC is worse than expected, compare with `LooperSalty/victory`
  `fd021bf` as is.
- PR #71 has one more commit than in lot 5 (`c57c9b0`, asymmetric right-hand
  partial hash in `CARD_COMPARE`).

## To measure on the owner's PC

1. Build this branch and `fd021bf` (exact lot 5).
2. Same spot in the world, ~2 min, read fps and draws per frame from the log.
3. Try with and without `VICTORY_RELAY=2` (lot 4 vs lot 5 trade-off).
