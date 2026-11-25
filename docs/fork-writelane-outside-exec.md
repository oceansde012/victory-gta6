# `v_writelane_b32` and EXEC

Written so that a person or a coding agent can test this or carry it on.

## What was wrong

On the console `v_writelane_b32 vN, sX, K` writes lane K whatever EXEC is. The translator
lowered it as an ordinary vector write, under the guard of the lane that runs it, so when
lane K was outside EXEC the write was lost and a later `v_readlane_b32 sY, vN, K` gave the
old value.

The compiler uses this pair to park scalars in VGPR lanes. `ir::unspill`
(`crates/victory-shader/src/ir.rs`) turns the pairs into scalar moves, but only for a VGPR
whose every use is such a pair with a constant lane. A VGPR with any other use keeps its
lane writes, and those were the ones affected.

## The fix

`crates/victory-shader/src/emit/lower.rs`, the `"v_writelane_b32"` arm. Lane K takes the
scalar exactly when the console wave runs the instruction:

* a write with a lane guard asks the wave whether some lane's guard holds (`waveAnyTrue`)
  and is written without the guard. A wave with no lane in EXEC, which on the console took
  the `s_cbranch_execz` around it, still writes nothing;
* a console wave64 program run as two host waves of 32 exchanges the two answers (two group
  barriers), and the lane written must be there to take the scalar. Both need every lane,
  so a lane write in a block not every lane runs is refused with "a lane write where not
  every lane runs", as a lane read already is, and `emit` goes on to the forms without the
  skips, where every lane runs the block.

## Test it

```
cargo test -p victory-shader --lib a_lane_write_reaches_its_lane_outside_exec -- --nocapture
```

It needs a hardware device with shader model 6.6; without one it prints `not run` and
passes, which is not a result. On Windows 10 the in-box D3D12 stops at 6.5 (PR #69 adds
the newer runtime). Useful to report: the card, its wave size, and the lines the test prints.

Program: one console wave of 64, `if (t > s0) { v_writelane_b32 v9, s6, 0 }` with s6 = 5 and
v9 = 7 before, then every thread stores `v_readlane_b32` of lane 0.

| s0 | threads inside | expected | before (RTX 3090) | now (RTX 3090) |
|----|----------------|----------|-------------------|----------------|
| 10 | 11..63, lane 0 outside | 5 | 7 | 5 |
| 62 | 63, lane 0 outside | 5 | 7 | 5 |
| 63 | none (the wave skips) | 7 | 7 | 7 |

"Expected" is the instruction's documented behaviour (it ignores EXEC), which `unspill`
already relies on. It has not been measured on a console.

## Not done, not known

* Not run on a card with 64-lane waves (AMD). There the fix is the `waveAnyTrue` alone.
* With the console's own wave size, a lane write inside a per-lane branch that lane K does
  not take is still lost: lane K is not there to take the scalar, and that host has no
  other form to fall back to. Unchanged by this.
* Not run over a title's shaders. To check on a machine that has them, before and after:
  `VICTORY_HOST_WAVE=32 victory translate --validate` and compare the "translated" and
  "validated" counts, and the reasons listed under them. A program that newly stops with
  "a lane write where not every lane runs" in every form is a regression to report.
* Cost: two group barriers per guarded lane write on a 32-lane host, none elsewhere. Not
  measured in a frame. Cheaper where it applies: let `unspill` take a parking VGPR that
  has other uses when the parked lanes' live ranges do not overlap them (no cost per frame).
* Whether this is what makes the 14 `CS_ShadowRenderTiled*` programs wrong in the "without
  skips, scalars followed" form (issue #62). `VICTORY_HOST_WAVE=32 VICTORY_WAVE_WHY=1
  victory translate --list` naming `v_writelane` / `v_readlane` for them would point here;
  then an A/B of the prison cell with and without this change settles it.
