# Storage staging for ProjectScarlett and other streaming titles

Status: draft for review. Date: 2026-11-26.

## Problem

A title's install does not fit on the machine's SSD. GTA VI 1.0.2.1 declares 208 GiB of
files; the host SSD has 2.7 GiB free. The install therefore lives on a rotational disk, where
a cold 64 KiB read costs about 12 ms — long enough to drop a frame — and the SSD holds only a
working-set stage.

The goal is that the title keeps its install on the HDD, a bounded region of the SSD holds the
parts the title actually reads, and memory holds what is hot. Both effects count: play must not
hitch, and loading should approach what an SSD install gives.

## Scope

In scope:

- A precomputed stage plan, learned from a recorded session, ordered by first use.
- A stage process that copies planned extents from the install to the SSD, staged during
  launcher setup for the boot set and in the background for the rest.
- A read path that the title sees as a normal directory, served from the stage when present and
  from the install otherwise.

Out of scope for the first version:

- Block-level or filesystem-level caching (bcache, dm-cache, LVM). Those need root, a dedicated
  partition, and cannot follow one title's lifecycle.
- Eviction between sessions. The plan decides what is staged; the stage fills to its budget and
  stops.
- Windows. This design targets the Linux backend.
- Compiling or transcoding assets. The stage copies bytes, never rewrites them.
- Several titles staging at once.

## Measured constraints

All measurements were taken on this host. The HDD figures use `O_DIRECT`, so they exclude the
page cache.

| Quantity | Value | Source |
| --- | --- | --- |
| HDD sequential read | 67.9 MB/s | 256 MiB `dd` read of `/home/fih/bulk/vmdata/sandbox/sandbox.vdi` |
| HDD 64 KiB random read | 12.0 ms median, 18.5 ms p90 | 24 `pread` calls at random offsets on the same file |
| SSD 64 KiB random read | 0.41 ms median, 0.62 ms p90 | 24 `pread` calls at random offsets on a cached NVMe file |
| Frame budget at 60 fps | 16.7 ms | definition |
| Staging rate | about 15 s per GiB | HDD sequential read above |
| Install size, GTA VI 1.0.2.1 | 208 GiB apparent | `du --apparent-size` over the package |
| Free space, SSD | 2.7 GiB of 234 GiB | `df` on `/dev/nvme0n1p2` |
| Free space, HDD | 251 GiB of 932 GiB | `df` on `/dev/sda5` |
| Memory | 32 GiB total | host |

Two consequences follow, and they shape everything below:

1. A cold read from the HDD, and any copy from the HDD, costs about one frame per 64 KiB. The
   read path must not trigger a copy while a read is waiting.
2. The whole stage cannot be filled before the title starts. At 67.9 MB/s, 40 GiB takes about
   ten minutes. The stage must fill in order of first use, ahead of the title.

## What the title reads

GTA VI packages its streaming data as RAGE package files. Two properties matter:

- An `.rpf` file holds a directory of entries at 512-byte granularity, and a streamable entry is
  an `RSC7` resource: a header plus system and graphics segments sized by packed page flags
  (`baseSize = 0x200 << ss`). The engine selects a resource and reads the whole compressed entry,
  then inflates it and relocates its pages into memory. It does not issue sub-resource scatter
  reads.
- Reads reach TuxBox through `xcrdapi.dll` as file-to-memory requests with an explicit 64-bit
  offset and length, either raw or zlib. TuxBox's own captured evidence for the other engine in
  the repo (Frosty, Unreal IoStore) shows 17,094 to 26,067 compressed bytes decoding to 65,536
  bytes in single requests.

So the request is the correct cache unit. For RAGE it is one resource; for Unreal it is a 64 KiB
block. Caching below the request size buys nothing, because the engine already chose the size.
Caching above it (whole files) wastes the budget: `levels.rpf` alone is 72.6 GiB.

### Package inventory, GTA VI 1.0.2.1

Declared sizes. `Identity RockstarGamesInc.28872443D20A7`, `Version 1.0.2.1`,
`TitleId 7952987A`, executable `game_scarlett_master.exe`, application id `GTAO`.

| Path | Size | Expected read shape |
| --- | --- | --- |
| `levels.rpf` | 72.6 GiB | streaming working set, high reuse while an area is active |
| `data/ttdb/` | 56 GiB | world database, reuse depends on area and camera |
| `audio.rpf` | 27.2 GiB | streamed by scene, effectively one-pass |
| `movies.rpf` | 8.4 GiB | cutscenes, strictly one-pass |
| `navmeshes.rpf` | 6.4 GiB | loaded once, then resident in memory |
| `packs.rpf` | 4.6 GiB | tables and packs, reread at load |
| `pack/core_bvh4/` | 7.8 GiB | ray-tracing structures, unused without ray tracing |
| `pack/story000/` | 20 GiB | chapter data; packaging is already partitioned per chapter |
| `pack/*_restofworld/` | 1.5 GiB | other regions' content |
| `shaders.rpf` | 393 MiB | reread constantly, small |
| `anim.rpf` | 4.2 GiB | streamed |
| `data.rpf`, `common.rpf` | 467 MiB | boot tables |
| `textures.rpf` | 184 MiB | reread at load |

The expected shapes are hypotheses about the engine, not measurements. The first recording turns
them into measurements, and the plan tool must not hard-code them as truth: they become default
exclusions with an override, not a fixed policy.

## Decisions

**D1 — The cache unit is the recorded request.** Key is `(file, offset, length)` as the title
submitted it. Rationale: section "What the title reads".

**D2 — The plan carries ordering and frequency, not membership alone.** Each extent carries its
first-use time in the recording and its read count. Ordering is what makes a 40 GiB stage usable
at 67.9 MB/s.

**D3 — The plan is advisory.** The title can read ranges the recording never saw. Those reads go
to the install and are served normally. A miss also advances the stage pointer, because a miss
means play is ahead of the plan.

**D4 — The stage process is separate from the title.** A daemon owns the stage file and the
policy; the title sees a mount. The launcher starts both and tears both down.

**D5 — The read path is a FUSE mount over the install tree.** This is the mechanism a symlink
cannot provide: per-range mapping, swap under open handles, and partial staging. Cost is the
FUSE round trip, tens of microseconds against a 12 ms read it replaces.

**D6 — The stage is a sparse mirror.** For each source file there is a file of the same size at
the same relative path under the stage root, with staged extents written at their original
offsets. No lookup table is needed to find data; the extent's own offset is its location. Only
written blocks consume space, so the budget is the sum of allocated blocks.

**D7 — Presence is an in-memory bitmap.** At mount the stager builds it by walking the mirror
with `SEEK_DATA`/`SEEK_HOLE`. A read is a hit when every block covering the range is present.

**D8 — The boot set is staged during launcher setup, before the title starts.** "Boot set" means
every extent whose first use falls before the first presented frame, plus a configurable margin
of recorded play after it. This mirrors the existing shader preparation step, including
`--prepare-only`, progress output and cache reuse across runs.

**D9 — Memory is the page cache in the first version.** The mirror is read continuously, so hot
extents stay resident without an allocator. A pinned tier keyed on recorded read counts is a
later step, taken only if measurements show SSD reads dominating.

**D10 — Plan validity is a hard check.** The plan names the identity, version and size of every
file it references. A mismatch means the plan is ignored, the session runs unstaged with the
trace on, and the next recording replaces the plan.

## Architecture

    launcher (tuxbox.py run)
      |
      +-- stager daemon -- stage root + plan ---> FUSE mount /run/user/<uid>/tuxbox-<title>/
      |        |                                        |
      |        +-- background: copy planned extents     +-- read(offset, len)
      |            from install to mirror                    |
      |                                                      +--> mirror hit: pread
      +-- game process, package root = the mount               +--> miss: pread install, bump plan

Components, each with one job:

- **Recorder** — in `runtime/xcrdapi`. Writes one line per file read when the trace variable is
  set. Implemented in this change.
- **Plan tool** — `tools/storage_plan.py`. Reads a trace, deduplicates, orders by first use,
  splits boot and stream sets, drops excluded files, writes plan JSON. Pure function of its
  input.
- **Stager** — new process. Builds the presence bitmap, fills the stage in plan order, serves the
  FUSE mount, reports counters.
- **Launcher integration** — starts the stager, waits for the boot set, passes the mount as the
  package root, tears both down on exit.

## Trace format

Implemented in `runtime/xcrdapi/src/queue.rs` (`trace_read`) and `lib.rs` (`FileBacking::name`).
Enabled by `TUXBOX_TRACE_STORAGE=<path>`; inert otherwise. The launcher copies `os.environ` into
the Proton environment, so the variable reaches the guest without extra plumbing.

One line per read, tab separated, appended, no header:

    <microseconds since provider load>\t<path as opened>\t<offset>\t<length>\t<codec>

`codec` is `raw` or `zlib`. The path is the Wine path the title passed to `OpenFile`. Verified
against a real Proton guest: a fixture run produced
`0  xcrdapi-fixture.bin  16  24  zlib` and five further lines, and the provider's own check
still passes (`raw=16 zlib=16 guards=2 errors=2 cancellation=1 autosubmit=64 PASS`).

Limits to remember when reading a trace: it records submission order per worker, not completion
order; several queue workers interleave lines; and the first-use time is microseconds since the
provider loaded, not since the process started, so a separate marker is needed to align with
presented frames. The recorder does not yet write such a marker — see open question 3.

## Plan format

JSON, schema 1. Written by the plan tool, read by the stager.

```json
{
  "schema": 1,
  "title": {
    "identity": "RockstarGamesInc.28872443D20A7",
    "version": "1.0.2.1",
    "title_id": "7952987A",
    "app_id": "GTAO"
  },
  "files": [
    {"path": "levels.rpf", "size": 77986335232, "recorded": true},
    {"path": "movies.rpf", "size": 9025011712, "recorded": false, "excluded": "one-pass sequential"}
  ],
  "boot_set": [
    {"file": 0, "offset": 12345, "length": 65536, "first_use_us": 1200, "reads": 1}
  ],
  "stream_set": [
    {"file": 0, "offset": 79361, "length": 65536, "first_use_us": 20400, "reads": 3}
  ]
}
```

Rules:

- `files` indexes extents. `size` must match the staged file, or D10 applies.
- Both sets are sorted by `first_use_us`.
- `reads` counts submissions of that exact extent in the recording. It orders the future pinned
  tier and nothing else in version 1.
- `excluded` files appear in `files` with no extents.

## Staging layout

    <stage root>/<title id>/mirror/<relative path>     same size, same offsets
    <stage root>/<title id>/plan.json                  the plan in use
    <stage root>/<title id>/stats.json                 counters from the last run

Budget is `TUXBOX_STAGE_BUDGET`, default 32 GiB, enforced at copy time. When the budget is
reached, staging stops. Nothing is evicted and nothing fails.

Filesystem: the stage root must be on a filesystem where a sparse file costs only its written
blocks (ext4, XFS, Btrfs). The stager must check free space before each copy batch and stop
rather than fill the device.

## Read path

For `read(offset, length)` on a mirrored file:

1. Every 4 KiB block covering `[offset, offset+length)` is present: `pread` the mirror.
2. Otherwise: `pread` the install file, and queue the same extent plus the next
   `TUXBOX_PREFETCH_AHEAD` stream-set extents for staging.

Counters per run: hits, misses, bytes staged, bytes served by tier, stage fill, stager copy rate.
These go to `stats.json` so a session can be compared against the recording.

Staging order, then, is: the boot set at setup, in plan order; then the stream set in plan order,
one worker by default because a rotational disk loses throughput to random access and the copy
must not starve the title's own reads. `TUXBOX_STAGE_THREADS` allows more on an SSD install.

## Lifecycle

1. Launcher reads the plan for the target; D10 check.
2. Stager starts, creates or verifies the mirror, builds the bitmap, mounts FUSE.
3. Boot set copied with progress, interruptible, skipped when already present.
4. Title starts with the mount as the package root; stream set copies in the background.
5. Title exits: stager finishes nothing, unmounts, writes `stats.json`, exits.
6. Stager exits first: the launcher treats the session as failed and reports it, because the
   title will see `EIO` from the mount.

Persistence: mirror and plan survive across runs, so the boot set is paid once. A changed
identity, version or file size invalidates the mirror and the plan.

## Failure modes

| Failure | Behaviour |
| --- | --- |
| Plan missing or stale | Run unstaged, trace on, record, build a new plan |
| Stage filesystem full | Stop staging, keep serving; report in `stats.json` |
| Stager exits mid-session | Launcher reports failure; title may see `EIO` |
| Mirror extent corrupt | zlib inflation in `xcrdapi` fails loudly; the read is retried from the install after re-staging |
| Mount invisible to Proton | Add the mount to `PRESSURE_VESSEL_FILESYSTEMS_RO`, as the constant-buffer layer already does |
| Unprivileged user namespaces unavailable | Offline launches already stop; this design adds no new requirement |

## Testing

- Plan tool: fixture traces covering duplicate extents, overlapping extents at different lengths,
  a file with no reads, and a truncated final line. Assert the plan is sorted, deduplicated and
  budget-independent.
- Read path: mount a fixture tree, read a random sample of ranges through the mount, and compare
  SHA-256 against the same ranges read from the source. Include ranges that straddle a
  staged/unstaged boundary and ranges at end of file.
- Budget: assert the stager stops at the configured bytes and that the bitmap matches `SEEK_DATA`.
- Version lock: a plan whose `size` differs must be refused, and the session must still run.
- End to end: one recorded session, then a staged session with `--log-file` and `--mangohud`;
  compare median and p99 frame times and total load time against the unstaged recording.

## Milestones

0. Recorder in `runtime/xcrdapi`, plus a trace of a real session. Done in this change; the
   session is outstanding.
1. `tools/storage_plan.py` with tests, run against the first real trace.
2. Stager with sparse mirror, bitmap and FUSE read path, no plan; verified by hash comparison.
3. Plan-driven boot-set staging in the launcher, background stream set, miss prefetch.
4. Measure, then decide on the pinned memory tier.

## Open questions

1. **Boot-set threshold.** "Before the first presented frame" may itself exceed the budget for an
   open-world title. If it does, the first threshold becomes first *playable* frame, not first
   menu frame.
2. **Chapter granularity.** `pack/story000` suggests chapter-scoped staging. Whether a session
   crosses chapters within one recording determines whether the plan needs a chapter selector.
3. **Frame alignment.** The trace has no presented-frame marker. Version 1 can approximate it by
   treating the first read after a fixed quiet period as the boundary; a marker written by
   `TUXBOX_TRACE_PRESENT` is cleaner and should be added to the recorder.
4. **Proton visibility.** Whether the mount needs `PRESSURE_VESSEL_FILESYSTEMS_RO` (or a bind
   into the prefix) is untested. Test before milestone 3.
5. **`data/ttdb` shape.** 56 GiB with unknown access pattern. The first recording answers whether
   it behaves like `levels.rpf` or like a database with its own page cache.

## References

- `runtime/xcrdapi/src/queue.rs`, `runtime/xcrdapi/src/lib.rs` — recorder implementation.
- `evidence/providers/xcrdapi-storage.json` — captured raw and zlib reads, block sizes and offsets.
- `docs/PROJECTSCARLETT.md` — package identity, staging procedure, launch requirements.
- `docs/USAGE.md` — shader preparation, which the boot-set step mirrors.
- RAGE archive structure: [RPF reference](https://raw.githubusercontent.com/TabbedScamper/GTAV-CLAUDE-MCP/main/RPF-REFERENCE.md),
  [rpf-archive source](https://docs.rs/rpf-archive/latest/src/rpf_archive/archive.rs.html),
  [ragekit archive notes](https://raw.githubusercontent.com/RatPrez/ragekit/main/docs/archives.md).
