# Bug and port recon TODO

Findings from comparing TuxBox against the sibling Xbox-title projects, plus the defects
found while doing it. Each entry says what the evidence is, where it lives, and what would
close it. Nothing here is a claim that the bug is reachable in play: the status field says
whether it was seen, measured, or only read in another project's log.

Sources read for this file:

- `DoubleGhosty/cool_fox` — a GPL-3.0 fork of VIctory (Windows-native HLE for the same title,
  RDNA3 performance work). Read at `f4b48ad`, 2026-11-25. **Licence: ideas only.** It is
  GPL-3.0-or-later and TuxBox is Apache-2.0, so no source may be copied from it. Findings and
  measurements are cited by document, not by diff.
- The `git.gtahub.st` mirror set, read 2026-11-26, filtered to forks that name AMD or Linux:
  - `gmanka/victory` (upstream `Marvisak/VIctory` plus merged forks; its own description:
    "Victory development and source branches for AMD on Linux, with Wine and native runtime
    tests"). Read at `d56316b`. Same licence caveat: GPL-3.0-or-later.
  - `gmanka/klbf-lab` (upstream `PastiiJay/klbf-lab`), AMD Radeon fixes measured on an RX 9070
    XT. Read at `420e1e1`. Licence caveat as above.
  - `avatarmanz/VIctory-7900XTX` (the RDNA3 tuning that `cool_fox` forks; its README carries a
    measured table of renderer patches). Read at `a9400a6`, 2026-11-26. Licence caveat as above.
  - `cool_fox` (already above) is a fork of `avatarmanz/VIctory-7900XTX`.
  - Not read in depth: `realblastex/flamingo` (RDNA2 recompiler to D3D12),
    `XaviFortes/VIctory-RDNA`, `trustno1-coder/VIctory-NVIDIA`, `Fayberr/Victory-United`,
    `edsuuu/VI-BR-PORT`, `DexNom/VIctory`, `xcl-dev/xcl`, `boykopovar/AnyPS5`.
- TuxBox itself at `773278a`, plus the measurements taken on this host on 2026-11-26.
- `docs/superpowers/specs/2026-11-26-storage-staging-design.md` for the storage work.

Status values: `open` (work to do), `already-fixed` (upstream already does it),
`not-transferable` (their fix does not fit our architecture), `verify` (needs a run or a
receipt, no code change), `doc` (documentation is wrong), `dead-end` (measured and rejected
elsewhere; do not spend time here).

---

## BR-1 — Memory mirror already narrows its copies

Status: `already-fixed`. Verification outstanding.

`docs/USAGE.md` (Memory copy) says the provider copies the CPU's changes into the private
guest-page copy before every submission, which reads as a whole-page copy. The code is
narrower than the doc: `runtime/d3d12/src/device/heap/shadow.rs:194-219` (`diff_copy_chunks`)
skips unchanged 4 KiB pages, then syncs only 64-byte chunks where the CPU copy diverged from
the baseline, leaving GPU-written chunks intact. That is the same idea cool_fox credits for
69 ms → about 6 ms (`docs/findings.md`, "Stores into guarded memory looked at first") and
14 ms → 4-7 ms ("bytes under shader writes compared with AVX2").

The remaining delta on their side is explicit AVX2. Our page compare is a slice `==`, which is
`memcmp`, which glibc already dispatches to AVX2 on this host, so the expected gain is small.

Action: none in code. The outstanding question is whether the mirror path is even taken on
RADV at acceptable cost — `docs/USAGE.md` records PR #200 on an RX 6700 XT as slow with two
crashes in three runs. Measure with `stats` before changing anything.

## BR-2 — Descriptor publication cannot be deferred to table bind

Status: `not-transferable`.

cool_fox leaves descriptor record mirroring to the draw and reports 280 ms → about 124 ms,
with "400,000 slots looked at, 40,000 mirrored a frame, most of them empty"
(`docs/findings.md`). Our path is a different shape: `Write::drop` publishes per created view
(`runtime/d3d12/src/device/descriptor_paging.rs:641-648`), copying staging → visible for that
view only, and the reclaimed-slot machinery already retires on real fence completion
(`retire_completed`, line 135) and evicts LRU (`evict_oldest`, line 158).

Deferring our copy to a table bind would break bindless indexing: the title reads heap slots
directly, with no table bind to hang a flush on. Their engine can do it because it tracks
per-draw table strides. Revisit only with a bindless-aware dirty-range design, which is a
spec, not a port.

## BR-3 — Oversized indirect dispatch is unguarded on our side

Status: `open`. Reachability unknown. See BR-14 for a design that splits it.

cool_fox records "Blocky patches: indirect dispatches cut at 65,535 groups"
(`docs/findings.md`, "Faults found") and later defaults `VICTORY_INDIRECT_REST=0` so one
oversized indirect record cannot zero later draws (bug journal, 2026-11-25 21:30). We do the
opposite of their first bug: `execute_indirect` passes `max_count` straight to vkd3d with no
clamp and no chunking (`runtime/d3d12/src/commands.rs:502-521`). So their truncation bug is
not ours, but if the title asks for more than 65,535 groups the host behaviour is vkd3d's
choice and we do not record what it does.

Action: log `max_count` and the dispatch count read from the count buffer when they exceed
65,535, so a run answers it. Then decide between chunking and an explicit refusal. A synthetic
case belongs beside the existing `gpu-indirect-dispatch` scenario in Lanes.

## BR-4 — Shader-written textures are never read back to guest memory

Status: `open`. Largest correctness gap found.

cool_fox: the ocean's height and displacement maps are textures that compute passes write and
the title's physics samples on the CPU; they were never brought back, so boats sank through the
sea in every run (`docs/findings.md`, "Boats sank"). Fixed by a texture read-back path, on by
default.

TuxBox reads back only BLAS sizes (`runtime/d3d12/src/device/blas/rebuild.rs:293-306`). Nothing
returns a shader-written texture to the CPU. Related but different: the deferred placed-image
refresh in `docs/SCARLETT_ALIAS_DEFERRED.md`, which defers publication of an overlapping image
rather than returning shader writes to the guest.

Action: needs its own spec (what marks a resource CPU-readable, when the copy happens relative
to submission, what the CPU sees between copies). Not a patch.

## BR-5 — Quad and helper-lane reads on AMD

Status: `verify`.

cool_fox: "Square holes and dashed edges in shadows: pixel shaders that read their quad's
neighbours with wave reads; on AMD helper lanes take no part in them"
(`docs/findings.md`). We have quad support (`tools/shader-compiler/src/hlsl.rs:328-329`
`uses_quad`, `hlsl/pixel_quad.rs`, `OP_QUAD_MOVE`) and `group_extrema::prepare_exchanges`.

Action: on a RADV run, check shadow map edges for square holes with `TUXBOX_WAVE_DEBUG=1` and
compare against the same scene on NVIDIA. A synthetic reproduction belongs in the shader
compiler's own tests, which do run on this host.

## BR-6 — Normalized buffer codes one step off

Status: `verify`.

cool_fox: "DXIL's `Log` is base 2, and normalized buffer codes were one step off"
(`docs/findings.md`). The `Log` half is already correct here: `v_log_f32` → `log2` and
`v_exp_f32` → `exp2` (`tools/shader-compiler/src/hlsl/emitter/vector.rs:542-543`,
`ray/alu.rs:553-554`). The normalization half is not verified: our buffer emitter handles
formats explicitly (`hlsl/emitter/buffer.rs:113-135`, `hlsl/emitter/image.rs:29,453`).

Action: a table test over the emitted conversion for `SNORM` inputs at the endpoints
(`-128`, `-127`, `0`, `127`) against the RDNA2 definition. Host-runnable:
`cargo test --manifest-path tools/shader-compiler/Cargo.toml --target x86_64-unknown-linux-gnu`.

## BR-7 — Ray dispatch counts: threads or groups

Status: `verify`.

cool_fox: "Every ray missed: the dispatch flag that says counts are threads, not groups"
(`docs/findings.md`). We handle the compute half of that flag
(`runtime/d3d12/src/commands/compute.rs:13`, `DISPATCH_USE_THREAD_DIMENSIONS`), and
`runtime/d3d12/src/commands/ray_dispatch.rs` shows no equivalent guard.

Action: read the ray dispatch path against the console's semantics and record the answer; if
absent, add the equivalent check with a synthetic case.

## BR-8 — Docs claim descriptor slots are never reclaimed

Status: `doc`.

`docs/PROJECTSCARLETT.md:271` still says "Shader-visible descriptor slots are not reclaimed and
can run out." That line came in with `a79aedb` (2026-11-25 06:08 -0600), about 90 minutes
*before* `98043df` ("d3d12: reclaim shader-visible descriptor slots with lease and fence
pinning", 2026-11-25 07:37 -0600), and the in-game exhaustion it records (frame 32,534, exit 3)
predates the fix too. The mechanism now exists (`descriptor_paging.rs`: `retire_completed`,
`evict_oldest`, `try_alloc_contiguous`, `ensure_direct_capacity`, `log_exhausted`) and Lanes
covers it with the `descriptor-reclaim` scenario.

Action: correct the bullet to describe the current behaviour and cite the commit and the
scenario. Keep the descriptor-memory risk visible, since the game's own usage is larger than the
scenario's.

## BR-9 — `python build.py lanes` fails on a clean tree

Status: `open`. Reproduced on this host, 2026-11-26.

`build.py:372` maps `lanes` to `("xmem", "d3d12", "gameinput", "lanes")`, but the lanes branch
copies `build/bvh/XboxBvhBuilder.dll`, which none of those targets build. On a clean checkout
the run ends with `Build failed: [Errno 2] No such file or directory:
'/home/fih/TuxBox/build/bvh/XboxBvhBuilder.dll'`, after all the expensive work has succeeded.
Building `bvh` first makes the rerun pass.

Action: add `"bvh"` to the lanes target tuple, with a test that the copy list's sources are a
subset of the targets' outputs.

## BR-10 — Descriptor-reclaim receipt missing on this GPU

Status: `verify`.

`evidence/` holds a `lanes-selftest-descriptor-reclaim` receipt only for `1002:7590` (RX 9060
XT). This host is `1002:73DF` (RX 6700 XT, RADV). The gate is
`python tuxbox.py run --app-id Lanes build/lanes --selftest descriptor-reclaim`; a run against
this card adds the receipt, or finds a difference between AMD generations.

## BR-11 — Dead ends recorded by cool_fox; do not re-attempt

Status: `dead-end`. Measured on an RX 7900 XTX (`docs/findings.md`, "Measured and not worth it"):

- Unordered early pipelines for everything: AMD's compiler crashed.
- Series S profile with smaller textures; a second drawing queue; `ASYNC_SUBMIT` off.
- Mapping pages on sixteen threads: no more pages mapped, longer frames.
- Scalar buffer loads of four dwords per call (`VICTORY_SHADER_OPT=127`): no gain.
- Wave sizes, fused multiply-adds, fast reciprocals, raw picked buffers, dropping barriers: each
  under 0.1 ms. Exact RDNA float rules: 0.8 ms.
- Running the console's shader code natively on the card: the Windows driver takes no raw code,
  the generations differ, and a Vulkan host layer would end at the same compiler.
- An on-screen number panel: costs frame time, so it is behind a key.

---

## BR-12 — Section commits under Wine scan every committed range

Status: `open`. Cross-platform, affects every Proton launch.

`gmanka/victory`, `crates/victory-hle/src/modules/xmem.rs:448-452`, comments that under Wine a
committed section is a sparse file that is free until touched, and that `SEC_RESERVE` then costs
a wineserver request *per commit*, each of which scans every committed range. Their code passes
`SEC_COMMIT` when `on_wine()` (detected through `ntdll!wine_get_version`, lines 37-47).

TuxBox creates its shared section unconditionally with `PAGE_READWRITE | SEC_RESERVE`
(`runtime/xmem/src/shared_pages.rs:76`) and commits every page through one temporary view
(same file, line 170). So our path pays exactly the cost they removed, on every title, on
every launch under Proton.

Action: choose `SEC_COMMIT` when running under Wine, detected the same way. Host-testable part:
the flag-selection helper. Needs a Proton run to measure.

## BR-13 — Host view heap cap is taken from the driver on AMD

Status: `open`. Possible device loss.

`gmanka/victory` commit `d29359c`, "Cap the host view heap on every GPU: AMD page-faulted at
view slot 2^23", and `crates/victory-gpu/src/descriptor/host.rs:1146-1215`: they size the host
view heap to end at a null slot that shaders clamp view indices to, and warn when the title's
count is cut. Their README also records the AMD fault at that slot.

TuxBox takes the driver's figure when it is larger than 1,000,000:
`runtime/d3d12/src/device/resources.rs:178`
(`options.MaxViewDescriptorHeapSize.max(1_000_000)`), with `TUXBOX_DESCRIPTOR_HEAP_LIMIT` as an
override that the Lanes `descriptor-reclaim` scenario uses to pin 1,000,000.

Action: log what vkd3d-proton reports for `MaxViewDescriptorHeapSize` on RADV, then decide
whether the AMD default should stay at 1,000,000 regardless of the driver. The override already
exists, so a run can answer it without a code change.

## BR-14 — A one-dimensional dispatch over the host group limit is split, not refused

Status: `open`. This is the design for BR-3.

`gmanka/victory` README, "World runtime integration": "The runtime can split a
one-dimensional compute dispatch that exceeds the host group count limit", implemented in
`crates/victory-gpu/src/indirect.rs` (their `3298729` also names the clamp mode in the draw
benchmark). Their `victory-shader` emit side carries the corresponding view-heap clamp comment
at `crates/victory-shader/src/emit/shader.rs:146-151`.

Our `execute_indirect` forwards `max_count` unchanged (`runtime/d3d12/src/commands.rs:502-521`)
and `commands/compute.rs` has no split. On the console a single indirect dispatch may exceed
65,535 groups; the host limit belongs to vkd3d.

Action: first log the observed counts (BR-3). If they exceed the limit, implement the split over
the argument buffer: N submissions of `min(65535, remaining)` groups with the argument offset
advanced, preserving one indirect record per submission. A synthetic case belongs beside
Lanes' `gpu-indirect-dispatch` scenario, which already checks untouched guard words for 0, 1 and
2 groups.

## BR-15 — Guarded reads and readbacks need title-memory range checks

Status: `verify`.

Two fixes carried in `gmanka/victory`: the guard over every view of a memory page
(myokex `victory-amd` `c6dc4d8`, adapting VIctory PR #115 and corrections), the read-beside fix
for reads into title memory (Joshua Odom, `10ea38e`), and their own `3c5b440` "GPU: guard
readbacks with title memory ranges".

TuxBox has guards and readbacks of its own (`runtime/d3d12/src/device/blas/rebuild.rs`,
`commands/page_mapping.rs`, xmem's write watch). What is unverified is whether a readback or a
guarded read that lands outside the mapped range is rejected rather than wrapping into the next
allocation.

Action: read both paths against that question; add a bounds case to the xmem and placement
scenarios if the check is missing.

## BR-16 — Stale-hole check races the mapping lock

Status: `verify`.

`gmanka/victory` `d56316b`, "GPU: hold the mapping read lock while checking stale holes". TuxBox
has the same shape of work in `runtime/d3d12/src/commands/page_mapping.rs` (pinned imports,
`ImportPin`, live/retired transitions) and `device/heap/shadow.rs` (retirement on remap).

Action: read our import path for a check of "are these pages still the ones I mapped" that runs
outside the lock that retires them. If it exists, it is a rare-write-loss bug of exactly the kind
cool_fox recorded ("A rare lost write, 6 of 200 runs").

## BR-17 — One submission per command list on AMD

Status: `verify`.

`gmanka/victory` `1d9aba9` sets "one submission per list on AMD", and `8d69a29` makes per-list
submission and dispatch splitting opt-in. TuxBox submits per list already, so this may be the
same behaviour; what matters is whether the batch boundaries differ between RADV and NVIDIA.

Action: compare the recorded queue trace on both vendors; look for partial submissions whose
fences stall.

## BR-18 — Textures waiting on other remappings, and upload ordering

Status: `verify`.

`gmanka/victory` `7a20fd3` "Textures no longer wait for remappings of other title memory" and
`acf2762` "Refresh texture staging before recording upload preludes" both describe upload path
ordering bugs that show up as late or missing textures on AMD. TuxBox's adjacent pieces are the
deferred placed-image refresh (`docs/SCARLETT_ALIAS_DEFERRED.md`) and the texture path in
`device/`.

Action: read the upload prelude ordering against their description. Same symptom family as
`docs/USAGE.md`'s note that retail titles stream textures late.

## BR-19 — AMD compiler and wave hazards found by klbf-lab

Status: `verify`.

`gmanka/klbf-lab` CHANGELOG, branch `klbf-amd` (RX 9070 XT):

- A shader stuck in the AMD compiler (`39f0e9aeb156a61c`) is skipped by default — a hash-keyed
  skip list, shipped as `docs/klbf/skip-amd.txt` and fed to `VICTORY_SKIP_PIPELINES`.
- 32-lane pixel shaders run per half wave on 64-lane hardware produces purple blocks.
- NaN/Inf filter on colour outputs.
- Larger, configurable texture upload queue, for late or missing textures.

The first two have direct TuxBox counterparts: `docs/SHADERS.md` ("64-lane programs on 32-lane
hosts") and the shader preparation cache. On Linux the compiler is RADV's, not `amdxc64.dll`, so
the skip list itself does not transfer; the mechanism does — a per-build deny list of shader
hashes that fail to compile, honoured by preparation and by translation during play.

Action: confirm whether preparation already records and skips a hash that fails to compile
(`project_tuxbox/runtime_shaders.py` `ShaderRefused` suggests it does), then decide whether a
manual deny list adds anything. Their Proton users reach the world on an RX 7800 XT and RX 6900
XT (`docs/GUIDE.md:119`), which is a useful expectation for this card.

## BR-20 — Runtime HLSL against Wine's `d3dcompiler_47`

Status: `verify`.

`gmanka/victory` README:653 and 673 record that under Wine and Proton `d3dcompiler_47` is
vkd3d-shader, which rejects `RWByteAddressBuffer` methods and, where it does compile, drops the
root signature the shader asks for, so the indirect clamp, the card-side compares and the depth
twins silently switched off. Their fix routes that source to `dxcompiler.dll`.

TuxBox already requires the Windows x64 DXC DLLs beside the game (`docs/PROJECTSCARLETT.md`, and
forwards `DxcCreateInstance` through `runtime/dxc`), so this is likely covered. Verify that every
runtime HLSL compile in the provider goes through DXC and none falls back to the system compiler.

## BR-21 — Constant buffers: the card reads the title's bytes where they are

Status: `already-fixed`, plus one thing that cannot transfer.

`avatarmanz/VIctory-7900XTX` README (2026-11-26) measures four renderer patches, of which two are
about constant buffers:

- `VICTORY_PORT_CONSTANTS=1`: the title's 174 MB constants heap is read by the card where the
  title writes it — no uploads, no fills.
- `VICTORY_PORT_CBV=1`: translated shaders read constant buffers in place as raw bytes,
  "~22,000 fills a frame to none; -4.7 ms".

TuxBox binds the guest address directly rather than filling a copy:
`runtime/d3d12/src/commands/compute.rs:143` (`constant_buffer(list, index, guest)`), and
`docs/SCARLETT_ROOT_CBV_WINDOW.md` records a regression control that reads slot 41 straight out of
CPU-backed committed guest pages. So the fill elimination is already our design, and the 22,000
figure is useful only as an order-of-magnitude for what a fill-based implementation would cost.

The part that cannot transfer: their "no uploads" version depends on the card reading the title's
own pages. On RADV the pages are not importable (`docs/USAGE.md`, memory copy), which is exactly
the cost recorded in BR-1 and the reason a private mirror exists. Their measurement is therefore
an upper bound we cannot reach on this host, not a target.

## BR-22 — Write-guard lock granularity

Status: `open`. Plausible win on the mirrored path.

`avatarmanz/VIctory-7900XTX` README: `VICTORY_PORT_GUARD=1` makes "the write guards' lock shared
by faults and refreshes, with a claim per 64 KiB block; the submission's planning runs beside it
(title threads' waits 29 ms to ~2 ms a frame; -2 ms)". Their commit `cc761bf` and `9d723e8`
describe the same change from both sides.

TuxBox's mirror keeps one `Mutex<ShadowState>` per shadow
(`runtime/d3d12/src/device/heap/shadow.rs:39`) and takes it for the whole diff/copy in
`sync_cpu_writes` (line 109), so a submission syncing a large shadow serialises every other user
of that shadow for the duration.

Action: claim per count, not per shadow — divide the shadow into 64 KiB blocks with a state per
block, so a fault and a refresh on different blocks proceed together. Verify on the same host
with two 64 MiB shadows driven from two threads and assert overlapping blocks only serialise
against themselves. This one is host-testable in the parts that do not need a device.

## BR-23 — Descriptor-table stores versus a whole-heap scan

Status: `not-transferable`, useful as a cost model.

`VICTORY_PORT_DESCRIPTORS=1` patches the title's descriptor-table stores so they report the slots
they write, "instead of a scan of ~600,000 slots a frame". That works because that project patches
the title's code. TuxBox does not patch the game, so the option does not exist here.

It does quantify what BR-2 is up against: a scan-based descriptor path is worth about a frame's
tail on an RX 7900 XTX. If our per-view publication ever shows up in a profile, that is the number
to compare against.

## BR-24 — Output size, render scale and the 30 fps cap

Status: `open` (product decision, low priority).

Same README, "Other switches": `VICTORY_OUTPUT=1920x1080` makes the title render to that size
itself ("about 16 ms less card time"), `VICTORY_RENDER_SCALE` and `VICTORY_FFX_SCALE` drive its
upscalers, `VICTORY_FRAME_INTERVAL_MS=16.667` lets the title pass 30 fps ("menus and videos reach
60; the game's clock stays right"), and `--console series-s` gives 1080p output for about 5 ms.

TuxBox's nearest pieces are the Series S graphics trial (`--graphics-menu`, `--graphics-profile
series-s`, `docs/USAGE.md`) and the packaged settings. There is no output-size or frame-interval
knob.

Action: none now. Worth revisiting only after the title reaches a picture, because a smaller
output is the cheapest large win on an RX 6700 XT and does not depend on fixing anything else.
See BR-25 for the cost model that ranks these levers.

## BR-25 — What a frame actually costs, and which settings reduce what

Status: reference. Not D3D12-specific, not platform-specific: it applies to any host that keeps a
second copy of title memory.

Recorded here because "lower the NPC density and water quality to 30 %" is a recurring idea, and
the answer is that it only touches half of the cost. The numbers are from
`avatarmanz/VIctory-7900XTX` `docs/findings.md` (RX 7900 XTX, Windows) and were reproduced in
substance by the other AMD forks.

Where a 55 ms frame goes on the water, first boat mission:

- The thread that prepares submissions: 48 ms, the wall. Inside it: list steps 18, refreshes of
  host copies 10, steps done on the card 7, waiting for descriptor records 6, asking the write
  watch 5, copies in the card's memory 4, the title's bytes under shader writes 4.
- The card: about 41 ms busy.
- List calls: 36,000 a frame, 230 ms of work across eight threads; a street chase with many cars is
  67,000 calls and 430 ms.
- Memory: 40-46 GB private, of which lists' allocators 9 GB, windows 7 GB, uploads 4 GB.

What scales with scene density:

- Draw count, therefore per-call translation, descriptor records and sync steps. Cutting density
  attacks the dominant CPU cost roughly in proportion, because it is per-call work.
- The title's own simulation thread, which is a smaller share than the layer's overhead.

What does not scale with density:

- The layer's per-submission memory work: the mirror copies, the write-watch scans and the
  descriptor publication run per frame regardless of how many people are on screen. This is the
  cost that BR-1, BR-12 and BR-22 are about.
- Its own allocation footprint (lists' allocators, windows, uploads), which is bookkeeping.
- VRAM held by render targets and ray-tracing structures.

Levers ranked by measured effect, for a 12 GB card:

1. **Output size.** The title rendering at 1920x1080 instead of 4K: about 16 ms less card time
   (`VICTORY_OUTPUT`). Also the largest VRAM saving, since every render target shrinks.
2. **Series S profile.** 1080p output with half-scale textures, about 5 ms on top of the above
   (`--console series-s`, or TuxBox's `--graphics-profile series-s`).
3. **Ray tracing off.** Removes the acceleration structures entirely; the install carries 7.8 GB of
   `pack/core_bvh4` for that path alone.
4. **Render scale** (`VICTORY_RENDER_SCALE`, `VICTORY_FFX_SCALE`): same family as 1, less well
   measured, and their README marks the FidelityFX variant "not safe".
5. **Scene density**, last: it is a title-internal value with no exposed knob in the console build.
   Changing it means patching the game's code, which the Windows forks do and TuxBox does not.

Consequence for TuxBox: the knobs we can actually offer are those in BR-24 (output size, Series S,
ray tracing off), because they are launch-side. Density and water quality are not ours to set. And
the reason a density reduction would not rescue a TuxBox frame is the same reason BR-1 exists: the
mirror cost per submission does not care how crowded the street is.

## BR-26 — A Windows fork builds and binds on this host under Wine

Status: `finding`. Recorded because it changes where the next hour of effort should go.

Tested 2026-11-27 on this host (`1002:73DF`, 32 GB, GE-Proton11-14). The subject is
`Belobug/VIctory-Belobug-AMD` `rx9070xt` at `81ebb7b` — a GPL-3.0 Windows tree. It was built and
executed here; no source was copied into TuxBox (licence: ideas and measurements only).

**It builds.** `cargo xwin build --locked --release -p victory --target
x86_64-pc-windows-msvc` finished in 1m40s with the toolchain from the build-requirements work
(rustup 1.98.1 from the repo's `rust-toolchain.toml`, cargo-xwin, the staged xwin SDK).
Output: `victory.exe`, 9,889,280 bytes, machine `0x8664`, subsystem 3 (console), 456 MB target
directory, 13 compiler warnings.

**Bare Proton wine does not start it.** `files/bin/wine ./victory.exe` fails before `main`:

    err:module:import_dll Library libvkd3d-1.dll (needed by wined3d.dll) not found
    err:module:import_dll Library dxgi.dll (needed by victory.exe) not found

Exit 53 (`ERROR_BAD_NETPATH`). The vkd3d DLLs live in `files/lib/vkd3d/x86_64-windows`, which
Proton's own launcher adds to the search path and a bare wine invocation does not. Either
`WINEDLLPATH=<proton>/files/lib/vkd3d/x86_64-windows` or `umu-run` works; with it, the CLI,
`config` and `inspect` all run.

**It binds this host's game executable.** `victory.exe inspect games/game_scarlett_master.exe`
reported BLAKE3 `b99f876b74d4a77f03af5f878223b27bd647b5c35a79d5decd9f716504239e93`, PDB
`X:/americas/src/sub_story/game/vs_project/Gaming.Xbox.Scarlett.x64/Build/master/game_scarlett_master.pdb`
(key `2A6912BAEB4F427F4C4C44205044422E1`), 10 sections, entry `0x56a1ec8`, 2 TLS callbacks.
`victory.exe run <exe>` mapped it, printed its per-import HLE coverage (Bink, XCurl and the rest
"missing" only because the packaged DLLs were not beside it) and stopped at
`not executing (pass --execute to start the title)`.

**Its host gates are met here.** `crates/victory-gpu/src/host.rs:3904-3926` requires
`D3D_SHADER_MODEL_6_6` and `D3D12_OPTIONS7.MeshShaderTier != NOT_SUPPORTED`. Our own probes
under GE-Proton11-14 on this card:

    d3d12-features PASS pci=1002:73df queries=7 binding=3 heap=2 root=1 dxr=0 allocations=4
    d3d12-mesh     PASS pci=1002:73df tier=1 stream=680 direct=6 amplified=12 indirect=2 zero=0

So mesh tier 1 is available through vkd3d on RADV here, and `dxr=0` says ray tracing is not.

**What is left is footprint, not portability.** Published figures, not measured here:

| | TuxBox | Belobug | avatarmanz |
| --- | --- | --- | --- |
| Private memory | ~15 GB plus swap | ~22 GB | 40-46 GB |
| Video memory | no stated floor | "more than a 16 GB card has" | 16-24 GB |

On 12 GB of VRAM the Belobug tree is already over budget and the avatarmanz tree would swap.

**Consequence.** The shortest path to a picture on this machine is a Windows tree under Wine with
`--console series-s` (1080p, half-scale textures), not waiting for TuxBox's remaining ~4,800
untranslated shader requests. TuxBox stays the only Linux-native vehicle and its open items stay in
this file. Reproduce with:

    cargo xwin build --locked --release -p victory --target x86_64-pc-windows-msvc
    export WINEPREFIX=... WINEDLLPATH="$PROTONPATH/files/lib/vkd3d/x86_64-windows"
    "$PROTONPATH/files/bin/wine" ./victory.exe run ./game_scarlett_master.exe

Do not commit their sources or binaries into this repository: GPL-3.0-or-later against our
Apache-2.0.

---

## Verification constraints on this host

- **`tuxbox-d3d12` cannot be tested here at all.** `cargo test -p tuxbox-d3d12` fails before
  reaching our code: dependency `windows-future 0.2.1` does not build for
  `x86_64-unknown-linux-gnu` (`cannot find type IMarshal in windows_core::imp`). The repo
  reflects this: `project_tuxbox/check.py` runs only
  `cargo check --target x86_64-pc-windows-msvc -p tuxbox-d3d12`. Any change in that crate made
  on this machine is compile-checked, never executed.
- Host-runnable suites: `tools/shader-compiler` tests, `tuxbox-dxbc --features native-bindings`,
  `tuxbox-xg-layout`, and the Python tests under `tests/`.
- Anything needing the GPU needs the owner to launch it, one GPU client at a time (the amdgpu
  panics recorded on 2026-11-21).
- The GTA VI package on this host is incomplete (`game_scarlett_master.exe` missing, 15 GiB of
  208 GiB present), so no run-based item can be closed until the download finishes.
