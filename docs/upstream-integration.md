# Upstream integration, stage 1 (2026-11-25)

Branch `claude/project-thread-2l2dcf`, on top of PR #5 (the AnyXboxX lot 5
fps mix). Goal: bring in what Marvisak/VIctory and fxceless/victory-fork did
since the fork's base without breaking the fps work, so it can be built and
measured on the PC.

## How

- **fxceless main (`8dd30ab`)**: merged as a whole (`git merge`), it is the
  base of the fps work. Three small conflicts with PR #5 (see the merge
  commit).
- **Marvisak/VIctory**: *not* merged. Upstream's 28 commits after `ffe310e`
  rebuilt the same subsystems the fork rebuilt (title memory page table,
  rectangle lists, heap views, lane masks, BVH) and changed the binding
  contract between translator and GPU code (root SRV/UAVs as heap views,
  page table `t0`/`u0`, view addresses `t1`, heap flags). A trial merge gave
  36 files / ~280 conflict hunks (same count as fxceless issue #126).
  Instead, upstream commits were cherry-picked (`-x`) one by one onto the
  fork: those that apply cleanly and build were kept, the rest are listed
  below with the reason. Each picked commit keeps its upstream author.
- Checks here: `cargo clippy --workspace --all-targets [--all-features]
  -- -D warnings` for `x86_64-pc-windows-msvc` and `cargo test` of
  `victory-shader`, `victory-game`, `victory-formats` on Linux. The full
  Windows test suite runs in CI. Nothing here was run against the title.

**Risk:** a cleanly applying pick can still interact with the fork's own
rewrite of the same code. If the PC run is worse than PR #5, bisect over
the picks below (they are separate commits).

## Taken from Marvisak/VIctory

108 commits (upstream merge commits are picked with `-m 1`, so each PR is one commit here).

| Upstream | Here | What |
|---|---|---|
| `7389303` | `427af72` | Game formats: effects, techniques and passes of the shader database; shader-info --effects |
| `286e33f` | `8a18bab` | Game formats: the shader pack; shader-info lists packs |
| `532cb7c` | `906f2dd` | Diagnostics: VICTORY_POKE writes title bytes twice a second |
| `224e786` | `b38cdc1` | GPU tests: the frame pacing test tolerates a busy host |
| `b5ac27f` | `7ff9e6b` | #115 Census: report directory traversal failures |
| `f8c7498` | `2383b19` | #122 XAsync: end the call that finished, not whatever call the block holds now |
| `ed6429e` | `f02a29a` | #132 XTaskQueue: terminating a queue cancels its delayed callbacks too |
| `95191f8` | `ca6de26` | #186 Kernel: a title image's file name that fits clears the last error |
| `5d1fbac` | `8650c59` | #207 Media: a share is refused however it is spelled |
| `65aef25` | `70e0e74` | #238 CLI: check for AVX2 before running a title (VICTORY_SKIP_CPU_CHECK=1 to bypass) |
| `c7f1ae3` | `0f3825d` | #335 XStore: a store id without an installed package is an invalid argument |
| `a222fe3` | `ed8ae1c` | #357 HLE: load a host DLL outside the module cache's lock (loader-lock deadlock) |
| `2d8cac2` | `1a9a47f` | #476 Census: deduplicate PE imports without hint |
| `a34ac58` | `0110df8` | #538 xmem: report allocations the host refuses |
| `e82a6ad` | `fb60c77` | #615 GPU: VICTORY_* switches set to 0 are off (#610) |
| `ff15f98` | `b5c048d` | #636 HLE: implement the RPC runtime's GUID calls |
| `417e3f9` | `ad1f2e8` | #664 translate: refuse a VICTORY_EMIT_DUMP directory inside the repo |
| `644ebeb` | `07b23c1` | #665 xtask: count \x escapes and 0x hex arrays as encoded runs in the policy check |
| `76bcd2b` | `18f4e79` | #1080 HLE: constrain the title before suspending it (window close never suspended) |
| `88de5fc` | `c17873e` | #1611 run: stop with a clear message when the title's own DLLs are missing |
| `d4ef9eb` | `b54b29f` | #1677 HLE: show title windows through a stand-in that always reads its messages |
| `26e03c3` | `e56367e` | #1729 XGameSave: a stray or unreadable version folder never costs the save (#870) |
| `4bf5100` | `14737b6` | #12 Check the required package archive before title startup |
| `1117251` | `312c711` | #52 GPU: hold SHADER_HEAPS across descriptor mirror and check query offsets |
| `c5b83b9` | `4d4d16a` | #219 fix(pe): validate section raw data bounds |
| `7b96711` | `c4266a4` | #247 XAsync: a new block where a result was left behind begins a call |
| `c8cbe8a` | `2938234` | #353 Shader info: show stage I/O even when instruction decoding fails |
| `8595259` | `b790842` | #359 HLE: a '>' in a quoted manifest attribute does not end its tag |
| `897efe0` | `0389562` | #370 XGameUi: a text entry's limit is the console's |
| `ac2525d` | `fd51b8e` | #373 Kernel: GetModuleFileName of a title image into a null buffer |
| `e686512` | `bdfb3fd` | #455 HLE: honor the requested executable on relaunch |
| `43b7c08` | `b3ddc03` | #474 GPU: sanitize CreateSamplerX LOD range, bias clamp, and invalid enum values |
| `ce2d91f` | `f07abf9` | #475 XStore: wire offline *Result slots and fix OOB XStoreShowProductPageUIResult slot |
| `2c67b09` | `f46ae8d` | #480 HLE: require two hex digits in percent escapes |
| `805317b` | `6a13b52` | #487 HLE: read manifest fields past inline XML comments |
| `54e7844` | `4930f5b` | #504 GPU: handle depth/stencil copy footprints |
| `648a9ff` | `80db5ef` | #555 PE: keep every name for a shared export ordinal |
| `2fb667b` | `1615cf1` | #617 Shaders: reuse lane guards for unchanged EXEC within a block |
| `dcd24fe` | `e594c94` | #649 Show on-disk file size in victory inspect |
| `9b6d1c0` | `49c1d05` | #957 HLE: test that XGSuggestSwizzleMode picks linear for a 1D texture |
| `05394a6` | `26a1b55` | #993 Formats: share immutable texture swizzle tables |
| `e9f43ed` | `e2f913c` | #1082 GPU: log the TDR settings, and warn when a GPU hang would freeze the PC |
| `357a8bc` | `ee323d0` | #1091 README: how to install Rust on Windows (#1087) |
| `6d06afb` | `3cedbf0` | #1098 XAsync: canceling a call that already completed leaves it alone, as documented |
| `d73d8c9` | `f8fb1d4` | #1109 fix(pipeline): retry collisions when creating cache staging files |
| `7582940` | `12c34bb` | #1161 docs(architecture): list BCPack as implemented in victory-formats |
| `fc438a2` | `6df00b1` | #1171 PE: add synthetic unit tests for implib.rs and layout.rs |
| `69bab30` | `5f837f5` | #1173 GPU: add synthetic unit tests for query.rs and signature.rs |
| `0fcbee4` | `8ad2856` | #1177 Deduplicate image paths in victory census |
| `432b110` | `5cb1f33` | #1204 GPU: guard zero/overflowing stride in expand_rects and add primitive.rs unit tests |
| `02deab1` | `8716583` | #1206 GPU: validate null args in create_deserializer to prevent d3d12 AV and add root.rs tests |
| `5c4f2e9` | `784525d` | #1248 XNetworking: wire UDP port change callbacks and UTF-16 security queries |
| `6355117` | `d351663` | #1274 HLE: complete XPackage progress callback unregistration |
| `eb17946` | `77ec8ff` | #1299 docs: explain Windows shader-test validator prerequisite |
| `05f988c` | `39d5746` | #1390 GPU: keep A8_UNORM out of the R8 typeless family |
| `7db1e07` | `f69a284` | #1411 HLE clears the result of blocked address lookups |
| `12ee4a1` | `8bb4f34` | #1453 GPU: interpolate normalized rectangle vertex attributes |
| `a985b9d` | `37df97d` | #1455 HLE: allow an explicit maximum title window size |
| `e011e88` | `5f43bca` | #1515 HLE: guard dialog commands before initialization |
| `17de445` | `f3f502c` | #1539 Loader: guard clwb_loop shortcut against u64 overflow and wraparound |
| `ebcd90f` | `4c8a3b8` | #1592 CLI: list shaders that complete emission |
| `cbb3431` | `4d03179` | #1594 CLI: preserve the title entry point exit status |
| `a9d233f` | `3781e29` | #1660 GPU: serialize pipeline library atomic writes with unique temp files |
| `f69e487` | `f4ba5f5` | #1667 flush a kept save blob that had to be copied |
| `d6e00a8` | `c432f41` | #133 Files: a title run by its file name alone gets its own folder as G: |
| `2a45e75` | `c5cec7c` | #194 Audio: an XMA loop's skip is of its first frame, not of its last |
| `280e484` | `49c0fee` | #941 Loader: a clwb in the last bytes of a page is emulated instead of faulting |
| `12fa535` | `6b374a4` | #96 xma: start the frame after a continuation at the next packet that has frames |
| `15de8f9` | `ccd7532` | #214 Loader: clzero loops in one exception; rdpru (#34 rebased, with a fix) |
| `1134418` | `03ad00f` | #447 Shaders: a signed 64-bit operand sign-extends its 32-bit literal |
| `0cffc54` | `fc2ef2d` | #937 policy: resolve a .. in the not-yet-existing part of a path |
| `0299812` | `81bd0b1` | #990 Shaders: hardware test for v_cvt_f16_u16 and v_cvt_f16_i16 |
| `181825d` | `a138659` | #1146 GPU diagnostics: create missing capture output directories |
| `1c16afd` | `704c5ec` | #1214 HLE: write decoded Opus samples through unaligned output buffers |
| `41b11b2` | `079887c` | #1477 Shaders: key immediate sampler slots by descriptor and comparison mode |
| `8fb0b28` | `68a870d` | #1549 Formats: test texture layouts against addrlib's own sizes and addresses |
| `aa1bf51` | `f6dcbd9` | #153 GPU: seek directly to private data on final release |
| `256d7b9` | `8449c8d` | #156 Shaders: lower v_mul_hi_u32_u24 and v_mul_hi_i32_i24 |
| `7d99f63` | `a3e6f4e` | #172 XMA: cache IMDCT rotation tables per subframe size |
| `338808e` | `ee8d043` | #183 XMA: noise tests, and end a frame at its data instead of looping forever |
| `3dce4ba` | `9fb32c9` | #221 Loader: suspend a thread before reading its TEB; skip bad dump times |
| `38011a0` | `e7d7ccf` | #270 GPU: every object answers for IUnknown |
| `32b9fbf` | `fdc1633` | #351 Formats: reject overlapping texture depth slices |
| `d763582` | `3d4e62b` | #363 Config: ignore an empty VICTORY_CONFIG |
| `13de0cc` | `f99518e` | #536 Audio: an XMA write block past the ring's end wraps before the first write |
| `e56919d` | `b77d3ee` | #553 GPU: clear DXGI outputs on out-of-range enumeration |
| `40f5553` | `5a6f3b8` | #560 Loader: stage prepared images in per-writer files |
| `9574cee` | `e10e52c` | #629 Reject multisampled linear texture layouts |
| `02fcf5b` | `b44d9c0` | #661 Shaders: fix missing and phantom operands in SP3 text |
| `c6c244d` | `5d0fdac` | #974 GPU: extend HostHeap wraparound search range to cursor bounds (fixes #871) |
| `924457e` | `e47820f` | #1016 XMA: gather frame payloads one byte at a time |
| `6e6004c` | `ef95fcb` | #1021 Shaders: s_atc_probe's immediate is not a register |
| `fdce5f5` | `7f0444a` | #1050 Shaders: negative-zero ordering test for the f16 min/max forms |
| `19469bd` | `b913891` | #1052 Shaders: hardware test for the integer v_min3/v_max3/v_med3 |
| `ef65f88` | `b831280` | #1125 Handle unsupported display deferrals in the HLE test |
| `e2a80cf` | `aec969d` | #1159 loader(crash): verify unwind against known caller frames when RtlCaptureStackBackTrace returns empty |
| `82494a9` | `1ca576b` | #1180 gfx10: resolve the opcode name once, at decode time (translation -4..-8%) |
| `825c045` | `81400c0` | #1282 Shaders: fold conditional branches to their fallthrough |
| `170ab23` | `78ace90` | #1337 Shaders: preserve mip point filtering in anisotropic static samplers |
| `21b4175` | `31f161d` | #1366 Shaders: do not parse VS inputs when reading mesh shader stage I/O |
| `67bcf47` | `5a5a379` | #1371 Shaders: compress replaced phi chains in remove_trivial_phis |
| `5e3bc84` | `1dec576` | #1391 Shaders: lower v_cvt_off_f32_i4 |
| `95f6c6e` | `e9b72b6` | #1443 Shaders: lower scalar quadmask instructions |
| `520fefc` | `8deab1b` | #1471 Shaders: fold empty loop latches containing only dead phis |
| `f73e8e1` | `97382c5` | #1476 Shaders: precompute MD5 round constants and avoid per-block Vec in md… |
| `3c47fd4` | `a508114` | #1562 GPU: fences answer for ID3D12Fence1 as documented |
| `3688adc` | `73fa833` | #1636 Shaders: GFX10 FLAT offsets ignore bit 11 |
| `1bde03c` | `4dce1e2` | #1695 Shader tests: require a device readback in every waterfall test |

## Left out for now (Marvisak/VIctory)

Commits up to `70564ae` (README of 2026-11-24):

| Upstream | Why |
|---|---|
| `0b35b7f` FidelityFX, 64-bit image atomics, BVH, lane masks, bindless strides | changes the translator/GPU contract; stage 2 |
| `a7fc47a` phi edge liveness | the fork rewrote `fill_phis` (`dead()`, `VICTORY_STRICT_JOINS`); upstream's builds on `dead_on_edge`, which the fork removed |
| `13091f6` pixel/mesh I/O, 16-bit packing, signature rows | overlaps the fork's `meet` cache and computed-barycentric halves; fxceless open PRs #110/#112/#124 target the same signature issue (#53) |
| `27c3dc9`, `0f63757` title memory page table | the fork has its own (`SLOT_MEMORY` directory); stage 2, pick one after an A/B |
| `8af0472` compute lane masks | duplicate of fxceless #11 (`90228a7`); A/B on cards first |
| `4051149` null views past a copy | GPU, conflicts with fork's descriptor code; stage 2 |
| `75677cb` buffer sizes from descriptors | 4 `lower.rs` conflicts with the fork's descriptor paths; stage 2 |
| `77053a0` discards, f16 rounding, denormals, med3, signed zeros | 13 conflicts in `structure.rs`/`lower.rs` (overlaps the fork's lane-mask work); stage 2 |
| `4f57f46` cube samples with gradients | the fork already lowers them |
| `c75fdba`, `d71c388`, `86a5ed6`, `bacb06a`, `6d021fe` heap views, bounded root views, copies, world frame cost | GPU, the fork's copy tracking is a different design; stage 2, one at a time with fps A/B |
| `683ad7a` rectangle lists | duplicate: the fork has its own geometry-shader rect lists |
| `13bbd6f` one host texture per texture memory, depth twins | GPU; the fork has depth twins of its own; stage 2 |
| `c47b529`, `819f622`, `9c58a1c` shader warm-up, pipeline pre-build | fxceless has `translate --warm` now; upstream's prebuild needs its GPU pipeline code; stage 2 |
| `e0034c2` register-relative move bounds (`range.rs`) | conflicts with the fork's `spill::build` entry point; stage 2 |
| `7000573` diagnostics in listed frames | GPU capture code differs; skip |

Commits after `70564ae` that did not apply cleanly (conflicts with the
fork's code; to look at in stage 2):

- `b00c14a` Merge pull request #1000 from TG0000/perf/parallel-pipeline-loads
- `9bd146d` Merge pull request #618 from JFIREonFIRE/fix/append-aligned-input-offsets
- `711e930` Merge pull request #1658 from HailLucifer666/fix/texture-staging-oob-and-committed-guard
- `4d140e2` Merge pull request #1718 from tjthebigbearcat-hub/fix/third-pr
- `c813777` Merge pull request #126 from TG0000/perf-pr5
- `46bde7f` Merge pull request #169 from Dufftwt/fix/tbuffer-d16-loads-packed
- `71ac890` Merge pull request #213 from Dufftwt/fix/crash-log-names-host
- `a9d6f17` Merge pull request #322 from jsthope/gpu/title-copies-without-region-queries
- `5c0ebce` Merge pull request #646 from nathanbzrr/shader/wave-exec-const-and-prefix-validation
- `ea48bd3` Merge pull request #669 from iurysf/dred-breadcrumb-identity
- `aa83e3f` Merge pull request #1246 from LaunchDoesntCode/gpu/grow-list-upload-scratch
- `7ae5e17` Merge pull request #1474 from Texbirds/fix/shader-lane-bit-wave-ops
- `fe88382` Merge pull request #1510 from rouges78/fix/indirect-budget-in-order
- `94856e2` Merge pull request #1635 from pes7/perf/indirect-skip-clamp
- `8479c18` Merge pull request #1726 from npanu420/dstorage/late-page-lists
- `83211b9` Merge pull request #162 from Rareuncommon/shaders/sad
- `8a772ea` Merge pull request #174 from AndyLatimer/fix/gpu-frame-wait-reserves-tick
- `a3169a1` Merge pull request #197 from Alpaca-exe/fix/victory-keys-with-connected-pad
- `7448632` Merge pull request #288 from Yaekai/fix/shader-bdd-memo-cap
- `e8e494c` Merge pull request #361 from booomxd/input/walk-all-devices
- `9ce9976` Merge pull request #368 from Quat3rnion/victory-shader-coverage
- `8bf20ad` Merge pull request #603 from AwsAli05/formats/detile-copy-runs
- `6160c9e` Merge pull request #1221 from bartikele/shader-cndmask-other-masks
- `87f01ff` Merge pull request #1290 from sklsp/shader-tests/show-device-error
- `24effa8` Merge pull request #1446 from geiyiliyabumo/fix/list-create-initial-pipeline
- `13d1237` Merge pull request #1626 from AwsAli05/gpu/throttle-swap-chain-retries
- `fcdefcf` Merge pull request #1322 from KevinPratap/fix/shadow-busy-title-writes
- `5fa5de3` Merge pull request #1733 from moe-miau/fix/shader-helper-lane-readfirstlane
- `4436fca` Merge pull request #1696 from Calvix2k/codex/shader-interp-p2-f16-029
- `4118f33` Merge pull request #928 from BastienRodz/gpu/device-removed-reason
- `42363ab` Tests: hardware-only shader checks skip on a machine without a GPU
- `ccfb029` Tests: a machine with no GPU has no hardware adapter to check shaders on
- `cd94d35` GPU: shaders mark the pages they write through views; copy-backs compare only those
- `940d32f` GPU: a marked page's copy-back does not save the page for its own copy

## Next (stage 2)

Measure this branch against PR #5 on the PC. If it is not worse, take the
GPU-side upstream work one feature at a time, each with its own fps run.
