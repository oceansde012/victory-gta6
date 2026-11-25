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

## Open fxceless/victory-fork PRs merged (2026-11-25)

Each is its own merge commit (`git revert -m 1 <merge>` drops one). Picked
for frame time, GPU stability and shader correctness; all merged without
conflicts and build. Duplicates left out per fxceless issue #126's overlap
map (#110 and #124 beside #112; #119 beside #139; #115, which fxceless
main's view guard already covers; #91/#106; the Wine/Proton set).

| PR | What | Why |
|---|---|---|
| #95 | textures no longer wait for remappings of other title memory | texture lane cost is the remap lock (fxceless issue #178) |
| #108 | batch adjacent physical-page scatter unmaps | fewer system calls when streaming |
| #144 | bind 256-byte-aligned table CBVs directly from shadow buffers | fewer copies per draw |
| #154 | VEH-guarded in-process copy for `copy_mapped` / `write_mapped` | copies without a system call |
| #158 | wave64: halve programs whose LDS accesses are the lane's own words | RTX cards are 32-lane |
| #172 | a skipped pipeline is not started early either | no wasted pipeline builds |
| #177 | batch 64 KiB block queries in texture `mapped_bytes` | fewer queries |
| #169 | `VICTORY_INDIRECT_CLAMP=0` opt-out of per-draw argument clamping | test switch for the indirect draw cost (fxceless issue #107: 40 of 63 ms in 3,227 indirect draws); default unchanged |
| #79, #66 | bound direct compute dispatch work; loop caps documented | GPU hang guard |
| #67 | release directly indexed buffer ranges with their heap | leak |
| #112, #61, #122 | DXIL signature order, PSV0 names, ISG1 usage after opt | shaders the runtime refused (fxceless #53, #12) |
| #135, #139, #155 | 64-bit compares, `v_writelane_b32`, f16 narrowing switch | shader correctness |
| #76, #78, #90, #87, #102 | test adapter wave width, 1-thread LDS programs, rect corner tolerance, volume pitch, a test | correctness |

Not merged (conflict with this branch, later): #175, #104, #165, #124, #150,
#160, #163, #170, #182 and the rest of the conflicting list.

Fix on top: the dynamic descriptor slot search (fork's scan cap cut off
upstream #974's wrap past the cursor; CI test).

## Correction and fxceless main `a26859f` (2026-11-25, later)

- The merge `6674a60` recorded fxceless `8dd30ab` as a parent, but its tree
  only kept the files `8dd30ab` added; the rest of its changes (about 6,000
  lines, mostly `victory-gpu`) were lost. Commit `4c44a70` applies the diff
  from the merge base `2119b0e` to `8dd30ab` on top, keeping the earlier
  conflict choices (depth bounds in `pipeline/host.rs`, Marvisak's
  descriptor-as-image path in `lower.rs`). Any fps number measured on the
  earlier head of this branch did not include `8dd30ab`'s GPU work.
- fxceless main `9376fd6` (heap pass on the CPU: 70-220 ms -> ~3 ms a
  frame) and `a26859f` (first room renders right) merged as one merge.
  Conflicts: `host.rs` relay fields (kept both: `order_after` from the
  copy-queue PR and `sent_as`/in-flight fetch), `lower.rs` unused
  descriptors (fxceless `unused_descriptor_kind` first, then Marvisak's
  fallback path).
- Open fxceless PRs merged clean, each as its own merge: #785 (relay
  `late_met`/`relay_land` race), #786 (relay/card merges write only
  GPU-changed runs), #783 (skip unchanged heap pages, one write-back).

## Stage 3 (2026-11-25): branch `integration/2026-11-25`

Rollback point: PR #7's head before this stage was `9de1ab7` (code as
`03e328f`); `git reset --hard 9de1ab7` on a local copy returns to it.

Base: the Codex stack (`codex/fix-clzero-loop`, PRs #10 #11 #13 #14) with
PR #7's head merged in. Backup of that state:
`backup/2026-11-25-pre-integration` (`deefbbb`). Kept as is: the
`VICTORY_WAVE_LDS_LOOSE` switch, `VICTORY_TEXTURE_STAGE_MS=60`, Codex's
fixes.

### Taken (separate commits, `-x` where cherry-picked)

| Commit | Source | What |
| --- | --- | --- |
| `1b55c20` | VItoriaBr `8cb509b` | vertex graphics pipelines compiled first |
| `316b83f` | VItoriaBr `fe0506a` | helper lane `v_readfirstlane` reads its own value |
| `79801f4` | VItoriaBr `1ac7518` | `Interp::Weighted` for lone `v_interp_p1` halves |
| `daa5262` | VItoriaBr `18df163` | `calculateLOD` array slice undef |
| `4faa222` | VItoriaBr `40beb26` (#10) | RDNA2 LDS bounds checks |
| `b3a5b0c`, `4452a4b`, `7014af1` | VItoriaBr `67bb579`, `2ec2aa2`, `c248547` | immediate sampler range (non-pixel), join-path phi |
| `14224aa` | VItoriaBr `2b3c339` | quad reads with helper lanes (lines on car paint, glass, skin) |
| `f8afec2` | fxceless #783 `d695015` + `5e3e18e` | #783's final form, see below |
| `005b9f1` | fxceless #781 `67efc51` | pooled texture staging buffers |
| `d826b90` | fxceless #793 `c4d5ca5` | no diagnostic budget atomics/readbacks in the indirect clamp outside diagnostics builds |
| ~~`cc72b39`~~ | fxceless #787 `04aff73` | reverted: CI crashes in victory-gpu tests (see revert commit) |

### #783 and #786: the write-back conflict

Our base merged #783 in its first form (`b5dee8d`). That form wrote a
copy back in one call: the gaps between the GPU's runs were filled with
the title's bytes as read earlier and the whole stretch written after
the loop, over any title write that landed in between. That undoes
upstream #2009 and #786 (write only the GPU's runs). The author dropped
it after review (`d695015`); it did not change the copy-back time.
`f8afec2` removes it and takes the rest of #783: kept page bytes are
forgotten when a heap is released, and only records between a page's
first and last changed byte are walked (upstream: descriptor sync 18.9
-> 10.5 ms a frame, RTX 3080). Adapted to Codex's registry-held
`CopiedSlots` (`CopiedSlots::needs`). #785 and #786 were already in.

### Not taken

- VItoriaBr `c0b60a0` (console element order in signatures): breaks
  `signature_sorts_packed_registers_by_semantic_name_offset` on our base.
- VItoriaBr #9 `7c19646` (dynamic descriptor wraparound): our base
  already scans `cursor + n - 1` (with `self.dynamic`), duplicate.
- VItoriaBr #11 `dcc0305`: test only, needs console formats 189-191
  that our base lacks.
- VItoriaBr #7 `5603b45` (indexed view byte stride), #6 `a6a8d39`
  (`victory warm`): conflicts in 4-5 files, next step.
- fxceless `f85101b` (submit thread, copy queue): its copy queue clashes
  with ours, own step.
- vitutu1/victory-perf: its 9-14 fps come from Series S mode, 50% render
  scale and stand-in lighting; ideas only.

### Checks (Linux, cross-target)

- `cargo clippy --workspace --all-targets --all-features --target
  x86_64-pc-windows-msvc -- -D warnings`: clean.
- `cargo test -p victory-shader --lib`: 198 passed.
- `cargo test -p xtask`: 6 passed.
- Not yet run on the title. Release build and fps runs on the PC.

### Regression bisect (1644f2b 3.5 fps -> 03e328f 1.85 fps)

Run K showed `VICTORY_WAVE_LDS_LOOSE=0` does not bring it back (2.1
fps), so the cause is elsewhere among the six code commits between them
(`c297f4f` #804 is reverted). Builds, each from `1644f2b`:

- a: `58a7b02` (#802) + `7f0ca9c`; b: `e34702d` + `477fe24` + `cc970bc`
  (#799) + `4096561`.
- quarters: a1 `58a7b02`, a2 `7f0ca9c`, b1 `e34702d`+`477fe24`, b2
  `cc970bc`+`4096561`.

## Other projects, checked 2026-11-25 (afternoon)

- **vitutu1/victory-perf** (GPL-3.0, base fxceless `310c6eb`): 9-14 fps
  claims are Series S mode + 50% render scale + lighting stand-ins on a
  4070 SUPER. New today: ports from victory-world-runtime (`1412d43`) and
  watch/JOURNAL lock work (`4cb867b`). Deep conflicts with our base; ideas.
- **ZSlayerHQ/victory-world-runtime** (GPL-3.0, `next`): fxceless
  `310c6eb` + joshuu/victory-amd, "walls fix confirmed on an RTX 5090":
  multi-view guard (we have it via fxceless `8dd30ab`) and the 9-bit mesh
  index mask (taken: `b204a79`, Marvisak `c0228d93`).
- **PastiiJay/klbf-lab** (GPL-3.0, AMD RX 9070 XT + 9800X3D): ~7.3 fps in
  the cell, ~5 fps native, both with `--console series-s` (1280x720) and
  AMD FSR. Its "cell walls missing" fix: pipelines waited for while the
  world loads, textures shown from their first loaded mip, lost placed
  views refreshed, dispatches over 65,535 groups split (from
  myokex/victory-amd). Candidates for our see-through walls if `b204a79`
  is not enough.
- **"Experimental victory-fork PR at 7.6 fps"**: not found. fxceless #770
  (validation stack on `f85101b`) is the likely one; its page was not
  readable today.
- Trackers (portwatch tracker.json, gta6-pc-race): no updates since
  2026-11-24 evening.

## Belobug/VIctory-Belobug-AMD (checked 2026-11-25)

GPL-3.0. Base: Marvisak main `8e79c80` (2026-11-24) plus 501 commits of
its own, not fxceless. One machine: RX 9070 XT 16 GB, 9800X3D, 64 GB;
3-5 fps in the world. Its README: about 22 GB RAM and "more video memory
than a 16 GB card has"; not tested on NVIDIA.

- Light in shadow is a **constant stand-in** (`VICTORY_AMBIENT`, 0.6):
  the ray-traced indirect light does not run there either. Our `03e328f`
  runs the title's indirect light computes (#799), which is better
  picture; not taken.
- Already in ours by other routes: quad reads with helper lanes,
  `v_readfirstlane` helper lanes, guard over every view of a page.
- Twice the dynamic descriptor slots (`2aad398`, draws dropped when they
  ran out): AMD only. On NVIDIA the host heap is capped at 1,000,000 views
  and our dynamic slots come out of the title's. To check on the PC: does
  the log say "no free dynamic descriptor slots" where walls vanish?
- Hoisted indirect clamps (`ccab973`, `918ab03`, `76387c2`: clamps of a run
  of indirect draws recorded before it, no flicker in mode 7): an idea for
  our indirect cost (113 ms a frame in run L). 22 conflict hunks on our
  base; to port by hand, not cherry-pick.
- Every other commit checked conflicts (ambient chain, root-view reads,
  shader swap); the two code bases differ too much for picks.

## Picture first (2026-11-25 evening)

Taken into PR #7 (`b65fad7`): Codex's `codex/pr7-voxel-visibility-mask`
(DexNom `76749ce`), `codex/pr7-empty-dispatch-2556` (Marvisak #2556),
`codex/stencil-texture-upload`; fxceless #956 (textures picked by index
read again), #795 (levels on demand keep their tiles); Marvisak #2297
(texture sampled before an in-list copy uploaded again). fxceless #922 was
left out: our base already turns blending off on integer targets
(`VICTORY_FIX_INTBLEND`), and #922 defined the same function twice.

**DexNom/VIctory** (GPL-3.0, `921d8d3`): Marvisak main with
victory-world-runtime squashed in (`570a941`, 170 files, +49k/-36k) and 11
commits of its own. It reports 25,742 of 25,758 shaders translating, an
open world close to retail on an RX 7900 XTX, and experimental RT
(`victory-play --rt`) at about 1 fps. Its own commits conflict in 4 to 18
files each on our fxceless base (29 to 58 hunks for the picture ones), so
porting it piece by piece is not practical. For a correct picture soonest,
build DexNom as it is on the PC (its `Setup.cmd`, or `cargo build --locked
--release -p victory --features diagnostics --bins`) and compare it with
PR #7 at the same spot. It is untested on NVIDIA.
