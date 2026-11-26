# Instructions for AI coding agents

TuxBox is a compatibility layer that runs Xbox Series X|S games on Linux and Windows. Read [CONTRIBUTING](CONTRIBUTING.md) before changing anything; its rules apply to you. [Source layout](docs/SOURCE_LAYOUT.md) says where code goes, and [Where to improve](docs/FOCUS.md) lists open areas.

## Ask the person first

- Commit, push, open or merge a pull request, or post a comment only when they ask.
- Ask before deleting files, changing their system settings, or replacing a Wine prefix, cache or save.
- A game launch can take many minutes and take over the screen. Say so before starting one.

## Safety and privacy

- Run ProjectScarlett only through `tuxbox.py run`, which keeps it offline. Its crash handler uploads crash reports otherwise.
- Never commit retail game files, dumps, captures, shader bytecode from a title, logs or Wine prefixes.
- Keep usernames, home paths and other personal details out of commits, issues, comments and evidence.
- Don't copy code from projects under licences that don't fit [Apache 2.0](LICENSE), such as the GPL. Reimplement an idea from your own understanding and credit it.

## Finding a bug

- Find the cause before changing code. Don't skip, hide or work around a fault unless the person agrees.
- Say what you measured and what you are guessing. A theory is not a finding until a test could have disproved it and didn't.
- Compare against a build that worked, under the same conditions: same caches, settings and input. Check the host too (audio, drivers, disk, memory).
- Keep temporary tracing out of the commit that fixes the bug.
- Check the other trees first, as described below. Dozens of people run the same title from the same files, and several of them are further along.

## Working with the other trees

[git.gtahub.st](https://git.gtahub.st/explore/repos) mirrors about forty efforts on the same title, most of them forked from one tree. They read the same game files we do. A fault we hit is usually a fault someone else hit first.

Before starting an investigation, and again before calling one finished:

- Search the trees for the symptom, not the file. Grep the commit logs, the `docs/` pages, the `CHANGELOG` and the source for the words around the fault: `UAV`, `unordered`, `descriptor`, `stride`, `store`, `wave`, `skip`, `TDR`, `dxil`, `radv`, `amd`.
- Take Linux and AMD results as applying to us. Take a memory, allocation, engine or archive finding as applying to every platform. Take a Direct3D, Metal, Windows-API or driver-registry result as theirs alone, and say why it does not transfer.
- Prefer a measurement to a claim. A number in a `findings` or `bug-journal` page is worth more than a feature list, and it can be re-measured here.
- Record what you found in the `BR-n` entry and in `BUG-RECON-TODO.md` (BR-28 holds the first pass), with the tree, the file and the date you read it.
- Take ideas, measurements and test cases only. Every VIctory fork is GPL-3.0-or-later and XCL is GPL-2.0. Reimplement from your own understanding and credit the tree.

How to read the mirror: its API answers 403 and its code search answers 404. Use `https://raw.githubusercontent.com/<owner>/<repo>/<branch>/<path>` for a file, `https://api.github.com/repos/<owner>/<repo>/git/trees/<branch>?recursive=1` to list one, or clone it into the scratch directory and grep it there.

Each check costs minutes. Do it when a bucket stalls, not on every commit.

## Writing code

- Reproduce the behaviour in a test or a [Lanes self-test](examples/lanes/README.md#self-test) first. The test should fail without your change.
- Choose paths by what the driver can do, not by GPU vendor.
- A patch to a title's own code applies only when the bytes match the exact build, and the comment explains the fault it fixes.
- Rust files stay under 1,200 lines, and tests go in the crate's `src/tests/`.
- Match the code around you. Don't reformat or refactor what your change doesn't need.

## Checking your work

```sh
python tuxbox.py check --no-gpu   # while you work
python tuxbox.py check            # before a pull request, on a real GPU
```

- Report failures as they are, with the output. Never describe a skipped or failed check as passing.
- Shared code needs a run on an AMD card and an NVIDIA card. Say which one is still missing.
- Compiling is not proof that translated shaders are correct. Compare their output with a reference on the GPU.
- `docs/RESULTS.md` and `docs/progress.svg` are generated from `evidence/`. Don't edit them by hand, and don't write evidence for something that wasn't run.
