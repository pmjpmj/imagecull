# imagecull

Point imagecull at a folder of Nikon RAW files and it rates every frame 1–5 with
a local vision model, then writes each rating into an XMP sidecar that Lightroom
Classic, Bridge, darktable and digiKam already understand. Open the folder in
your editor and the cull is partly done: the keepers already carry stars, and
the frames that lost their group are marked so you can look at them first.

Nothing leaves your machine. There is no cloud call, no telemetry, no catalogue
and no editing — just ratings written beside your raws, which are never modified.

## What it does

There are two things to run:

- **`imagecull score <DIR>`** judges every frame on its own and writes a 1–5
  star rating into its sidecar.
- **`imagecull groups <DIR>`** finds the frames of the same moment (a burst, a
  bracket, a small reframing) and asks the model which frame of each group to
  keep. The keeper gets a green label, the rest get red — or yellow when the
  model found the choice close.

Run `score` when you want a star rating across a whole shoot. Run `groups` when
the shoot has bursts and near-duplicates and you want to know which frames to
delete first. You can run either on its own, or both.

```
1 delete · 2 not great · 3 okay to keep · 4 good · 5 great
```

## Requirements

- **macOS on Apple Silicon.** The build is tested on arm64; other platforms are
  not supported in v1.
- **Rust** (a recent stable toolchain; the workspace declares its minimum in
  `Cargo.toml`).
- **A local oMLX server, version 0.7.1.dev1 or newer**, serving a Clef decision
  model. The default endpoint is `http://127.0.0.1:8000`. imagecull does not
  start, stop or install oMLX; it only talks to it.
- Nikon mirrorless RAW files: `.nef`, `.nrw` or `.dng`. Other files in the
  folder are ignored.

## Install

From a clone of this repository:

```sh
cargo install --path crates/imagecull-cli
```

Or build without installing:

```sh
cargo build --release
./target/release/imagecull --help
```

## Check the setup first

Ask oMLX what it is serving:

```sh
imagecull models
```

Decision models are flagged. Then run the self-test:

```sh
imagecull doctor
imagecull doctor /path/to/shoot/DSC_0001.NEF
```

`doctor` checks that the server is reachable and new enough, that the model
alias exists, that it is being served as a decision model rather than a plain
vision model, and — when you pass a file — that the camera's embedded preview
survives the pipeline. If something is wrong it prints one actionable sentence
instead of letting a 400-file run fail 400 times.

## Quick start

Preview what would be sent, without making any requests:

```sh
imagecull score --dry-run /path/to/shoot
```

The exact JPEGs a real run would send land in `./previews`. Check the
orientation of a portrait frame and that detail survives the downscale.

Score a few frames:

```sh
imagecull score --limit 20 /path/to/shoot
```

Read what would be written before writing it:

```sh
imagecull score --print-sidecars --limit 20 /path/to/shoot
```

This makes the requests and prints each sidecar it would create or update, but
writes nothing. When the ratings look sensible:

```sh
imagecull score /path/to/shoot
```

A run prints one line per frame, then a summary:

```
142/400  DSC_0555.NEF  4★  812ms  eta 3m10s
```

## `score` — rate every frame

Each frame is judged by one question: what is this frame worth as a keeper,
from 1 to 5. The model sees the camera's own embedded JPEG, already oriented
and downscaled — never demosaiced RAW sensor data.

- **Ratings go to `<frame>.xmp`**, e.g. `DSC_0001.NEF` → `DSC_0001.xmp`, as
  `xmp:Rating`. The sidecar also records which model, rubric and build produced
  the rating, and when.
- **An existing rating is left alone.** If a sidecar already carries a star
  rating of 1–5, the frame is skipped (`5★ kept`) and costs one fewer request.
  Interrupting a run with Ctrl-C is therefore cheap: finished frames are already
  on disk. Pass `--force` to rate them again.
- **Raws are opened read-only.** Existing sidecars are merged, not replaced:
  develop settings, keywords, GPS and other applications' metadata are preserved
  byte for byte. A `.xmp.bak` backup is written the first time an existing
  sidecar is updated.

### The rubric

The rubric is the wording the frame is judged by. Three are built in:

| Rubric | Use it for |
|---|---|
| `general` (default) | Any shoot. Assumes no genre and judges the frame on the terms it appears to set for itself. |
| `press` | A photojournalistic shoot, judged as a photo editor would. |
| `holiday` | A family album from a trip, where the people and the moment matter more than the landscape. |

```sh
imagecull score --rubric holiday /path/to/shoot
```

`--rubric` also takes a path to your own rubric file. `--shoot-context` passes a
sentence about the shoot to the model, so a dim, high-ISO night shoot is judged
as that:

```sh
imagecull score --shoot-context "night street, pushed two stops" /path/to/shoot
```

Ratings made under different rubrics or models are not comparable, which is why
each sidecar records the rubric hash and the model alias. Changing the rubric and
re-running will not re-rate anything unless you also pass `--force`.

### Useful options

| Option | What it does |
|---|---|
| `--limit <N>` | Smoke-test the first N files. |
| `--recursive` | Also scan subdirectories (hidden directories are never scanned). |
| `--dry-run` | Write the previews that would be sent; make no requests. |
| `--print-sidecars` | Make the requests, print what each sidecar would become, write nothing. |
| `--force` | Re-rate frames whose sidecars already carry a rating. |
| `--json-scores <FILE>` | Keep every raw model answer, for debugging. |
| `--quiet` / `--verbose` | Summary only, or one detailed line per frame. |
| `--allow-foreign` | Write into a sidecar owned by another application (darktable), which will overwrite it from its own history. |

## `groups` — cull bursts

`groups` first works out which frames belong together, entirely on your CPU and
with no model involved: close in capture time, the same body and optics, an
exposure within tolerance, and the same scene by image signature. Each group of
two or more is then compared by the model, which picks the frame to keep.

- The keeper gets `xmp:Label="Green"`.
- The other frames get `xmp:Label="Red"`.
- If the comparison was a near-tie, they get `Yellow` instead — look at those
  before deleting.

The tool only writes a label that is absent or that a previous imagecull run
wrote. Your own colour labels are never touched.

```sh
# See how the shoot groups, without asking the model or writing anything
imagecull groups --no-cull /path/to/shoot

# Check the comparisons and the sidecars before committing
imagecull groups --print-sidecars --limit 40 /path/to/shoot

# Do the cull
imagecull groups /path/to/shoot
```

Every run also writes a measures CSV (`groups.csv` by default) with one row per
frame: which group it fell in, and the frame's own sharpness, shake, roll and
clipping numbers, plus the camera's focus report. Those are measurements for
comparing frames within a group — not another score.

### Grouping thresholds

The defaults suit a typical burst. Tune them when they do not:

| Option | Default | Meaning |
|---|---|---|
| `--group-gap <SECS>` | `2.0` | Most time between consecutive frames of one group. |
| `--group-similarity <0..1>` | `0.90` | Least scene similarity to the group's first frame. Lower it to group a subject who moved; raise it to keep a pan as separate compositions. |
| `--group-exposure-stops <STOPS>` | `2.0` | Most brightness drift inside a group. `0` asks for one exposure exactly. |
| `--max-edge <PX>` | `512` | Longest edge of one image in a comparison. A rating's preview is `score --max-edge`, default `1568`. |
| `--jpeg-quality <1-100>` | `88` | JPEG quality of one image in a comparison. |
| `--cull-jobs <N>` | `4` | Groups compared at once. |
| `-j, --jobs <N>` | one per core | Frames read at once. |
| `--no-cull` | — | Group and measure only. No request, no label. |

Changing a grouping threshold changes which frames were compared together, so
verdicts made under one setting are not comparable with those made under
another. Each sidecar records the grouping hash it was written under.

## Configuration file

Instead of repeating the same flags, put them in a TOML file. Copy
[`imagecull.toml.example`](imagecull.toml.example) and edit it.

Put it in the shoot folder to configure that shoot:

```
/path/to/shoot/imagecull.toml
```

Or in your user config directory to apply it everywhere:

```
~/.config/imagecull/config.toml
```

Or pass one explicitly with `--config <FILE>`. Exactly one file is read — files
never layer — and the first that exists wins:

1. `--config <FILE>`
2. `./imagecull.toml`
3. `$XDG_CONFIG_HOME/imagecull/config.toml`
4. `~/.config/imagecull/config.toml`
5. built-in defaults

A flag beats the file, and the file beats the built-in default. Unknown keys are
an error, so a typo cannot silently run a shoot at the wrong resolution.
`--no-config` ignores every config file.

```toml
[server]
endpoint = "http://127.0.0.1:8000"
# model = "clef-flash-oQ4e"

[preview]
# max_edge = 1568
# jpeg_quality = 88

[scan]
# recursive = false

[group]
# gap = 2.0
# similarity = 0.90
# exposure_stops = 2.0
# max_edge = 512
# jpeg_quality = 88

[run]
# rubric = "holiday"
# shoot_context = "night street, pushed two stops"
```

## What gets written

| File | Written by | Contents |
|---|---|---|
| `DSC_0001.xmp` | `score` | `xmp:Rating` and the provenance of the rating |
| `DSC_0001.xmp` | `groups` | `xmp:Label` (green/red/yellow) and the provenance of the group verdict |
| `DSC_0001.xmp.bak` | either | The previous sidecar, written only when one already existed |
| `groups.csv` | `groups` | One row per frame: group, measures, status, error |
| `previews/*.jpg` | `score --dry-run` | The exact previews a run would send |
| the `--json-scores` file | `score` | Every raw model answer |

The raws themselves are never written to, and the output paths are configurable
where it makes sense (`--out` for the CSV, `--previews-dir` for previews).

## Interrupting and re-running

The sidecar is the only memory between runs. Ctrl-C stops the run with whatever
was already rated on disk, and a re-run skips those frames. There is no database
to corrupt and no resume flag to remember. A frame the model never answered gets
no sidecar at all — no zero, no blank file.

## Exit codes

| Code | Meaning |
|---|---|
| `0` | Every frame succeeded (or was skipped because it was already rated). |
| `1` | The run completed, but some frames have failed rows. |
| `2` | Fatal: bad arguments, missing folder, unreachable server, rejected rubric, or a `score` run whose sidecar could not be written. |
| `3` | Interrupted with Ctrl-C. What was rated is already on disk. |

## Troubleshooting

- **`doctor` says the server is unreachable** — start oMLX and check
  `--endpoint`. The default is `http://127.0.0.1:8000`.
- **`doctor` says the model is not a decision model** — the oMLX version is too
  old, or the Clef checkpoint was loaded as a plain vision model. Upgrade to
  oMLX 0.7.1.dev1 or newer and load a Clef decision model.
- **Scores look wrong for portrait frames** — run `score --dry-run` and inspect
  `./previews`. Orientation is applied before the model sees the frame;
  `doctor <FILE>` reports the same preview pipeline on one file.
- **A frame was skipped that you wanted re-rated** — its sidecar already has a
  rating. Use `--force`, or a different rubric plus `--force`.
- **A sidecar was not written** — the run summary reports the status. Common
  causes: the sidecar is owned by darktable (`xmp_foreign`, override with
  `--allow-foreign`), it is not valid XMP (`xmp_unreadable`), or the folder is
  read-only (`xmp_write_failed`). In `groups` such a frame becomes a failed row
  and the run exits 1.

## How it is built

`imagecull` is a Rust workspace with all scoring logic in `imagecull-core` and a
thin CLI in `imagecull-cli`, so the core has no terminal or process-exit
assumptions. The full behaviour is specified in [`SPEC.md`](SPEC.md); the
vocabulary (frame, group, keeper, measure, rubric, sidecar) is defined in
[`GLOSSARY.md`](GLOSSARY.md); the architectural decisions are in
[`docs/adr/`](docs/adr/).

Local checks run on commit through `.githooks/pre-commit` (`rustfmt` and
`cargo clippy --workspace --all-targets --locked -- -D warnings`). Enable it once
per clone with `git config core.hooksPath .githooks`, then run the tests with:

```sh
cargo test --workspace --locked
```
