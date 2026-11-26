# LOGBLOCK — the questions that only a log can answer

This file lists every open question whose answer is a line from a real run. It does not
repeat `BUG-RECON-TODO.md`. Each entry names the `BR-n` or `docs/FOCUS.md` item it closes,
the switch that produces the line, and the line to keep.

An entry stays in this file until the line is in hand. Then record the result in its
`BR-n` entry and put the receipt under `evidence/`.

## Rules

- Put `--log-file <path>` before the game path. `run` saves no log otherwise
  (`docs/USAGE.md:49`).
- Record the run conditions beside the log: package version, GPU PCI id, Proton version,
  host profile, and any `TUXBOX_*` override.
- Run one GPU client at a time. A second client panics amdgpu on an RX 6700 XT.
- Start the title only with `tuxbox.py run`. It keeps ProjectScarlett offline. The title's
  crash handler uploads reports otherwise.
- Do not share a capture folder. These logs contain the title's own data.
- Do not write an evidence file for a run that did not happen.

## Every run waits for the download

The package holds holes. The layout preallocates each RPF. A partial download therefore
reports the full apparent size, and the holes read as zeros with no error.

```sh
du -sh /home/fih/bulk/media/downloads/GTA6.Xbox.v1.0.2.1-CYBERLEEK    # 31G
du -sh --apparent-size /home/fih/bulk/media/downloads/GTA6.Xbox.v1.0.2.1-CYBERLEEK  # 209G
```

Recorded on 2026-11-27. Every entry that needs the title stays blocked until these two
figures agree. Lanes runs are not blocked.

## A. Instrument first, then log

These four questions have no log line today. Add the line, then run once.

### LB-1 — How large does the title's indirect dispatch get?

Closes BR-3 and BR-14.

- Today: the provider prints `recorded ExecuteIndirect list=... kind=... max_count=...`
  once per kind (`runtime/d3d12/src/commands.rs:526`). It shows `max_count`, not the value in the
  count buffer, and it misses every call after the first.
- Add: a line per call whose `max_count` or count-buffer value passes 65,535.
- Also read the count buffer. The count value decides the real group count.
- Closes when: you record the largest group count of a run, and name the host behaviour
  above 65,535.

### LB-2 — What does vkd3d report for `MaxViewDescriptorHeapSize` on RADV?

Closes BR-13.

- Today: `runtime/d3d12/src/device/resources.rs:172-178` queries
  `D3D12_OPTIONS19` and discards the answer. The code substitutes 1,000,000.
- Add: one log line with the returned `MaxViewDescriptorHeapSize`, the requested
  `NumDescriptors`, and the limit actually used.
- Closes when: you record the figure and compare it with the 1,000,000 default. The
  decision on the AMD default follows from that number.

### LB-3 — Does the mirrored copy cost too much on RADV?

Closes BR-1 and BR-22.

- Today: `log_first_mirror` (`runtime/d3d12/src/device/heap/shadow.rs:274`) says the path
  is active. No per-submission time exists.
- Capture on a run whose host profile reads `section-import=substituted` or `refused`:

```sh
python tuxbox.py run --log-file build/mirror.log --mangohud <game>
python tools/sample_process_cpu.py <pid> --seconds 30 --output build/mirror-cpu.jsonl
```

- Keep: `summary.json`, the frame CSV, the host profile line, the CPU samples.
- Closes when: you record the mirror share of a frame with those run conditions.
- Attribution needs the added timing. The MangoHud totals alone name no cause.

### LB-4 — Which queue stopped, and on what?

Closes BR-17 and the stall in `docs/FOCUS.md:43`. It feeds BR-15 and BR-16.

- Today: `queue_trace` prints `queue {addr} type {n} passed {done} of {mark} operations`
  when a memory wait stalls (`runtime/d3d12/src/commands/queue_trace.rs:111`, called from
  `memory_wait.rs:957`).
- Capture: the whole report block from a stalled run, with the `GPU memory wait released`
  lines that follow it.
- Closes when: a stall names the operation each queue stopped on, on RADV. The NVIDIA half
  needs another card.

## A2. Shader coverage — one session answers four buckets

The refusals in BR-27 and BR-28 split into four groups. A run that carries these four lines turns
them into a coverage report. The tracer is the work in front of the run.

Each line belongs beside the existing request record in `project_tuxbox/runtime_shaders.py` and in the
provider's shader cache, so one `--log-file` captures all of them.

### LB-17 — Did the mesh root recording fire?

Closes the mesh bucket of BR-27, 1,551 refusals.

- Change: `compile::record_source` (`runtime/d3d12/src/device/shader_cache/compile.rs`) counts each
  recorded rooted mesh program and prints the total once at exit.
- Keep: the count of `mesh_link_source` calls that had a root, the count recorded, and the
  `mesh root signature missing` lines from the preparation phase.
- Closes when: the recorded count is not zero and the refusal count for that reason drops. A run with
  zero recorded is the more useful result, because it says the recording path never ran.

### LB-18 — What do the refused stores look like?

Closes the store bucket of BR-27, 577 refusals.

- Change: on each `buffer_store_format to a non-UAV table` and `image_store to a non-UAV table`
  refusal, print the program digest, the slot, the descriptor's origin (heap or root), and
  descriptor word 1 (the V#).
- Keep: those lines for one session, with the refusal totals beside them.
- Closes when: the lines name what share of the bucket is heap and what share is root. That decides
  which half can be finished on this host. BR-28 records why the answer is host-side work.

### LB-19 — Which descriptor stride does a refused `IDXEN` load need?

Closes the `IDXEN` bucket of BR-27, 129 refusals.

- Change: at the `raw buffer IDXEN needs the console descriptor stride` refusal
  (`tools/shader-compiler/src/hlsl/emitter/buffer.rs:70`), print the slot and descriptor word 1.
- Keep: the stride field from every refused program.
- Closes when: the recorded strides can be compared against the V# values. Victory-United reads the
  same field from the host (`SLOT_TABLE_STRIDES`, BR-28), so a disagreement between our decode and
  theirs is a finding.

### LB-20 — What does the large-program set cost to compile?

Closes the wave-reduction bucket of BR-27, 1,502 refusals.

- Artifact: the request archive from a preparation run. That archive already keeps each refused
  program, so this item adds no switch.
- Then measure here, with the `dxc` on this host, the DXIL size and the compile time of each large
  program. The docs' trial reports 2-12 s and 1-2 MB per sample. A 1,502-program set multiplies that.
- Closes when: the archive exists and the per-program figures are in hand. The decision on the
  6,000-instruction limit follows from those figures, not from the refusal count.

## B. Switches that already exist

Each of these needs one run with a documented switch.

### LB-5 — Which path did this run take?

Closes the preconditions of every other entry.

- Line: `host profile: adapter=1002:73df wave=... section-import=...` (`docs/USAGE.md:24`).
- Keep it in every log. It decides whether the mirror, the paging arena or the direct
  import was in use.

### LB-6 — Do wave reductions go unmasked on RADV?

Closes BR-5.

- Switch: `TUXBOX_WAVE_DEBUG=1` (`docs/ENVIRONMENT.md:88`). It names why the translator
  did not mask a reduction.
- Look at shadow map edges for square holes in the same scene.
- Closes when: you record the reason lines and a screen capture of the edges. This host
  has no NVIDIA card, so the comparison half stays open.

### LB-7 — Did a GPU memory wait give up?

Closes the open item in `docs/PROJECTSCARLETT.md:108`.

- Line: `GPU memory wait released after {}s without reaching its target (#{n}):
  address={a} op={op} target={t} value={v}` (`runtime/d3d12/src/commands/memory_wait.rs:949`).
- Keep: the first eight lines and each power of two after them, with the addresses.
- Closes when: you record each occurrence in a retail run with its address. These are
  known compatibility problems, not noise.

### LB-8 — Why do pipelines fail, ranked by cost?

Closes the ranking half of BR-27 and `docs/FOCUS.md:32`.

- File: `tuxbox_shader_cache/pipeline-failures/<run>.tsv` in the prefix's temporary folder.
- Then:

```sh
python tools/rank_pipeline_failures.py "$PREFIX/drive_c/users/steamuser/AppData/Local/Temp/tuxbox_shader_cache"
```

- Closes when: you record the ranking of a retail run. The ranking holds no paths or
  shader data, so it can be shared.

### LB-9 — Where does a cold and a warm start spend its time?

Closes `docs/FOCUS.md:9` and the memory point at `docs/FOCUS.md:11`.

- Switch: `--profile-pipelines`, which writes `timeline.json` beside the log.
- Then:

```sh
python -m project_tuxbox.pipeline_profile build/timeline.json
```

- Also record `--prepare-only` wall time for the same cache, cold and warm.
- Keep: the timeline, the summary, the prep times, peak memory and swap use.
- Closes when: you record per-stage time for one cold start and one warm start.

### LB-10 — What runs at the frame-time spikes?

Closes `docs/FOCUS.md:28`.

- Switch: `--mangohud`. It writes a frame CSV and `summary.json` (`docs/USAGE.md:55`).
- Keep: the p95 and p99 frame times and the spike timestamps from the CSV.
- Closes when: you record the spike pattern with the frame numbers it repeats at.

### LB-11 — Which bindings cause `Failed to remap`?

Closes `docs/FOCUS.md:33`.

- Switch: `VKD3D_DEBUG=warn`. The line comes from dxil-spirv inside vkd3d-proton.
- Keep: every `Failed to remap` line with the shader digest it follows.
- Closes when: you name the binding kinds behind the message.

### LB-12 — Does the pipeline cache stay warm over a long session?

Closes `docs/FOCUS.md:16`.

- Record the size of `<prefix>/shadercache` before and after a long run.
- Then count repeat compiles of the same cache key in the log.
- Closes when: you record cache growth and the repeat-compile count. The 5 GB index
  failure is an NVIDIA result, so it does not transfer to this card.

### LB-13 — What does one real session read from storage?

Closes step 0 of the storage spec
(`docs/superpowers/specs/2026-11-26-storage-staging-design.md:288`).

- Switch: `TUXBOX_TRACE_STORAGE=<path>` (`runtime/xcrdapi/src/queue.rs:141`). It writes
  `micros<TAB>path<TAB>offset<TAB>length<TAB>raw|zlib` per read.
- Needs: the complete package and one session that reaches gameplay.
- Then: `tools/storage_plan.py` reads the trace. That tool is not written yet.
- Closes when: a trace of a real session exists and you measure the read shapes.

### LB-14 — Do the two-lane translations hold in a title?

Closes `docs/FOCUS.md:34`.

- The synthetic tests pass. No game verified the output.
- Capture: a frame that runs a 64-lane program, on this card.
- Reference: the same frame from a console capture or from another GPU. This host has one
  card, so the reference is missing as well as the run.
- Closes when: the card's output matches the reference, or you record the difference.

## C. Receipts that need no title

### LB-15 — Descriptor reclamation on this GPU

Closes BR-10.

- `evidence/` holds this receipt only for `1002:7590` (RX 9060 XT). This host is
  `1002:73df` (RX 6700 XT, RADV).
- Build `bvh` first. `python build.py lanes` does not build it (BR-9).
- Run:

```sh
python build.py bvh
python build.py lanes
python tuxbox.py check --only lanes-selftest --output build/lanes-selftest-73df.json
```

- Line to keep: `lanes-selftest PASS pci=1002:73df scenario=descriptor-reclaim passed=1`.
- Closes when: that line is in the receipt. A failure is a difference between AMD
  generations, and it is the more useful result.

### LB-16 — Does the section commit cost show under Proton?

Closes BR-12.

- The code change comes first. Then time a warm start under Proton, before and after.
- Keep: start time to the first presented frame, both runs, same prefix and caches.
- Closes when: you record the difference. The flag-selection helper alone proves nothing
  about start time.

## D. Not log items

Do not wait for a run on these.

- BR-7, BR-15, BR-16, BR-19, BR-20: read the code and write the answer. This tree answers BR-7 and
  BR-19.
- BR-27 item 1, the mesh root recording, landed here (`4bdb158`). Its log line is LB-17.
- BR-27 item 2, the `IDXEN` stride, is offline work with a log line at LB-19.
- BR-27 item 3, the wave limit, needs the archive at LB-20 before any decision.
- BR-9: build-order bug, fixed here (`37f20bd`).
- `docs/FOCUS.md:24`: the black screen needs a screen capture, not a log.
- `docs/FOCUS.md:21`: the audio stall needs the failing USB device attached, and the
  kernel log needs `systemd-journal` membership. Check both before promising a result.
