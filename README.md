# imagecull

Point imagecull at a folder of Nikon RAW files and it rates every frame 1–5 with
a local vision model, then writes each rating into an XMP sidecar that Lightroom
Classic, Bridge, darktable and digiKam already understand. Open the folder in
your editor and the cull is partly done: the keepers already carry stars, and
the frames that lost their group are marked so you can look at them first.

Nothing leaves your machine by default. There is no cloud call, no telemetry,
no catalogue and no editing — just ratings written beside your raws, which are
never modified. A configured `[server] endpoint` may name a service, and a run
then sends it previews; that is your decision, and the reason a token lives in
the environment rather than in a config file ([A server somewhere
else](#a-server-somewhere-else)).

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
- **A server that answers one decision per request.** A local oMLX on
  `127.0.0.1:8000`, version 0.7.1.dev1 or newer, by default; any host speaking
  the same decision API when `[server]` names one — Cloudflare's hosted Clef on
  Workers AI, for instance ([A server somewhere
  else](#a-server-somewhere-else)). imagecull does not start, stop or install a
  server; it only talks to one.
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

Decision models are flagged. `models` is oMLX's: it reads the server's own
endpoints, which another server does not have. Pointed at one, it says so and
makes no request. See [A server somewhere
else](#a-server-somewhere-else).

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
each sidecar records the rubric hash and the model name. Changing the rubric and
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

The report prints one line per group — the frames it spans and who kept it —
then a summary of the grouping and the cull. The per-frame measures are not
written to a file; they are available on the progress lines while the pass runs.

### Grouping thresholds

The defaults suit a typical burst. Tune them when they do not:

| Option | Default | Meaning |
|---|---|---|
| `--group-gap <SECS>` | `2.0` | Most time between consecutive frames of one group. |
| `--group-similarity <0..1>` | `0.90` | Least scene similarity to the group's first frame. Lower it to group a subject who moved; raise it to keep a pan as separate compositions. |
| `--group-exposure-stops <STOPS>` | `2.0` | Most brightness drift inside a group. `0` asks for one exposure exactly. |
| `--max-edge <PX>` | `512` | Longest edge of one image in a comparison. A rating's preview is `score --max-edge`, default `1568`. |
| `--jpeg-quality <1-100>` | `88` | JPEG quality of one image in a comparison. |
| `--cull-jobs <N>` | `4` | Groups compared at once. `[group] cull_jobs` sets it per shoot. |
| `-j, --jobs <N>` | one per core | Frames read at once. |
| `--no-cull` | — | Group and measure only. No request, no label. |

A server that takes fewer than twenty images per request sets `[group]
max_images`; a group larger than that is split into batches and its winners
compared again, so the cap costs a few more requests and changes no verdict.

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
`--no-config` ignores every config file. The `[preview]`, `[outputs]` and
`[run]` sections of older files are gone: a file that still sets one is refused,
naming the `[score]` key it became.

```toml
[server]
endpoint = "http://127.0.0.1:8000/v1/systemone"
# model = "clef-flash-oQ4e"
# token_env = "CLOUDFLARE_API_TOKEN"
# token = "..."
# truncate = true
# timeout = 120

[scan]
# recursive = false

[score]
# max_edge = 1568
# jpeg_quality = 88
# json_scores = "scores.json"
# rubric = "holiday"
# shoot_context = "night street, pushed two stops"
# dedupe = true

[group]
# gap = 2.0
# similarity = 0.90
# exposure_stops = 2.0
# max_edge = 512
# jpeg_quality = 88
# cull_jobs = 4
# max_images = 20
```

`[score]` is what a rating needs and `[group]` what a cull needs; `[server]` and
`[scan]` are shared. `groups` reads one key from `[score]` — `shoot_context`, so a
burst is framed the way the frame was rated — and nothing else. `[group]
cull_jobs` is the one degree a shoot carries in its file: how many groups may be
compared at once, which `--cull-jobs` overrides for a single run. `[group]
max_images` is the same kind of number one step down: how many frames one
comparison carries, for a server that refuses more per request.

## A server somewhere else

`[server]` describes the HTTP wire a run posts on, not a vendor: the address a
request goes to, what its body says it is, and what it carries to be let in. The
address is the whole request — path and all — and `{model}` in it becomes the
model name, verbatim and unencoded. So a local oMLX is the default, and
Cloudflare's hosted Clef — the same model family on Workers AI, for a machine
with no Apple Silicon to spare — is one config block away rather than a second
code path. Copy the recipe at the bottom of
[`imagecull.toml.example`](imagecull.toml.example), put your account id where
`<ACCOUNT_ID>` is, export the token, and run as usual:

```toml
[server]
endpoint = "https://api.cloudflare.com/client/v4/accounts/<ACCOUNT_ID>/ai/run/@cf/cloudflare/{model}"
model = "clef-flash"
token_env = "CLOUDFLARE_API_TOKEN"
truncate = false

[score]
max_edge = 1024      # Workers AI resizes past ~1 MP, so 1568 buys nothing

[group]
max_images = 4       # Workers AI's per-request cap; a bigger burst is split
```

```sh
export CLOUDFLARE_API_TOKEN=...      # Workers AI:Read is enough
imagecull score /path/to/shoot
```

Four facts about the request differ from a local oMLX, and each is one key, plus
`truncate`, which Workers AI's schema does not list at all:

| | local oMLX | Workers AI |
|---|---|---|
| address | `http://127.0.0.1:8000/v1/systemone` (the default) | `…/ai/run/@cf/cloudflare/{model}` — the id's prefix is in the address |
| body `model` | the alias, `clef-flash-oQ4e` | the name, `clef-flash` — the same string `{model}` becomes |
| auth | none | `Authorization: Bearer $CLOUDFLARE_API_TOKEN` |
| body extras | `truncate: true` | none — `truncate = false` sends the field absent |
| images per request | 20 | 4 — a bigger group is split, never refused |

Two models are served: `@cf/cloudflare/clef-flash`, the fast one in the recipe
above, which is `model = "clef-flash"`, and `@cf/cloudflare/clef`, the bigger one
— the same address ending in `{model}`, with `model = "clef"`.

**`models` stays oMLX-only.** It reads oMLX's own endpoints
(`/v1/models`, `/v1/models/status`), which no other server has, so
against a Workers AI config it refuses by name and makes no request — rather than
reporting a 404 as "nothing loaded". There is no cheap "is this service alive"
probe: the run's first real request is the test, and a refused credential (HTTP
401 or 403) stops the run at that one request instead of paying for four hundred
of them.

**A different model id is a different rubric hash**, so the frames already rated
are re-rated only under `--force`, and frames are never mixed under one hash.
Moving a shoot from a local 4-bit checkpoint to `@cf/cloudflare/clef-flash`
therefore costs a re-rate, which is the point: two models' stars are not
comparable. The one case the hash cannot tell apart is two servers serving the
same alias — the same `clef-flash-oQ4e` from a machine and from a tunnel is one
hash.

## What gets written

| File | Written by | Contents |
|---|---|---|
| `DSC_0001.xmp` | `score` | `xmp:Rating` and the provenance of the rating |
| `DSC_0001.xmp` | `groups` | `xmp:Label` (green/red/yellow) and the provenance of the group verdict |
| `DSC_0001.xmp.bak` | either | The previous sidecar, written only when one already existed |
| `previews/*.jpg` | `score --dry-run` | The exact previews a run would send |
| the `--json-scores` file | `score` | Every raw model answer |

The raws themselves are never written to, and the output paths are configurable
where it makes sense (`--previews-dir` for previews, `--json-scores` for the raw
answers).

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

- **The server is unreachable** — start oMLX and check `--endpoint`. The default
  is `http://127.0.0.1:8000/v1/systemone`; an endpoint with no path is posted to
  as written, which means `/`.
- **`models` refuses the config instead of answering** — it says the run posts
  to a server that is not oMLX. It reads oMLX's own probes and nothing else;
  `score` and `groups` still work there. See
  [A server somewhere else](#a-server-somewhere-else).
- **A run stops on `http_401` or `http_403`** — the server refused the
  credential, so every remaining frame would fail the same way. The row names
  `[server] token_env` (or `[server] token`) and the account the endpoint names;
  check that the variable is exported in this shell and that its token has
  Workers AI permission.
- **Scores look wrong for portrait frames** — run `score --dry-run` and inspect
  `./previews`. Orientation is applied before the model sees the frame.
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
