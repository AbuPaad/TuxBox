# Bug and port recon TODO

This file holds findings from a comparison of TuxBox against the sibling projects for the same
title. It also holds the defects found during that comparison. Each entry gives the evidence, the
file and line that decide it, and what would close it.

No entry claims that a bug is reachable in play. The status field says whether someone saw it,
measured it, or only read it in another project's log.

`LOGBLOCK.md` collects the open questions whose answer is a log line from a real run. Each
entry there names the `BR-n` item it closes, the switch that produces the line and the text to
keep. Use it for anything with status `verify`, and for the run half of `open`.

## Sources

- `DoubleGhosty/cool_fox`. A GPL-3.0 fork of VIctory, with RDNA3 work. Read at `f4b48ad` on
  2026-11-25. Take ideas only. It is GPL-3.0-or-later and TuxBox is Apache-2.0. Do not copy source
  from it. This file cites its documents and its measurements, not its diffs.
- The `git.gtahub.st` mirrors, read on 2026-11-26. This pass covers only forks that name AMD or
  Linux.
  - `gmanka/victory` carries `Marvisak/VIctory` plus merged forks. Its description is "Victory
    development and source branches for AMD on Linux, with Wine and native runtime tests". Read at
    `d56316b`. Licence: GPL-3.0-or-later.
  - `gmanka/klbf-lab` comes from `PastiiJay/klbf-lab`. Its author measured the AMD fixes on an RX
    9070 XT. Read at `420e1e1`. Licence as above.
  - `avatarmanz/VIctory-7900XTX` is the RDNA3 tree that `cool_fox` forks. Its README has a measured
    table of renderer patches. Read at `a9400a6` on 2026-11-26. Licence as above.
  - `cool_fox` is a fork of `avatarmanz/VIctory-7900XTX`.
  - These trees were not read in depth: `realblastex/flamingo` (an RDNA2 recompiler),
    `XaviFortes/VIctory-RDNA`, `trustno1-coder/VIctory-NVIDIA`, `Fayberr/Victory-United`,
    `edsuuu/VI-BR-PORT`, `DexNom/VIctory`, `xcl-dev/xcl`, `boykopovar/AnyPS5`.
- TuxBox itself at `773278a`, and measurements taken on this host on 2026-11-26 and 2026-11-27.
- `docs/superpowers/specs/2026-11-26-storage-staging-design.md` covers the storage work.

## Status values

- `open`: work to do.
- `already-fixed`: upstream already does it.
- `not-transferable`: their fix does not fit our architecture.
- `verify`: needs a run or a receipt. No code change.
- `doc`: the documentation is wrong.
- `dead-end`: another project measured it and rejected it. Do not spend time here.
- `finding`: a result from this host that changes the plan.
- `reference`: a cost model or a table to consult.

---

## BR-1 — The memory mirror already narrows its copies

Status: `already-fixed`. One check is outstanding.

`docs/USAGE.md` (Memory copy) says the provider copies the CPU's changes into the private guest-page
copy before each submission. That text reads as a whole-page copy. The code is narrower.
`diff_copy_chunks` at `runtime/d3d12/src/device/heap/shadow.rs:194-219` skips unchanged 4 KiB pages.
It then syncs only 64-byte chunks where the CPU copy diverged from the baseline. GPU-written chunks
stay intact.

That is the idea cool_fox credits for 69 ms down to about 6 ms. Its source is `docs/findings.md`,
under "Stores into guarded memory looked at first". The same document records 14 ms down to 4-7 ms
for the shader-write compare.

The delta on their side is explicit AVX2. Our page compare is a slice `==`, which is `memcmp`.
glibc dispatches `memcmp` to AVX2 on this host. Expect a small gain at best.

Action: no code change. One question remains. Does the mirror path cost too much on RADV to keep?
`docs/USAGE.md` records PR #200 on an RX 6700 XT as slow, with two crashes in three runs. Measure
`stats` before any change.

## BR-2 — Descriptor publication cannot wait for a table bind

Status: `not-transferable`.

cool_fox leaves descriptor record mirroring to the draw. It reports 280 ms down to about 124 ms. Its
document counts 400,000 slots looked at per frame, with only 40,000 mirrored. Most of them were
empty. The source is `docs/findings.md`.

Our path has a different shape. `Write::drop` publishes per created view
(`runtime/d3d12/src/device/descriptor_paging.rs:641-648`). It copies staging to visible for that view
only. The slot machinery already retires on real fence completion (`retire_completed`, line 135). It
also evicts the least recently used entry (`evict_oldest`, line 158).

A deferred copy would break bindless indexing. The title reads heap slots directly. There is no table
bind to carry the flush. Their engine can do this because it tracks per-draw table strides.

Action: revisit only with a dirty-range design that understands bindless. That work is a spec, not a
port.

## BR-3 — An oversized indirect dispatch has no guard here

Status: `open`. Reachability is unknown. BR-14 describes a design that splits it.

cool_fox records "Blocky patches: indirect dispatches cut at 65,535 groups" in `docs/findings.md`.
It also defaults `VICTORY_INDIRECT_REST=0`, so one oversized record cannot zero later draws. See its
bug journal, 2026-11-25 21:30.

We do the opposite of their first bug. `execute_indirect` passes `max_count` to vkd3d with no clamp
and no split (`runtime/d3d12/src/commands.rs:502-521`). Their truncation bug is therefore not ours.
If the title asks for more than 65,535 groups, the host behaviour belongs to vkd3d. We do not record
what it does.

Action: log `max_count` and the dispatch count from the count buffer when either passes 65,535. That
log answers the question on the next run. Then choose between splitting and refusing. Put a
synthetic case beside the Lanes `gpu-indirect-dispatch` scenario.

## BR-4 — Shader-written textures never return to guest memory

Status: `open`. This is the largest correctness gap found.

cool_fox reports that the ocean height and displacement maps are textures. Compute passes write them.
The title's physics samples them on the CPU. Nothing brought them back, so boats sank through the sea
in every run. See `docs/findings.md`, "Boats sank".

TuxBox reads back only BLAS sizes (`runtime/d3d12/src/device/blas/rebuild.rs:293-306`). Nothing
returns a shader-written texture to the CPU. A related but different path is the deferred placed-image
refresh in `docs/SCARLETT_ALIAS_DEFERRED.md`. That path defers publication of an overlapping image. It
does not return shader writes to the guest.

Action: write a spec first. It must state which resources are CPU-readable, when the copy runs
relative to submission, and what the CPU sees between copies. This work is not a patch.

## BR-5 — Quad and helper-lane reads on AMD

Status: `done` for the synthetic part, `verify` for the visual part. Updated 2026-11-27.

cool_fox reports square holes and dashed shadow edges. Its cause is pixel shaders that read their
quad's neighbours with wave reads. On AMD, helper lanes take no part in them. See
`docs/findings.md`.

We have quad support: `uses_quad` at `tools/shader-compiler/src/hlsl.rs:328-329`,
`hlsl/pixel_quad.rs`, and `OP_QUAD_MOVE`. We also have `group_extrema::prepare_exchanges`.

Action: on a RADV run, check shadow map edges for square holes with `TUXBOX_WAVE_DEBUG=1`. Compare
the same scene on NVIDIA. Put a synthetic reproduction in the shader compiler tests. Those tests run
on this host.

Done on 2026-11-27, in two parts.

The synthetic reproduction exists: `tests/hlsl/pixel_quad.rs` now covers `ds_swizzle_b32` with
QDMode, which is the instruction a shadow-edge neighbour read compiles to. Three tests:

- a broadcast pattern reads one lane with `QuadReadLaneAt`. That line carries no `exec_lo` predicate,
  so a helper lane's value participates.
- a rotation pattern reads all four lanes and indexes them by the quad lane.
- a pattern without the QDMode bit stays refused with "ds_swizzle without QDMode unsupported".

A host wave intrinsic skips helper lanes. AMD includes them in the quad read only when the read
sits outside the emulated EXEC mask. The emitter already places it outside. That is now pinned
rather than assumed.

Still open: the visual comparison needs a RADV run and an NVIDIA run on the same scene. No test on
this host can replace it.

## BR-6 — Normalized buffer codes may be one step off

Status: `done`. Tested on 2026-11-27.

cool_fox reports two defects under "Faults found". DXIL's `Log` is base 2. Normalized buffer codes
were one step off.

The `Log` half is already right here. `v_log_f32` emits `log2` and `v_exp_f32` emits `exp2`. See
`tools/shader-compiler/src/hlsl/emitter/vector.rs:542-543` and `ray/alu.rs:553-554`. The
normalization half is not checked. Our buffer emitter handles formats explicitly
(`hlsl/emitter/buffer.rs:113-135`, `hlsl/emitter/image.rs:29,453`).

Action: write a table test for the emitted SNORM conversion. Use the endpoints `-128`, `-127`, `0`
and `127`. Compare against the RDNA2 definition. The test runs on this host:
`cargo test --manifest-path tools/shader-compiler/Cargo.toml --target x86_64-unknown-linux-gnu`.

Done on 2026-11-27 as `tests/hlsl/buffer_snorm.rs`, with the conversion moved into `snorm16_hlsl`.
The emitter divides by 32767, which is the D3D definition, and clamps the one code below -1 to -1.0.
The test also pins a property that reads wrong at first: the midpoint code 16384 yields 0.500015,
not 0.5, because the range is asymmetric. The full crate suite passes, 340 tests.

## BR-7 — Ray dispatch counts: threads or groups

Status: `verify`.

cool_fox records "Every ray missed: the dispatch flag that says counts are threads, not groups"
(`docs/findings.md`). We handle the compute half of that flag. See `DISPATCH_USE_THREAD_DIMENSIONS`
at `runtime/d3d12/src/commands/compute.rs:13`. `runtime/d3d12/src/commands/ray_dispatch.rs` has no
equivalent guard.

Action: read the ray dispatch path against the console semantics. Record the answer. If the guard is
absent, add it with a synthetic case.

Answer, read on 2026-11-27. No gap. `ray_dispatch.rs:47-50` multiplies `Width`, `Height` and `Depth`
as thread counts and compares the product against `D3D12_RAYTRACING_MAX_RAY_GENERATION_SHADER_THREADS`.
It then passes the descriptor to `host.DispatchRays` unchanged, and the indirect path carries the same
`D3D12_DISPATCH_RAYS_DESC`. That entry point has no group-versus-thread flag to misread, unlike
compute, where `DISPATCH_USE_THREAD_DIMENSIONS` decides. The fork's fault sat in a path that reads a
flag. We have no such path. No test added, because there is no behaviour to pin.

## BR-8 — The documentation says the slots are never reclaimed

Status: `doc`.

`docs/PROJECTSCARLETT.md:271` says "Shader-visible descriptor slots are not reclaimed and can run
out." That line came in with `a79aedb` at 2026-11-25 06:08 -0600. Commit `98043df` added the reclaim
about 90 minutes later, at 07:37 -0600. The in-game exhaustion in the same section (frame 32,534,
exit 3) predates the fix as well.

The mechanism exists now in `descriptor_paging.rs`. It provides `retire_completed`, `evict_oldest`,
`try_alloc_contiguous`, `ensure_direct_capacity` and `log_exhausted`. The Lanes `descriptor-reclaim`
scenario covers it.

Action: fix the bullet. Describe the current behaviour. Cite the commit and the scenario. Keep the
risk visible, because the title uses far more slots than the scenario does.

## BR-9 — `python build.py lanes` fails on a clean tree

Status: `done`. Fixed on 2026-11-27.

`build.py:372` maps `lanes` to `("xmem", "d3d12", "gameinput", "lanes")`. The lanes branch then copies
`build/bvh/XboxBvhBuilder.dll`. None of those targets builds that file. A clean checkout ends with
`Build failed: [Errno 2] No such file or directory`. The failure comes after all the expensive work
succeeds. Build `bvh` first and the rerun passes.

Action: add `"bvh"` to the lanes target tuple. Add a test that every source in the copy list is an
output of the listed targets.

Done on 2026-11-27. `LANES_FILES` pairs each shipped file with the target that builds it.
`LANES_TARGETS` derives from that list with lanes last, because its link step needs every provider
library. `tests/test_build_lanes.py` covers the pairing, the omission and the order.
`python build.py lanes` now completes on a clean tree.

## BR-10 — No descriptor-reclaim receipt for this GPU

Status: `verify`.

`evidence/` holds a `lanes-selftest-descriptor-reclaim` receipt only for `1002:7590` (RX 9060 XT).
This host reports `1002:73DF` (RX 6700 XT, RADV).

Action: run `python tuxbox.py run --app-id Lanes build/lanes --selftest descriptor-reclaim`. A pass
adds the receipt. A failure finds a difference between AMD generations.

## BR-11 — Dead ends another project measured

Status: `dead-end`. Do not try these again. cool_fox measured them on an RX 7900 XTX, under "Measured
and not worth it" in `docs/findings.md`.

- Unordered early pipelines for everything. AMD's compiler crashed.
- The Series S profile with smaller textures.
- A second drawing queue.
- `ASYNC_SUBMIT` off.
- Mapping pages on sixteen threads. It mapped no more pages and made frames longer.
- Scalar buffer loads of four dwords per call. No gain.
- Wave sizes, fused multiply-adds, fast reciprocals, raw picked buffers and dropped barriers. Each
  cost under 0.1 ms.
- Exact RDNA float rules. 0.8 ms.
- Running the console's shader code natively on the card. The Windows driver takes no raw code. The
  generations differ. A Vulkan host layer ends at the same compiler.
- An on-screen number panel. It costs frame time, so it sits behind a key.

---

## BR-12 — Section commits under Wine scan every committed range

Status: `open`. This affects every Proton start. Not testable on this host: `tuxbox-xmem` does not
build for Linux either, because the crate gates `os::windows`, so any change here is compile-checked
only.

`gmanka/victory` comments on this at `crates/victory-hle/src/modules/xmem.rs:448-452`. Under Wine a
committed section is a sparse file, free until touched. `SEC_RESERVE` then costs one wineserver
request per commit. Each request scans every committed range. Their code passes `SEC_COMMIT` when
`on_wine()` is true. They detect Wine through `ntdll!wine_get_version`, lines 37-47.

TuxBox creates its shared section with `PAGE_READWRITE | SEC_RESERVE` and no condition. See
`runtime/xmem/src/shared_pages.rs:76`. It then commits every page through one temporary view, line 170
of the same file. Our path therefore pays the cost they removed, on every start.

Action: choose `SEC_COMMIT` when the process runs under Wine. Detect it the same way. The
flag-selection helper is testable on this host. A Proton run must measure the gain.

## BR-13 — The host view heap cap comes from the driver

Status: `open`. This can cause a device loss.

`gmanka/victory` commit `d29359c` carries the message "Cap the host view heap on every GPU: AMD
page-faulted at view slot 2^23". Their `crates/victory-gpu/src/descriptor/host.rs:1146-1215` sizes the
host view heap to end at a null slot. Shaders clamp view indices to that slot. Their code warns when
the title's count is cut.

TuxBox takes the driver's figure when it is larger than 1,000,000. See
`runtime/d3d12/src/device/resources.rs:178`, which calls `options.MaxViewDescriptorHeapSize.max(1_000_000)`.
`TUXBOX_DESCRIPTOR_HEAP_LIMIT` overrides it. The Lanes `descriptor-reclaim` scenario uses that
override to pin 1,000,000.

Action: log what vkd3d-proton reports for `MaxViewDescriptorHeapSize` on RADV. Then decide whether the
AMD default stays at 1,000,000. The override already exists, so a run answers this with no code
change.

## BR-14 — Split a one-dimensional dispatch that is over the host group limit

Status: `open`. This is the design for BR-3.

The `gmanka/victory` README says the runtime can split a one-dimensional compute dispatch that
exceeds the host group count limit. The code is in `crates/victory-gpu/src/indirect.rs`. Their commit
`3298729` also names the clamp mode in the draw benchmark. Their emit side carries the matching
comment at `crates/victory-shader/src/emit/shader.rs:146-151`.

Our `execute_indirect` forwards `max_count` unchanged (`runtime/d3d12/src/commands.rs:502-521`).
`commands/compute.rs` has no split. On the console, one indirect dispatch can exceed 65,535 groups.
The host limit belongs to vkd3d.

Action: first log the counts, as BR-3 says. If they exceed the limit, split over the argument buffer.
Each submission takes `min(65535, remaining)` groups with the argument offset advanced. Keep one
indirect record per submission. Put the case beside the Lanes `gpu-indirect-dispatch` scenario. That
scenario already checks untouched guard words for 0, 1 and 2 groups.

## BR-15 — Guarded reads and readbacks need a range check

Status: `verify`.

`gmanka/victory` carries two fixes. One guards every view of a memory page. It comes from myokex
`victory-amd` commit `c6dc4d8`, which adapts VIctory PR #115 and later corrections. The other is the
read-beside fix for reads into title memory, by Joshua Odom at `10ea38e`. Their own commit `3c5b440`
carries the message "GPU: guard readbacks with title memory ranges".

TuxBox has guards and readbacks of its own. See `runtime/d3d12/src/device/blas/rebuild.rs`,
`commands/page_mapping.rs` and the xmem write watch. One question is open. Does a readback or a
guarded read that lands outside the mapped range get rejected? Or does it wrap into the next
allocation?

Action: read both paths against that question. Add a bounds case to the xmem and placement scenarios
if the check is missing.

## BR-16 — The stale-hole check may race the mapping lock

Status: `verify`.

`gmanka/victory` commit `d56316b` carries the message "GPU: hold the mapping read lock while checking
stale holes". TuxBox has the same kind of work in `runtime/d3d12/src/commands/page_mapping.rs`. That
file holds pinned imports, `ImportPin` and the live and retired transitions. `device/heap/shadow.rs`
retires a shadow on remap.

Action: find any check of "are these pages still the ones I mapped" that runs outside the lock that
retires them. If such a check exists, it is a rare lost write. cool_fox recorded that class: six of 200
runs.

## BR-17 — One submission per command list on AMD

Status: `verify`.

`gmanka/victory` commit `1d9aba9` sets one submission per list on AMD. Its commit `8d69a29` makes
per-list submission and dispatch splitting opt-in. TuxBox already submits per list, so the behaviour
may match. What matters is whether batch boundaries differ between RADV and NVIDIA.

Action: compare the recorded queue trace on both vendors. Look for partial submissions whose fences
stall.

## BR-18 — Textures waiting on other remappings, and upload order

Status: `verify`.

Two `gmanka/victory` commits describe upload bugs. `7a20fd3` is "Textures no longer wait for
remappings of other title memory". `acf2762` is "Refresh texture staging before recording upload
preludes". Both appear as late or missing textures on AMD.

TuxBox's nearest pieces are the deferred placed-image refresh (`docs/SCARLETT_ALIAS_DEFERRED.md`) and
the texture path under `device/`.

Action: read the upload prelude order against their description. `docs/USAGE.md` records that retail
titles stream textures late, so the symptom family matches.

## BR-19 — AMD compiler and wave hazards from klbf-lab

Status: `done` for the skip list, `verify` for the wave and upload findings. Updated 2026-11-27.

The `gmanka/klbf-lab` CHANGELOG on branch `klbf-amd` reports four AMD findings. Their author measured
them on an RX 9070 XT.

- One shader stuck in the AMD compiler, hash `39f0e9aeb156a61c`. Their build skips it by default,
  using a hash-keyed skip list at `docs/klbf/skip-amd.txt`.
- 32-lane pixel shaders run per half wave on 64-lane hardware. The result is purple blocks.
- A NaN/Inf filter on colour outputs.
- A larger, configurable texture upload queue, for late or missing textures.

The first two have TuxBox counterparts. `docs/SHADERS.md` covers 64-lane programs on 32-lane hosts.
The shader preparation cache covers the other. On Linux the compiler is RADV's, not `amdxc64.dll`, so
their skip list does not transfer. The mechanism does transfer. It is a per-build deny list of shader
hashes that fail to compile.

Action: check whether preparation already records and skips a hash that fails to compile.
`ShaderRefused` in `project_tuxbox/runtime_shaders.py` suggests it does. Then decide whether a manual
deny list adds anything. Their Proton users reach the world on an RX 7800 XT and an RX 6900 XT
(`docs/GUIDE.md:119`).

Answered and extended on 2026-11-27.

Preparation already skips a program that fails to compile. `compile_one` writes `<key>.reason` and an
`<key>.unsupported` marker, and `reuse()` returns the remembered reason on the next boot, so a refused
translation costs one attempt. A manual deny list would duplicate that. It would also need upkeep
against a build we do not control.

The gap was the other failure. A *hang* never returns, so there was no error to remember, and every
boot paid the timeout again. `run_tool` now reports a timeout as `ToolFailure(timed_out=True)`, and
`note_hang` counts it in `<key>.timeouts`. The second timeout publishes the same marker a refusal
writes, with the reason "compiler timed out after 30s, 2 attempts". One timeout may be a stall of the
host, so a single one only counts. `tests/test_shader_hang_skip.py` covers the count, the publication,
the counter reset and the timeout reporting.

## BR-20 — Runtime HLSL against Wine's `d3dcompiler_47`

Status: `verify`.

Under Wine and Proton, `d3dcompiler_47` is vkd3d-shader. The `gmanka/victory` README records two
effects at lines 653 and 673. This compiler rejects `RWByteAddressBuffer` methods. Where it does
compile, it drops the root signature the shader asks for. Their indirect clamp, card-side compares and
depth twins then switched off with no error. Their fix sends that source to `dxcompiler.dll`.

TuxBox requires the Windows x64 DXC DLLs beside the game (`docs/PROJECTSCARLETT.md`). It also forwards
`DxcCreateInstance` through `runtime/dxc`.

Action: check that every runtime HLSL compile in the provider goes through DXC. None may fall back to
the system compiler.

## BR-21 — The card reads constant buffers where the title writes them

Status: `already-fixed`, with one part that cannot transfer.

The `avatarmanz/VIctory-7900XTX` README of 2026-11-26 measures four renderer patches. Two concern
constant buffers.

- `VICTORY_PORT_CONSTANTS=1` lets the card read the title's 174 MB constants heap where the title
  writes it. There are no uploads and no fills.
- `VICTORY_PORT_CBV=1` makes translated shaders read constant buffers in place as raw bytes. Their
  note reads "~22,000 fills a frame to none, -4.7 ms".

TuxBox binds the guest address directly. There is no fill. See `commands/compute.rs:143`, which calls
`constant_buffer(list, index, guest)`. `docs/SCARLETT_ROOT_CBV_WINDOW.md` records a regression control
that reads slot 41 straight from CPU-backed guest pages.

The part that cannot transfer is their "no uploads" result. It needs the card to read the title's own
pages. RADV cannot import those pages (`docs/USAGE.md`, Memory copy). Their measurement is therefore
an upper bound for this host, not a target.

## BR-22 — Write-guard lock granularity

Status: `open`. This is a plausible win on the mirrored path.

The `avatarmanz/VIctory-7900XTX` README describes `VICTORY_PORT_GUARD=1`. It makes the write guards'
lock shared by faults and refreshes, with one claim per 64 KiB block. Planning then runs beside the
lock. Their note reads "title threads' waits 29 ms to ~2 ms a frame, -2 ms". Commits `cc761bf` and
`9d723e8` describe the same change from both sides.

TuxBox keeps one `Mutex<ShadowState>` per shadow (`runtime/d3d12/src/device/heap/shadow.rs:39`). It
holds that lock for the whole diff and copy in `sync_cpu_writes` (line 109). A submission that syncs a
large shadow therefore blocks every other user of that shadow.

Action: claim per block, not per shadow. Divide the shadow into 64 KiB blocks with a state per block.
A fault and a refresh on different blocks then proceed together. Check with two 64 MiB shadows driven
from two threads. Assert that only overlapping blocks wait on each other. The parts that need no
device can run on this host.

## BR-23 — Descriptor-table stores against a whole-heap scan

Status: `not-transferable`. Useful as a cost model.

`VICTORY_PORT_DESCRIPTORS=1` patches the title's descriptor-table stores. Each store then reports the
slots it writes. That replaces "a scan of ~600,000 slots a frame". The approach works because that
project patches the title's code. TuxBox does not patch the game, so the option is not available here.

The figure still sizes the problem for BR-2. A scan-based descriptor path costs about a frame's tail on
an RX 7900 XTX. Use that number if our per-view publication ever appears in a profile.

## BR-24 — Output size, render scale and the 30 fps cap

Status: `open`. This is a product decision and a low priority.

The same README lists three switches under "Other switches". `VICTORY_OUTPUT=1920x1080` makes the
title render at that size, for "about 16 ms less card time". `VICTORY_RENDER_SCALE` and
`VICTORY_FFX_SCALE` drive its upscalers. `VICTORY_FRAME_INTERVAL_MS=16.667` lets the title pass 30 fps,
and their note says the game's clock stays right. `--console series-s` gives 1080p output for about
5 ms.

TuxBox's nearest pieces are the Series S graphics trial (`--graphics-menu`, `--graphics-profile
series-s`, `docs/USAGE.md`) and the packaged settings. It has no output-size or frame-interval switch.

Action: none now. Revisit after the title reaches a picture. A smaller output is the cheapest large win
on an RX 6700 XT. BR-25 ranks the levers.

## BR-25 — What a frame costs, and which settings reduce what

Status: `reference`. This entry is not specific to D3D12 or to one platform. It applies to any host
that keeps a second copy of title memory.

The entry exists because "lower the NPC density and water quality to 30 %" is a recurring idea. The
answer is that density touches half of the cost. The numbers come from `avatarmanz/VIctory-7900XTX`
`docs/findings.md`. The measurement host was an RX 7900 XTX on Windows. The other AMD forks report
the same shape.

Where a 55 ms frame goes on the water, in the first boat mission:

- The thread that prepares submissions takes 48 ms. It is the wall.
  - Its parts are list steps 18, refreshes of host copies 10, and steps done on the card 7.
  - The rest is waiting for descriptor records 6, the write watch 5 and card-memory copies 4.
- The card is busy for about 41 ms.
- List calls number 36,000 a frame, which is 230 ms of work across eight threads. A street chase with
  many cars reaches 67,000 calls and 430 ms.
- Private memory reaches 40-46 GB. Lists' allocators take 9 GB, windows 7 GB and uploads 4 GB.

What scales with scene density:

- Draw count, and with it per-call translation, descriptor records and sync steps. Cutting density
  attacks the dominant CPU cost in rough proportion, because that cost is per call.
- The title's own simulation thread. That thread is a smaller share than the layer's overhead.

What does not scale with density:

- Work per submission: mirror copies, write-watch scans and descriptor publication. These run every
  frame whatever is on screen. BR-1, BR-12 and BR-22 cover this cost.
- The layer's allocation footprint: list allocators, windows and uploads.
- Video memory held by render targets and by ray-tracing structures.

Levers, ranked by measured effect on a 12 GB card:

1. Output size. The title renders at 1920x1080 instead of 4K, for about 16 ms less card time
   (`VICTORY_OUTPUT`). It also makes the largest video-memory saving, because every render target
   shrinks.
2. The Series S profile. It gives 1080p output with half-scale textures, about 5 ms on top of item 1
   (`--console series-s`, or `--graphics-profile series-s` in TuxBox).
3. Ray tracing off. It removes the acceleration structures. The install carries 7.8 GB of
   `pack/core_bvh4` for that path alone.
4. Render scale (`VICTORY_RENDER_SCALE`, `VICTORY_FFX_SCALE`). It belongs to the same family as item 1
   and is less well measured. Their README marks the FidelityFX variant "not safe".
5. Scene density, last. It is a title-internal value with no exposed switch in the console build.
   Changing it means patching the game's code. The Windows forks do that. TuxBox does not.

For TuxBox, the available switches are the BR-24 set, because they act at start. Density and water
quality are not ours to set. A density cut would also not rescue a TuxBox frame. BR-1 gives the reason:
the mirror cost per submission does not change with the number of people on screen.

## BR-26 — A Windows fork builds and binds on this host under Wine

Status: `finding`. It changes where the next hour of effort should go.

Tested on 2026-11-27 on this host (`1002:73DF`, 32 GB, GE-Proton11-14). The subject was
`Belobug/VIctory-Belobug-AMD` branch `rx9070xt` at `81ebb7b`, a GPL-3.0 Windows tree. This session
built and ran it here. No source entered TuxBox. Licence: ideas and measurements only.

The build works. `cargo xwin build --locked --release -p victory --target x86_64-pc-windows-msvc`
finished in 1m40s. It used the toolchain from the build-requirements work: rustup 1.98.1 from the
repo's `rust-toolchain.toml`, cargo-xwin, and the staged xwin SDK. The output is `victory.exe` at
9,889,280 bytes, machine `0x8664`, subsystem 3 (console). The target directory is 456 MB. The compiler
printed 13 warnings.

A bare Proton wine does not start it. `files/bin/wine ./victory.exe` fails before `main`:

    err:module:import_dll Library libvkd3d-1.dll (needed by wined3d.dll) not found
    err:module:import_dll Library dxgi.dll (needed by victory.exe) not found

The exit code is 53 (`ERROR_BAD_NETPATH`). The vkd3d DLLs live in `files/lib/vkd3d/x86_64-windows`.
Proton's own launcher adds that directory to the search path. A bare wine call does not. Set
`WINEDLLPATH=<proton>/files/lib/vkd3d/x86_64-windows`, or use `umu-run`. With either, the CLI, `config`
and `inspect` all run.

The tree binds this host's game executable. `victory.exe inspect games/game_scarlett_master.exe`
reported four things. The BLAKE3 hash is
`b99f876b74d4a77f03af5f878223b27bd647b5c35a79d5decd9f716504239e93`. The PDB path is
`X:/americas/src/sub_story/game/vs_project/Gaming.Xbox.Scarlett.x64/Build/master/game_scarlett_master.pdb`,
with key `2A6912BAEB4F427F4C4C44205044422E1`. The image has 10 sections, entry `0x56a1ec8` and 2 TLS
callbacks. `victory.exe run <exe>` mapped the image and printed its per-import HLE coverage. Bink,
XCurl and the rest read as "missing" because the packaged DLLs were not beside it. It stopped at
`not executing (pass --execute to start the title)`.

Its host gates are met here. `crates/victory-gpu/src/host.rs:3904-3926` requires `D3D_SHADER_MODEL_6_6`
and `D3D12_OPTIONS7.MeshShaderTier != NOT_SUPPORTED`. Our own probes under GE-Proton11-14 on this card
report:

    d3d12-features PASS pci=1002:73df queries=7 binding=3 heap=2 root=1 dxr=0 allocations=4
    d3d12-mesh     PASS pci=1002:73df tier=1 stream=680 direct=6 amplified=12 indirect=2 zero=0

Mesh tier 1 is therefore available through vkd3d on RADV here. `dxr=0` says ray tracing is not.

The remaining gap is footprint, not portability. These figures come from the projects' READMEs:

| | TuxBox | Belobug | avatarmanz |
| --- | --- | --- | --- |
| Private memory | ~15 GB plus swap | ~22 GB | 40-46 GB |
| Video memory | no stated floor | "more than a 16 GB card has" | 16-24 GB |

On 12 GB of video memory the Belobug tree is over budget. The avatarmanz tree would use swap.

Consequence: the shortest path to a picture on this machine is a Windows tree under Wine with
`--console series-s`. That gives 1080p output and half-scale textures. The alternative is to wait for
TuxBox to translate the remaining ~4,800 shader requests. TuxBox stays the only Linux-native vehicle,
and its open items stay in this file.

Reproduce the result with these commands:

    cargo xwin build --locked --release -p victory --target x86_64-pc-windows-msvc
    export WINEPREFIX=... WINEDLLPATH="$PROTONPATH/files/lib/vkd3d/x86_64-windows"
    "$PROTONPATH/files/bin/wine" ./victory.exe run ./game_scarlett_master.exe

Do not commit their sources or binaries into this repository. Their licence is GPL-3.0-or-later. Ours
is Apache-2.0.

## BR-27 — Translation coverage: the four buckets and what the forks solved

Status: `open`. This is the gate in front of any frame-rate work.

The current figures come from `docs/SHADERS.md` (ProjectScarlett refusals). Preparation compiled
20,583 of 25,079 recorded requests and refused 4,496. PortWatch still quotes an older reading of
12,087 of 16,868. Use the 25,079 figures.

The refusals group into four work buckets. Each row gives the count, the code that raises the
refusal, and the idea from a fork that applies.

| Bucket | Requests | Ours raises it at | Fork idea to reuse |
| --- | --- | --- | --- |
| Mesh request recorded without a root signature | 1,551 | `tools/shader-compiler/src/hlsl/mesh.rs:87` (`mesh root signature missing`) | none needed. This is our recording gap |
| Wave reduction that may run in divergent control flow | 1,502 | `hlsl/rewrite/lanes.rs`, limit at 6,000 instructions | `a2a358d` merges several loop back edges into one latch and raises the pipeline stack |
| Store to a table that is not an unordered-access table | 577 (432 image, 145 buffer) | image and buffer emitters, UAV rule | not documented in their trees |
| Raw buffer `IDXEN` access that needs the console stride | 129 | `device/shader_cache.rs:322` | `1f1195c` resolves heap-loaded view addresses |
| Lane operation with a non-constant lane | 115 | `hlsl/rewrite/lanes.rs:285` | `40205c7` and `c866560` scope wave operations per half wave |

Order of work, cheapest first:

1. **Mesh root signature, 1,551 requests, one third of the total.** The provider already attaches a
   root at pipeline creation (`mesh_link_source` and `link_mesh` in `device/shader_cache.rs:248-274`).
   The offline record does not carry it, so preparation translates without it. Action: record the
   root beside the source, as `.link` already records a linked pair. This needs no GPU.
   Implemented on 2026-11-27. `compile::record_source` writes the pipeline-root variant into
   `requests/` under its own digest, and `mesh_link_source` calls it once per digest. The worker
   already accepts a 64-hex `.dxbc` name, so the change needed no Python edit. The provider builds,
   but the crate has no runnable tests on this host, so a run must check the 1,551 drop.
2. **`IDXEN` stride, 129 requests.** `refused_for_heap_strides` already exists in the same file.
   Action: record the per-slot stride with the request. This also needs no GPU.
   Partly done on 2026-11-27. The refusal now names the descriptor it tried, which is the line LB-19
   needs. The stride field itself is not recorded yet.
3. **Wave reduction in large programs, 1,502 requests.** The limit stays until the generated code
   compiles faster. A trial translated 1,211 of 1,442. Its samples took 2-12 s in DXC, produced
   1-2 MB of DXIL, and driver compiles reached six minutes. Action: cut emitted code size first. The fork
   commit `a2a358d` is the model, because one latch for several back edges removes duplicated blocks.
4. **Store to a non-UAV table, 577 requests.** Action: bind the table as unordered access when the
   program writes through it. Check this against a synthetic case in the shader compiler tests.
   Partly done on 2026-11-27. The heap and MTBUF half now names its descriptor at the refusal, which
   is the line LB-18 needs. The root-descriptor half still needs a root fixture.

The forks report near-complete coverage: `avatarmanz` translates 25,790 of 25,985 and calls the rest
unused geometry shaders. That number comes from a tree that patches the title, so treat it as a
ceiling, not a target.

Every change in this entry can be tested on this host:
`cargo test --manifest-path tools/shader-compiler/Cargo.toml --target x86_64-unknown-linux-gnu`.

---

## Verification constraints on this host

- **`tuxbox-d3d12` cannot be tested here.** `cargo test -p tuxbox-d3d12` fails before it reaches our
  code. Dependency `windows-future 0.2.1` does not build for `x86_64-unknown-linux-gnu`, with
  `cannot find type IMarshal in windows_core::imp`. The repo reflects this. `project_tuxbox/check.py`
  runs only `cargo check --target x86_64-pc-windows-msvc -p tuxbox-d3d12`. Any change in that crate
  made on this machine is compile-checked only.
- Host-runnable suites: `tools/shader-compiler` tests, `tuxbox-dxbc --features native-bindings`,
  `tuxbox-xg-layout`, and the Python tests under `tests/`.
- Any work that needs the GPU needs the owner to start it, with one GPU client at a time. The amdgpu
  panics recorded on 2026-11-21 give the reason.
- The GTA VI package on this host is incomplete. `game_scarlett_master.exe` is present and hashes to
  the pinned build. 29 GiB of 208.5 GiB is on disk. No run-based item can close until the download
  finishes.

---

## BR-28 — What the other ports found about stores, strides and hangs

Status: `research`. Read on 2026-11-27 from `git.gtahub.st` mirrors and the upstream GitHub trees.

Four findings bear on the open buckets.

**The store and stride pair belongs to the host, not the translator.**
`Fayberr/Victory-United` `docs/bug-journal.md`, entry "16:50–18:20, facial deformation & wrapped
meshes (`d16` + table strides)": adds a table view-kind lookup, `SLOT_TABLE_STRIDES = 252`,
`heap_strides`, and dynamic descriptor slot stride population in `descriptor/host.rs`. Its typed
integer lowering branches on `VIEW_INTEGER` / `VIEW_SIGNED`, and after a regression it restricts that
branching to `buffer_load_format_d16_*` and `buffer_store_format_d16_*`. The host reads the descriptor
and tells the shader what kind of view it is. The shader does not infer it from the root signature.

**Write access is a property of the handle.** `xcl-dev/xcl` `crates/xcl/src/gpu/dxil/shader.rs`
models it directly: `ResClass::Uav`, `raw_uav()`, `raw_uav_coherent()`, and `annotateHandle` with the
IsUAV property (bit 12). A program that stores gets a UAV handle for the descriptor it names. There is
no separate "is this table a UAV" test.

**The AMD deny list is one hash.** `PastiiJay/klbf-lab` `docs/klbf/skip-amd.txt` holds a single line,
`39f0e9aeb156a61c`. That matches BR-19: one program hangs the AMD compiler, and the rest translate.

**Unordered early pipelines are a dead end.** `avatarmanz/VIctory-7900XTX` `docs/findings.md` records
"Unordered early pipelines for everything (AMD's compiler crashed)".

Licences: the VIctory family is GPL-3.0-or-later, XCL is GPL-2.0. Take the shapes and the
measurements, not the code.

Consequence for the 577 refusals: marking the binding writable when a program stores through it is
the approach two independent trees reached. That agrees with the architecture note in BR-27. It also
shows the work spans the provider, which cannot be tested on this host.

### Logs this host still needs

The four log lines and the archive moved into `LOGBLOCK.md` as LB-17 to LB-20, so this file does not
carry a second copy of them. LB-6 already covers the shadow-edge capture for BR-5.

---

## BR-29 — Check the other trees, every time a bucket stalls

Status: `standing`. Instruction added to `AGENTS.md`, section "Working with the other trees".

[git.gtahub.st](https://git.gtahub.st/explore/repos) mirrors about forty efforts on the same title. They
read the same game files. A fault we hit is usually a fault someone else hit first, and several trees
are further along than ours.

What to search, on every check:

- The commit logs, `docs/`, the `CHANGELOG`, the `README` and the source, for the words around the
  fault. Useful words so far: `UAV`, `unordered`, `descriptor`, `stride`, `store`, `wave`, `skip`,
  `TDR`, `dxil`, `radv`, `amd`.
- Search for the symptom, not for the file. Their file layout differs from ours.

What transfers:

- A Linux result, and an AMD result, applies to this host.
- A memory, allocation, engine or archive finding applies to every platform.
- A Direct3D, Metal, Windows-API or driver-registry result belongs to its tree alone. Say why it does
  not transfer instead of dropping it silently.

How to read the mirror. Its API answers 403 and its code search answers 404. Use one of these:

```sh
curl -s "https://api.github.com/repos/<owner>/<repo>/git/trees/<branch>?recursive=1"   # list paths
curl -s "https://raw.githubusercontent.com/<owner>/<repo>/<branch>/<path>"              # one file
git clone --depth 1 <repo> ~/.hermes/cache/scratch/<name>                              # grep locally
```

Licences: every VIctory fork is GPL-3.0-or-later and XCL is GPL-2.0. Take ideas, measurements and
test cases. Do not copy code. Credit the tree in the entry.

Checks so far:

| Date | Trees read | Result |
| --- | --- | --- |
| 2026-11-27 | Victory-United, XCL, klbf-lab, VIctory-7900XTX | BR-28: the store and stride pair is host-side work. Write access is a handle property. The AMD deny list is one hash. Unordered early pipelines crash AMD's compiler |
| 2026-11-27, second pass | Victory-United `docs/bug-journal.md`, full | BR-30: the upload and staging budget numbers, an allocator test technique, and a reverted alias experiment |
| 2026-11-27, third pass | DexNom/VIctory, to `64bd58f5` | BR-31: interior-mip residency, per-texture mapping waits, the helper-lane quad case, the copy-back cost, and cube mip by footprint |

---

## BR-31 — What DexNom/VIctory changed to get every texture rendering

Status: `research`, recorded 2026-11-27. Source: `DexNom/VIctory`, `main` at `64bd58f5`.
Licence: GPL-3.0. Ideas, measurements and test shapes only.

The tree reports all textures rendering. Five commits carry the work that matters to us.

**1. A texture's resident bytes must handle interior-only mips (`921d8d3b`, `texture.rs`).**
`mapped_bytes` answers how many bytes of a texture sit in resident memory. Their old code returned zero
when neither the first nor the last page was resident. That is wrong while a texture streams in. Its
smaller mips sit at the start of its memory, so a texture can hold only interior mips. They now walk
the ranges and skip each missing run in one query. Their test:
`a_texture_with_only_interior_pages_counts_its_resident_bytes`.

**2. A texture refresh waits for its own mapping only (`5f4c3df4`).** A refresh that waits on any
mapping stalls streaming behind unrelated work. Their test:
`a_texture_refresh_waits_only_for_its_own_mapping`.

**3. A pixel quad broadcast must read helper coordinates at a shared triangle edge (`921d8d3b`,
`emit/lower.rs`).** Their `quad_pick` now takes the swizzle pattern. It returns the quad including
pixel helpers across a triangle edge. Common broadcasts and exchanges need one native quad operation. Test: `pixel_quad_broadcast_reads_helper_coordinates_at_shared_triangle_edge`. A second
test pins that DPP8 cannot cross a quad. This is the BR-5 fault seen from the other side.

**4. Copy-back only the pages a shader marked (`cd94d353`, `940d32f6`).** Keep their first
measurement. The world snapshotted every buffer range declared as written before each submission, then
compared it afterwards. That cost 2.8-4.5 GB a frame. It found 98 % unchanged, and it took 0.45-0.7 s
of a 1.5-1.7 s intro-cutscene frame. Almost all of it was one 174 MB GPU-heap buffer. The fix sets a
bit per 64 KiB page in a bitmap for every store or atomic through an unordered view. Each host view
heap keeps that bitmap, and the copy-back compares only the marked pages after a submission. A page
the title also writes keeps its pre-write bytes, saved by the guard on the first write.

**5. A cube map picks its mip by footprint (`921d8d3b`).** The cube SRV carries a mip count and uses
D3D subresource order. Test: `guest_cube_gradient_selects_mip_by_footprint`.

A sixth, smaller item: cached test textures were outgrowing their reserved guest memory
(`2e01ab47`), which is the class of fault a cache needs a bound for.

Actions for this tree, in order:

1. Check whether our provider ever needs a resident-bytes answer for a streaming texture
   (`runtime/d3d12/src/device/heap/*`). If it does, take finding 1 as the rule: interior mips count.
2. Copy finding 4's measurement into BR-1 and BR-22 as the expected cost of a per-submission
   comparison, and the page-bitmap as the shape that avoids it.
3. Check the cube path in `tools/shader-compiler/src/hlsl/emitter/image.rs`: a cube sample with
   gradients must choose its mip by footprint, and that is testable on this host.
4. Copy the quad test case from finding 3 into `tests/hlsl/pixel_quad.rs`: a broadcast where the
   helper lane is the only lane holding the value.

---

---

## BR-34 — A package with symlinked members is unreadable

Status: `bug`, recorded 2026-11-27. Found while starting a TuxBox run from a symlink farm.

`inspect.py:138-155` resolves every package member with `Path.resolve()` and then requires the result
to stay inside the resolved package root. A member that is a symlink into another directory resolves
outside the root, so the loader raises:

```
tuxbox: Package member leaves the package directory: 'MicrosoftGame.config'
```

The check is a sound guard against a manifest naming `..` or an absolute path. The fault is that it
cannot tell a hostile path from a link the user made on purpose.

This blocks the storage staging design in
`docs/superpowers/specs/2026-11-26-storage-staging-design.md`. That design proposes exactly this shape.
A package root sits on fast storage, and the large archives link in from the HDD, later through a FUSE
view keyed by request. The loader would reject every such member.

Options, cheapest first:

1. Resolve the root once and compare the *link's own* parent against it, following the link only when
   the manifest declares the path. A link inside the root then counts as a member.
2. Keep the strict check for manifest-declared paths and add an opt-in environment variable for a
   staging root.
3. Drop the requirement and rely on the declared-path grammar check alone, which already refuses `..`
   and absolute paths.

Option 1 keeps the guard and unblocks staging. Whichever route you take needs a test. The test builds
a package root with one real file and one symlink to a file outside the root.

---

## BR-35 — A title run needs the whole provider set built and copied beside the exe

Status: `finding`, recorded 2026-11-27. Found on the first `tuxbox.py run` of the title.

The launcher does not stage provider DLLs. It expects them beside the executable.
`console_cbv.py:32` looks for `executable.with_name("d3d12_xs.dll")`, and shader preparation reads
`build/d3d12-smoke/d3d12_xs.dll`. The package directory held the title's own DLLs and none of ours, so
the run reached the loader and stopped there:

```
err:module:import_dll Library xmem.dll ... not found
err:module:import_dll Library d3d12_xs.dll ... not found
err:module:import_dll Library chad.dll ... not found
err:module:import_dll Library xg_xs.dll ... not found
err:module:import_dll Library XCRDAPI.dll ... not found
err:module:import_dll Library xapu.dll ... not found
err:module:import_dll Library AcpHal.dll ... not found
err:module:import_dll Library Bink2Scarlett.dll ... not found
err:module:loader_init Importing dlls for game_scarlett_master.exe failed, status c0000135
Game exited with exit code 53 after 6.5s
```

`Bink2Scarlett.dll` sits in the package and still reported missing, because its own dependency
`xmem.dll` was absent. The last line of a log like this names the wrong file. Read the first one.

Two gaps follow.

1. The tree held only six of the provider DLLs: `d3d12_xs`, `GameInput`, `tuxbox_xhttp`,
   `XboxBvhBuilder`, `xmem`, `xcrdapi`. `chad.dll`, `xg_xs.dll`, `xapu.dll` and `AcpHal.dll` come from
   the `route`, `graphics`, `xapu`, `xg` and `acphal` targets. No earlier step ran them.
2. Nothing told the operator that. A run should fail with one line that names the missing provider set
   and the build command that produces it. One line beats a page of `import_dll` errors.

Wanted: `run` checks the provider set before it starts the title. Then it either copies the built DLLs
beside the exe or names the target to build. A missing file should name itself once, not seven times.

Series S profile note: `--graphics-profile series-s` failed separately, at
`graphics_profile.py:79-83`, where `reg add` inside the `bwrap --unshare-net` clone returns nonzero and
the error text drops the reason. Recorded with it because both faults hit the same first run.

## BR-33 — First boot attempt: it binds, opens every archive, then aborts in System Init

Status: `finding`, recorded 2026-11-27 evening.

Setup: `victory.exe` from `Belobug/VIctory-Belobug-AMD` at `81ebb7b`, under GE-Proton's `wine`, prefix
`p2`, `VICTORY_GAME_DIR=/home/fih/gta6-run`, `VICTORY_CACHE_DIR=/home/fih/belobug-run/cache`,
`--console series-s`, `RUST_LOG=info`. The exe is the verified `e95020b2...` build. Process exit 3 after
about 0.8 s.

**What worked.**

| Stage | Result |
| --- | --- |
| Image map and imports | every import bound, 0 missing: exe 94 HLE, 421 host, 72 title |
| Companion DLLs | `xboxbvhbuilder`, `amd_fidelityfx_loader_xbox`, `libhttpclient.gdk`, `bink2scarlett` all bound |
| Console profile | reported `Lockhart`, which is the Series S profile asked for |
| DirectStorage | 8 workers, 32 MB staging, 10 archives opened: appdata0, common, movies, shaders, anim, data, levels, navmeshes, packs, textures |
| Frames | none. Never reached a present |

**Where it died.** `unhandled exception 0x80000003` at `victory-image-b99f876b74d4a77f.dll+0x1ce6e13`,
with frame 10 of the stack at `+0x56a1e5a`, a few instructions below the entry point `0x56a1ec8`.
`0x80000003` is a breakpoint instruction, so the title's own assert fired.

**What the title's crash log says.** It wrote `victory-20261127-052914-244.crash.log` beside a 121 kB
minidump. The fields that matter:

```
Game Running Time: 0h:0m:0s     Game State: System Init [T:103153513]   Frame: 0
Conole Type: Lockhart           Streaming Requests: 0
MemoryINFO: Game Virtual 0/0, Game Physical 0/0, OS 0/0, Pool 0/0
TileHeap: used 0Mb - total 0Mb  Version: (empty)
```

Zero allocated memory and zero streaming requests place the abort before any asset read. The 24 %
download is therefore unlikely to be the cause of this crash. The empty `Version:` field is the most
suspicious value in the report. A title that fills that field from a build description would assert
when the description comes back empty.

**Safety.** The title's crash handler POSTed a 126,161-byte minidump to `submit.backtrace.io` and the
layer refused it: `HTTP connection blocked destination=submit.backtrace.io:443`. This is the fault
`AGENTS.md` warns about, and it is the reason a title runs only through a layer with egress blocked.

**What this is not.** It is not a picture, and it is not a data verdict. It is a boot receipt. Every
archive opens, every import binds, the layer runs the profile the caller asks for, and the title stops
in its own start-up.

Next steps, in order:

1. Find what fills `Version:` in the title image and what the assert compares it against. The image is
   already in `belobug-run/cache/images`, so this needs no run.
2. Build `DexNom/VIctory` for a fair comparison. Their branch is the one that reports every texture
   rendering, and this result comes from a tree three commits behind it.
3. Do not repeat this on TuxBox yet. Coverage is still the gate there, and a Linux boot would add a
   second variable to a fault that already has one.

## BR-32 — A contributor's numbers on your exact card (RX 6700 XT, 12 GB)

Status: `research`, recorded 2026-11-27. Source: a commenter on the VIctory developer post, running a
private VIctory fork. No repository, so these are self-reported numbers with no commit to read.

Their machine is the same class as this host. Ryzen 5 5500, 16 GB RAM, RX 6700 XT 12 GB, PCIe 3.0.
Series S profile, with ray tracing and the BVH builder on. `VICTORY_AMBIENT=0`, FSR 4.1.2 INT8 from
720p to 1080p, and no frame generation.

**What they reached.** 2-2.4 FPS in demanding scenes of 4,100-4,600 draw calls, and about 5 FPS in
lighter gameplay. GPU utilisation rose from 20-39 % to 49-55 %. Runs lasted 220-244 seconds without a
crash. Ray-traced indirect light, reflections and shadows are visible. The black-face fault is gone,
and all 53 BVH builder pipelines are active.

That is the first data point on this card's class with sound visuals. It sets the honest baseline:
**2-5 FPS, not 20**, for a faithful picture.

**1. D3D12 faults.** With `VICTORY_D3D12_DEBUG=1` they found two causes of `DEVICE_REMOVED`:
incorrect resource states during `CopyBufferRegion`, and barriers out of step across execution paths.
They fixed the state tracking and added a local scratch buffer so a copy never runs inside the same
memory window. Result: over 200 seconds of 3D with no severity 0 or 1 error. Our equivalents are
`commands/copy_memory.rs:375-407` and `commands/barriers.rs`.

**2. RT memory budget.** They separate explicit mappings and real GPU faults from speculative
allocations. They cap the speculative share at 25 % of the budget, and they reuse an existing window
at no cost. Before: 62 of 833 RT pages backed while the whole 768 MB budget was spent. After: 634 of 634 backed on the
first RT frame, no rejections for the rest of the run. They moved up to 512 MB of RT windows into
VRAM, and their CPU-side RT comparisons fell from 360-584 MB to 92-130 MB per frame.

**3. Remapping is the biggest single win they report.** Remapping per frame fell from 298-552 ms to
1.6-19.4 ms. Page faults fell from 51,000-137,000 to 45,000-51,000. The levers were dirty page
tracking, memory protection on views, less lock contention, and fewer redundant remaps.

**4. Descriptor overhead.** `descriptor::sync_heaps` fell from 35-94 ms to 14-15 ms per frame. The
levers were parallelising it, removing allocations inside `HostHeap::mirror()`, and spreading
revalidation over several frames, all at about 41,000-42,000 mirrored slots. Our equivalent is
`device/descriptor_paging.rs:585 publish` and the mirror in `device/heap/shadow.rs`.

**5. `VICTORY_INPLACE=1` is a dead end on 16 GB.** Pinning about 416 MB dropped free physical RAM to
57-184 MB and pushed page faults to 1.27 million per frame. Keep it off. This bears on BR-12.

**6. Never measure with diagnostics on.** Building with `--features diagnostics` adds about 200-225 ms
per frame. Benchmarks need a clean release build.

**Still open for them:** CPU-GPU serialisation at 180-250 ms per frame, and copy or copy-back at
80-115 MB per frame. Textures and geometry still stream in late, some objects fail to draw, and GPU
utilisation sits below where it should be.

Actions for this tree:

1. Compare our copy path with finding 1. A copy inside one memory window needs a scratch buffer, and
   the resource-state bookkeeping around `CopyBufferRegion` deserves a test.
2. Take finding 2's split, explicit mapping against speculative allocation, and the 25 % cap, as the
   shape for any RT or BVH residency budget here.
3. Add a remap cost line to `LOGBLOCK.md`. Their 298-552 ms per frame says this is worth measuring
   before it is worth optimising.
4. Record finding 6 beside every performance entry: a diagnostics build cannot be timed.

---

---

## BR-30 — Upload, staging and memory budget numbers from Victory-United

Status: `research`, recorded 2026-11-27. Source: `Fayberr/Victory-United` `docs/bug-journal.md`.
Licence: GPL-3.0-or-later. Numbers and shapes only.

Their entry "19:15-20:30, commit-gate starvation & synchronous bake uploads" carries figures a
limited-memory host can use as a starting point:

| Setting | Value |
| --- | --- |
| Upload reserve | 256 MiB during a bake, 512 MiB otherwise |
| Growth reserve | 256 MiB during a bake, 1024 MiB for speculative bindless |
| Pressure entry | 3 GiB RAM used, 4 GiB commit |
| Pressure recovery | 5 GiB RAM used, 6 GiB commit |
| Staging block growth | 64 MiB blocks, gated to speculative bindless only, rest 2500 ms |
| Speculative staging per frame | 16 MiB under pressure, 32 MiB on a limited-RAM host with 1 GiB live staging |

Their entry "15:45, staging admission" adds three rules. One texture may cross the cap. Required
bakes keep their own budget. An explicit stage-budget override wins.

Two behaviours behind those numbers:

- One-shot bake uploads go through a dedicated synchronous host list. The upload publishes the
  descriptor afterwards.
- The maintenance pass releases spare staging blocks when host pressure rises.

A test technique worth copying: their entry "15:56-15:59, reproducer and fix" replays eight
reported upload sizes through the pool with a failing block allocator. The unpatched test made eight
block creation attempts. That turns an out-of-memory report into a unit test.

A dead end, recorded so we do not repeat it: "20:50-21:10, alias-flood experiment reverted".
Propagating aliased view maps and unmaps into `REMAPPED` with a wider bake lookahead produced 2-4 GiB
bake truncations and visual regressions, and they reverted it. This bears on the alias work at BR-1
and BR-22.

Actions for this tree, in order:

1. Find whether our upload path has a per-frame speculative cap.
   Answered on 2026-11-27: this tree does not stage, so there is nothing to cap. The title's upload
   heaps go to vkd3d-proton. The comparable budget is the imported-heap page cache in
   `device/heap/pages.rs`. Its comment records about 4,500 live buffers, and it notes that RADV has to
   look at every live buffer on each submit. The log line for that is LB-21. Do not add a staging cap
   to this tree.
2. Write the allocator failure-injection test for our own pool, using their eight-size replay as the
   shape.
3. Record the alias-flood result against BR-1 and BR-22 before anyone widens the alias propagation.
