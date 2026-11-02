# imagecull — Specification

**Status:** Draft v0.1 · **Target platform:** macOS on Apple Silicon · **Language:** Rust
**One-liner:** Score a folder of Nikon mirrorless NEFs with a local Clef decision model over oMLX, and write each rating into an XMP sidecar so Lightroom Classic opens pre-culled.

---

## 1. Problem

A shoot lands as 300–800 NEFs. All of them are technically fine; maybe 10% are the keepers. Judging that today means opening every frame in a RAW editor and tapping stars — an hour or two of pure triage, before any editing starts.

The task is *decision*, not generation: each frame gets one keeper rating from 1 to 5. That maps exactly onto Cloudflare's **Clef**, served locally by **oMLX** on `POST /v1/systemone` — it reads a state plus a schema of typed questions and returns a probability for every allowed option in **one forward pass**, with no generated text to parse.

## 2. Goals

| # | Goal |
|---|---|
| G1 | Point at a folder; get one rating per Nikon mirrorless RAW, written where the cull reads it, with no manual work. |
| G2 | Preview-quality input: never decode RAW sensor data; use the JPEG the camera embedded. |
| G3 | One keeper rating per frame, 1–5, recorded exactly as the model chose it. |
| G4 | No post-hoc thresholds: the rating is the model's own `choice`, never derived from other numbers. |
| G5 | Interruptible and cheap to finish; a 400-file run stopped by Ctrl-C, sleep or a server restart costs only the frames it had not rated. |
| G6 | Deterministic: same image + same rubric + same model ⇒ same numbers, so results are cacheable. |
| G7 | The raws are read-only. A run writes sidecars beside them and never modifies a frame. |
| G8 | The core is a library crate, so the eventual macOS GUI is a new front end, not a rewrite. |

## 3. Non-goals (v1)

- No RAW demosaic, no editing, no colour pipeline, no output rendering.
- No catalog database, no library management, no import/export to Lightroom's catalog.
- No writing anywhere but the sidecars of the shoot folder (no catalogue import, no raw edits), no GUI (v3, §13).
- No video (`/v1/systemone` does not accept video over HTTP).
- No cloud calls, no telemetry, no server management (starting/stopping oMLX).
- No batching. A frame is rated by one request and a group is compared by one request per round; the client never packs several frames into a per-frame decision. The group cull may have several *groups'* comparative requests in flight at once (`--cull-jobs`), which is a pipeline, not a batch.

## 4. Locked decisions

| Decision | Choice | Why |
|---|---|---|
| Language | **Rust** (clap CLI → Tauri GUI) | Strict types for the systemone request/answer contract; one fast binary; Tauri reuses the same core crate for the Mac GUI. |
| Preview source | **Largest embedded JPEG** in the NEF | Camera-rendered, already sharpened; no colour pipeline to build. |
| Scoring shape | **One `choice` question, answered 1–5** | A cull wants a keeper rating, not a diagnosis; a single 5-way choice is Clef's native mode and leaves nothing to combine. |
| Rating legend | **1 = delete · 2 = not great · 3 = okay to keep · 4 = good · 5 = great** | Five named levels, so the answer is a decision the photographer can act on and a spreadsheet can sort. |
| Output | **An XMP sidecar per frame** — `DSC_0001.xmp` | The cull happens in a catalogue, so the rating goes where the catalogue reads it (`xmp:Rating`), not into a file beside the terminal. |
| Run model | **Sequential ratings, a fan-out cull, and skip what is already rated** | oMLX serialises requests, so a rating run gains nothing from overlap; the cull's groups are independent and `--cull-jobs` overlaps their preview rendering and server waits. The sidecar is the state, so an interrupted run finishes cheaply. |
| A rating already there | **Left alone** (1–5), `--force` to override | The photographer's star and the model's must never fight over one property. |

## 5. Architecture

```
                    ┌──────────────────────────────────────────────┐
  DIR/*.NEF  ──────▶│ ingest     scan, filter, sort (mtime, name)  │
                    └───────────────┬──────────────────────────────┘
                                    ▼
                    ┌──────────────────────────────────────────────┐
                    │ extract    raw-files → largest_preview()     │
                    │            → orientation + size              │
                    └───────────────┬──────────────────────────────┘
                                    ▼
                    ┌──────────────────────────────────────────────┐
                    │ preprocess orient → downscale to max_edge    │
                    │            → JPEG q88 → data URI             │
                    └───────────────┬──────────────────────────────┘
                                    ▼
                    ┌──────────────────────────────────────────────┐
                    │ decide     POST /v1/systemone (1 image,      │
                    │            1 question, 1 forward pass)       │
                    └───────────────┬──────────────────────────────┘
                                    ▼
                    ┌──────────────────────────────────────────────┐
                    │ record     <frame>.xmp (merge, atomic rename) │
                    │            → xmp:Rating + imagecull:*         │
                    └──────────────────────────────────────────────┘
```

### 5.1 Crate layout

```
imagecull/
  Cargo.toml                    # workspace
  crates/
    imagecull-core/             # no CLI, no TTY assumptions — GUI-safe
      src/lib.rs
      src/ingest.rs             # folder scan, extension filter, dedupe keys
      src/preview.rs            # raw-files extraction + orientation
      src/preprocess.rs         # downscale + JPEG encode + data URI
      src/clef.rs               # systemone client, request/answer types
      src/rubric.rs             # question schema, TOML load, rubric hash
      src/scoring.rs            # orchestration: one file → one Outcome
      src/sidecar.rs            # the XMP document, the writer port, raw answers
      src/csv.rs                # RFC 4180 rows, for the measures report
      src/error.rs              # ErrorKind taxonomy
    imagecull-cli/              # bin: imagecull
      src/main.rs
      src/progress.rs           # indicatif + tracing
    imagecull-gui/              # (v3) Tauri app, depends on core only
  crates/imagecull-core/rubrics/  # general.toml, press.toml, holiday.toml
  tests/fixtures/               # see §13.1
```

**Hard rule:** `imagecull-core` takes a `Reporter` trait for progress and never touches stdout, TTY, or process exit. That is what makes the Tauri port cheap.

---

## 6. Input handling

### 6.1 File selection

- Extensions (case-insensitive): `.nef`, `.nrw` (Nikon 1 / Coolpix), `.dng` (Nikon Z can be set to DNG). Anything else is skipped with a `skipped_extension` count, never an error.
- Non-recursive by default; `--recursive` walks subdirectories.
- Sorted by filename (natural/NFC-normalised) so runs are reproducible and the order a shoot is rated in is stable.
- macOS specifics: ignore `._*` AppleDouble files, `.DS_Store`, and anything under `__MACOSX`. Report how many were ignored.
- Hidden directories are not descended into.

### 6.2 Preview extraction

Use `raw-files` (pure Rust, no C deps, documented never to panic on malformed input):

```rust
let mut file = RawFile::open(&path)?;
let preview = file.largest_preview().ok_or(Error::NoPreview)?;
let jpeg = file.read_preview(&preview)?;      // bytes, still un-oriented
let (w, h) = preview.size();
let orientation = preview.orientation();      // "the orientation a viewer must apply"
```

Fallbacks and rejections:

- If `largest_preview()` is absent → `status=no_preview`, row written, run continues.
- Selection rule is **largest by pixel area**, not largest byte count (some cameras embed a smaller, higher-quality `Medium` next to`Full`).
- If the whole file fails to open → `status=unreadable`, row written with the error kind.
- Do **not** shell out to `exiftool` and do **not** link libraw in v1. Both are user-facing install steps and a licensing/signing tax on the eventual `.app`.

### 6.3 Orientation — the bug that silently ruins scores

Nikon's embedded preview is stored in sensor orientation; a portrait frame arrives sideways. A vision model scoring a sideways photo will judge composition of a rotated frame. So:

1. Read `orientation` from the NEF/preview.
2. Decode with `image`, then `DynamicImage::apply_orientation(..)` — EXIF orientations 1–8.
3. Only *then* downscale.

Assets: `image` (JPEG decode/encode, `apply_orientation`), `fast_image_resize` (Lanczos3). Test this with a portrait fixture; it is the single most likely v1 defect after the HTTP layer.

### 6.4 Preprocess

| Parameter | Default | Notes |
|---|---|---|
| `max_edge` | 1568 px | Longest edge after orientation. Matches common VLM tiling budgets; keeps the image inside a fraction of Clef's 16,384-token context. |
| `jpeg_quality` | 88 | Encode via `image`'s JPEG encoder, `ycbcr` 8-bit. Only affects bytes on the wire, not tokens. |
| Encoding | `data:image/jpeg;base64,…` | oMLX takes images as data URIs or raw base64. A full data URI is explicit and unambiguous. |
| EXIF in output | stripped | The encoder writes no APP1; nothing to leak, nothing to confuse the model. |

Typical payload: 1568 px JPEG ≈ 250–400 KB → ≈ 340–540 KB base64. Irrelevant over loopback.

A comparison carries several images in one request, so `groups` has its own
pair — `[group] max_edge` / `[group] jpeg_quality`, `groups --max-edge` /
`--jpeg-quality`, defaulting to 512 px / 88 — and `[preview]` never reaches a
cull (ADR 0014). Twenty previews at 512 px are 4,154 of the 16,384 image
tokens; the same twenty at 1568 px do not fit, which is why the two numbers are
separate.

`--dry-run` renders the exact bytes that would be sent into `./imagecull-previews/<name>.jpg`, so the user can eyeball orientation, crop, and detail loss before burning a 400-file run.

---

## 7. Clef over oMLX — the wire contract

**Requires oMLX ≥ 0.7.1.dev1** (decision models merged in `jundot/omlx#4315`). Before that, Clef checkpoints loaded as plain VLMs and returned meaningless text; `POST /v1/chat/completions` on a decision model returns **400** pointing at `/v1/systemone`. `imagecull doctor` must detect and explain exactly this.

### 7.1 Request

```http
POST {endpoint}/v1/systemone
Content-Type: application/json
```

```json
{
  "model": "clef-flash-oQ4e",
  "state": {
    "task": "photo_cull",
    "filename": "DSC_0421.NEF",
    "shoot_context": "street at dusk, high ISO expected, 1/60s handheld"
  },
  "images": ["data:image/jpeg;base64,/9j/4AAQSkZJRg..."],
  "truncate": true,
  "questions": { "…§8…" }
}
```

- `state` must not be null. It carries the *task framing* and user context — never the scores.
- Exactly **one** image per request. Several are supported by the API, but one frame per decision keeps results attributable.
- `truncate: true` (default). Clef cuts the state to fit 16,384 tokens instead of returning 413. Our prompt is far smaller than that, so truncation should never fire — if it does, the telemetry in §9 will show it via an unexpectedly low `input_tokens` ceiling.
- A per-frame rating is one request at a time. The group cull may have up to `--cull-jobs` comparative requests in flight at once (default 4); oMLX serialises them server-side, so the degree is bounded by the option and is not a promise of server parallelism.

### 7.2 Response

Per the oMLX implementation, one answer per question:

| type | answer fields |
|---|---|
| `noul` | probability of yes |
| `choice` | `choice`, `probabilities`, `confidence` |
| `score` | `score`, `probabilities`, `confidence`, `legend` |

Plus `usage.input_tokens` (and `usage.latency_ms` on the reference server).

Parsing rules (defensive — this contract is new surface):

- `answer.choice` is a **string** key from `criteria`; parse to `u8` for the rating.
- `answer.score` is a **float** (expected value over the legend). The cull asks no `score` question today, so none is recorded; the type stays part of the wire contract.
- `probabilities` may arrive as an ordered array or as an object keyed by option. Deserialize as `serde_json::Value` and only *display* it; **never** derive the recorded answer from it. `choice` and `confidence` are the source of truth.
- The full raw answer JSON is preserved for `--json-scores`, so nothing is lost if the shape shifts.
- Treat an answer key that is missing from `answers` as `status=bad_response` for that file.

### 7.3 Client

`ureq` (blocking) + `serde_json`. No tokio in v1: the client is blocking and `Sync`, so a rating run stays serial and the cull's workers can share it. A blocking client keeps `imagecull-core` trivially portable into Tauri (which can call it from a background thread). Timeouts: connect 5 s, read 120 s (`--timeout`), overridable because 27B Clef on a long prompt can take tens of seconds.

---

## 8. The rubric

One question, one forward pass. Encoded as TOML so wording can be tuned without a rebuild; three rubrics ship via `include_str!`, `general` is the one a run uses when nothing selects another, and `--rubric` takes a name or a path. The effective rubric is hashed (§8.3) and the name it was selected by is provenance only.

The wording is the judging, so three rubrics are compiled into the binary and
selected by name: `general` (the default — no genre assumed, the frame judged on
the terms it appears to set for itself), `press` (a photo editor culling a
photojournalistic shoot) and `holiday` (a family album from a trip, where a family
member recognised and unguarded is the subject and a landscape must hold the
place and its light). `--rubric` takes one of those three names or a path to any
other file that keeps the `stars` id and the five rating keys; `[run] rubric` in
a config file takes the same value. Each rubric's file stays in the tree under
`crates/imagecull-core/rubrics/` as the copy to read and to start a draft from;
ADR 0004 records the family.

Three rubrics differ only in words, and the hash keeps their reports from ever
being compared. A name is provenance, not identity: `--rubric holiday` and a path
to a byte-identical copy of that file are the same judgement under the same hash,
because the hash covers the question schema and the alias, never how the rubric
was named.

### 8.1 The one question

The cull asks one `choice` question, `stars`, answered on a five-point legend.
The frame is judged whole in that one call — what it is trying to be, how well it
is made, and whether it holds anything worth looking at — because a cull wants a
keeper rating, not a diagnosis. The legend is fixed:

```toml
[[question]]
id = "stars"
type = "choice"
instructions = "…the rubric's own words…"
criteria = [
    { key = "1", description = "delete — nothing in it anyone would keep" },
    { key = "2", description = "not great — a real flaw, or nothing to hold the eye" },
    { key = "3", description = "okay to keep — fine, and worth keeping as a record" },
    { key = "4", description = "good — clearly worth keeping; one to share or print" },
    { key = "5", description = "great — the frame the folder is remembered by" },
]
```

- **1 = delete · 2 = not great · 3 = okay to keep · 4 = good · 5 = great.** The
  gloss after the dash is the rubric's to write; the anchor word is not, because
  those five words are what a report's reader is sorting by.
- The keys are `"1"`…`"5"` in that order. The report's integer column parses
  them, and the sidecar maps the key straight to `xmp:Rating`.
- Because the answer is the model's own choice, there is nothing to combine, rescale or
  threshold. A frame is one number, and the number is the decision.

`criteria` for `choice` is an **array of `{key, description}`** in TOML rather
than a table, because TOML tables do not guarantee key order and the option order
matters to the model's legend. On the wire it is rendered as an object keyed by
option, the shape the Clef `choice` head reads; see
`docs/adr/0001-rubric-renders-two-criteria-shapes.md` and
`docs/adr/0005-one-rating-replaces-six-questions.md`.

### 8.2 Confidence

Each `choice` answer carries `confidence`, the top-option probability. It is
recorded per frame (`conf_stars`) because it is the cheapest "the model is
unsure" signal available, and the summary reports the distribution of ratings
beside it. Confidence never changes an answer: a low number marks a frame a
photographer may want to look at, not a frame to re-ask about.

### 8.3 Rubric hash

`sha256` over the canonicalised question schema (id, instructions, criteria, in
declaration order) + `model` alias, with the schema tag `imagecull-rubric-v2`.
Recorded in the summary, printed in the provenance line, and written into every
sidecar as `imagecull:rubricHash`. It is how a rating says which wording made it,
and how a skipped frame can be explained: changing wording means a re-score needs
`--force`, by design.

---

## 9. Output — one sidecar per frame

A `score` run writes the rating into an XMP sidecar beside the frame it judged,
and nothing else; an `imagecull groups` run writes the group's verdict and the
red mark that follows from it, and no rating. `DSC_0001.NEF` → `DSC_0001.xmp`: the base name with the extension
replaced, never `DSC_0001.NEF.xmp`, which is darktable's history file. The raws are
opened read-only and never modified.

### 9.1 The document

```xml
<x:xmpmeta xmlns:x="adobe:ns:meta/" x:xmptk="imagecull 0.1.0">
 <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Description rdf:about=""
    xmlns:xmp="http://ns.adobe.com/xap/1.0/"
    xmlns:imagecull="https://ns.imagecull.app/1.0/"
    xmp:Rating="4"
    imagecull:confStars="0.63"
    imagecull:model="clef-flash-oQ4e"
    imagecull:rubricHash="9f3c1a77"
    imagecull:scoredAt="2026-11-08T10:14:02Z"
    imagecull:version="0.1.0"/>
 </rdf:RDF>
</x:xmpmeta>
```

| property | value | why |
|---|---|---|
| `xmp:Rating` | the model's 1–5 | the portable channel: LrC, Bridge, darktable and digiKam all read it |
| `imagecull:confStars` | the model's own confidence, unrounded | the cheapest "the model is unsure" signal, kept where the rating is |
| `imagecull:model` | the alias used | provenance: which model said 4 |
| `imagecull:rubricHash` | the rubric hash's first 8 hex | provenance: which wording said 4, and why a skip can be explained |
| `imagecull:scoredAt` | RFC 3339, UTC | provenance: when |
| `imagecull:version` | the binary's version | provenance: which build wrote it |

A frame that lost its group also carries the verdict that made it red, so the
red can be explained without the folder (ADR 0008):

| property | value | why |
|---|---|---|
| `imagecull:groupKeeper` | `true` / `false` | whether this frame is its group's keeper — not a copy of the label, which may be missing or the photographer's |
| `imagecull:groupStart` | the group's first frame's capture time, ISO 8601 | which moment it lost |
| `imagecull:groupSize` | how many frames the group holds | how much of a choice it lost |
| `imagecull:groupingHash` | the grouping hash's full hex | the thresholds the grouping was computed under, so a stale verdict is recognisable |
| `imagecull:pickBy` | `comparison` | what chose the keeper |
| `imagecull:pickConf` | the comparison's top-option probability | how sure the choice was, on every member |
| `imagecull:pickedAt` | RFC 3339, UTC | when |
| `imagecull:pickRounds` | how many comparative requests the verdict took | a group compared in batches never reads as one request's certainty |
| `imagecull:labeledBy` | `imagecull` | written with the label: the proof a later run may rewrite or retire it |

There is no `imagecull:stars`: the rating is `xmp:Rating`, and two properties
holding one number is a disagreement waiting to happen. `xmpDM:pick` is never
written, and `xmp:Rating=-1` (the destructive reject) is never written by this
tool. `xmp:Label` is written only by the group cull: `Green` on the keeper,
`Red` on a non-keeper of a confident group, and `Yellow` on a non-keeper of a
group below `pick_conf_floor`, and only where the label is absent or was written
by a previous run. A mark this tool wrote is rewritten when a later verdict says
something else, or retired when the frame is left in no group of two or more;
a label without `imagecull:labeledBy` is never touched
([ADR 0008](docs/adr/0008-the-group-command-culls.md),
[ADR 0012](docs/adr/0012-green-marks-the-keeper-and-yellow-the-unsure.md)). Every
other label, on every other frame, stays the photographer's.

### 9.2 Merge, not replace

A sidecar may already carry ACR develop settings (`crs:`), keywords, GPS or
darktable history. Rule: **byte-preserve everything we do not own.**

The update is a targeted text edit, not a parse-and-reserialise: find the first
`rdf:Description` opening tag, rewrite the value of each property it already
carries *in place*, append the ones it lacks, add the namespace declarations the
tag does not already carry, and retire any `imagecull:` property this build no
longer writes. Quoting style, attribute order, comments, CDATA and foreign
namespaces are untouched. A document that already says exactly what this run
would write is returned unchanged. No XML dependency is added: byte-preservation
is the requirement, and a parser cannot give it.

A document with no `rdf:Description` to write into is a refusal
(`xmp_unreadable`), never a guess.

### 9.3 Writing

- **Atomic.** A temp file in the same directory, then `rename` over the target:
  a killed run or a full disk leaves the old sidecar or the new one, never half of
  one. An existing sidecar's permission bits are preserved.
- **Backup.** `DSC_0001.xmp.bak` holds what was there before, written only when
  the sidecar already existed, and never over an existing `.bak`. A first run
  over a fresh shoot leaves none.
- **Ordering.** The sidecar is written before the frame line is printed, so the
  terminal never claims a rating that is not yet on disk.
- **Silence on failure.** A frame the model never answered gets no sidecar at
  all: "no answer" must never reach the disk as a zero, a blank or an empty file.

### 9.4 Skipping what is already rated

If a frame's sidecar already carries an integer `xmp:Rating` of 1–5, the run
makes no request, writes nothing, and reports the frame as `5★ kept`. That single
rule protects a photographer's rating and makes an interrupted run cheap to
finish. `--force` rates such frames again; the merge and backup rules then apply.

The rule cannot tell who set the rating. Consequently a re-score under a changed
rubric is a no-op without `--force`, which is why `imagecull:rubricHash` is in the
document: the run can say what the existing rating was judged by.

### 9.5 What a run still writes to a file

`--json-scores answers.json` writes every raw `/v1/systemone` answer document,
once, at exit, keyed by the frame's path inside the shoot folder. It is the
debugging record, not the deliverable, and it is off unless asked for.

### 9.6 Review before a shoot

`--print-sidecars` scores the folder, prints what each sidecar would become —
`DSC_0421.NEF  would create DSC_0421.xmp`, then one line per property with the
value already there beside it — and writes nothing at all. Run it over 20 frames
and read them before letting the run loose on 400: the failure being guarded
against is a wrong legend quietly labelling an entire library.

---

## 10. Failure taxonomy

| status | cause | action |
|---|---|---|
| `ok` | — | — |
| `unreadable` | file open/parse error | continue |
| `no_preview` | no embedded JPEG found | continue |
| `preview_broken` | JPEG decode fails | continue |
| `http_400` | invalid criteria / bad question schema | **abort run** (rubric is wrong) |
| `http_413` | context too long | record, continue, suggest smaller `--max-edge` |
| `http_422` | malformed body | abort run (client bug) |
| `server_unreachable` | connection refused / DNS | abort after retries |
| `timeout` | exceeded `--timeout` | retry, then record |
| `bad_response` | answer missing/typed wrong | keep the raw JSON for `--json-scores`, continue |
| `xmp_unreadable` | the existing sidecar has no `rdf:Description` to write into, or cannot be read | leave it alone, continue |
| `xmp_foreign` | the sidecar belongs to another application (a `darktable:` namespace) | leave it alone, continue; `--allow-foreign` overrides |
| `xmp_write_failed` | the sidecar could not be written (read-only mount, full disk) | **abort run** in `score`: every remaining frame would fail identically, and each was already paid for. In `groups` it is a failed row (exit 1): a cull's remaining verdicts were not paid for, and writing them is better than leaving the delete set half-applied and silent |

Retries: 3 attempts with 0.5/1/2 s backoff, only for `server_unreachable`, `timeout`, and 5xx. Never retry 4xx (a wrong rubric will fail identically 400 times).

**Preflight** (before the first file): `GET /v1/models`, check the alias exists, and if the server exposes `/v1/models/status` confirm `model_type == "decision"`. Fail fast with an actionable message rather than 400 files of the same 400.

Exit codes: `0` all ok (a run that skipped every frame is all ok) · `1` completed with failed rows · `2` fatal (bad args, dir missing, server unreachable, rubric rejected, a sidecar write failed) · `3` interrupted (SIGINT; what was rated is already on disk).

---

## 11. CLI surface (v1)

```
imagecull score <DIR> [OPTIONS]
        --endpoint <URL>        default: http://127.0.0.1:8000
    -m, --model <ALIAS>         default: from rubric.toml
        --rubric <NAME|FILE>    one of general/press/holiday, or a path
                                default: general
        --shoot-context <STR>   free text passed into state.shoot_context
        --max-edge <PX>         default 1568
        --jpeg-quality <1-100>  default 88
        --recursive [BOOL]      also scan subdirectories
        --json-scores <FILE>    every raw answer document, for debugging
        --no-dedupe
        --force                 rate frames again even though their sidecars hold a rating
        --allow-foreign         write into a sidecar another application owns
        --print-sidecars        score and print what each sidecar would become, write nothing
        --limit <N>             smoke test on the first N files
        --dry-run               write previews to ./imagecull-previews, no HTTP
        --timeout <SECS>        default 120
        --quiet | --verbose
        --config <FILE>         read this config file instead of the discovered one
        --no-config             ignore any config file; built-in defaults only

imagecull groups <DIR> [OPTIONS]   # group by moment, cull each group with clef, mark the reds
        --out <FILE>            the measures CSV, default ./groups.csv
        --group-gap <SECS>      default 2.0
        --group-similarity <COSINE>   default 0.90
        --group-exposure-stops <STOPS>  default 2.0; 0 asks for one exposure exactly
        --max-edge <PX>         one image in a comparison, default 512
        --jpeg-quality <1-100>  one image in a comparison, default 88
    -j, --jobs <N>              frames read at once, default one per core
        --cull-jobs <N>         groups culled at once, default 4
        --recursive [BOOL]      also scan subdirectories
        --limit <N>             smoke test on the first N files
        --no-cull               group and measure only: no request, no label
        --print-sidecars        ask clef, print what each sidecar would become, write nothing
        --allow-foreign         write into a sidecar another application owns
        --quiet | --verbose
        --endpoint / --model / --timeout / --config / --no-config   as `score`

imagecull models               # GET /v1/models, flag decision models
imagecull doctor [FILE.nef]    # server reachability + preview extraction self-test
imagecull calibrate <DIR>      # (M4, optional) compare scores vs existing xmp:Rating
```

`--config` and `--no-config` are on every subcommand (`§11.1`), and `§11.1`
lists, per command, which of these flags also have a config key.

One line per frame — `142/400  DSC_0555.NEF  4★  812ms  eta 3m10s`, `5★ kept` for a
frame whose sidecar already had a rating, and `duplicate of DSC_0540.NEF` when an
identical preview paid for the request — plus a final summary: how much of the
folder this run rated, counts by status, how many were skipped or deduplicated,
the rating's mean/median and histogram, what the scan left out, and the wall time. These lines print whether or not stdout is a
terminal: a frame line ends in a newline, so a redirected log is the same text a
terminal received, and `--quiet` is the switch for a script that wants none of
it. `--verbose` adds the preview's size, the answers at full precision and each
failure's message.

`--version` prints the version, the default built-in's name and its rubric hash.

### 11.1 Config files

Values that stay true across runs can come from a TOML file instead of being
retyped as flags. Keys are grouped into six sections, each owning what one
command needs: `[server]` (`endpoint`, `model`, `timeout`),
`[preview]` (`max_edge`, `jpeg_quality`), `[scan]` (`recursive`), `[outputs]`
(`json_scores`), `[run]` (`rubric`, `shoot_context`, `dedupe`), and `[group]`
(`gap`, `similarity`, `exposure_stops`, `max_edge`, `jpeg_quality`).
`[run] rubric` holds exactly what `--rubric` holds: a built-in's name or a path.
`imagecull.toml.example` is the commented reference; ADR 0003 records why this
shape, ADR 0004 the three built-ins, ADR 0010 the `[group]` section, and ADR 0014
why a comparison has a preview pair of its own.

**Which command reads which key.** `score` resolves every key. `groups` resolves
`[server]` (the alias and endpoint a cull asks), all of `[group]` — including
`max_edge` and `jpeg_quality`, the pair every image in a comparison is rendered
at — `[scan] recursive` and `[run] shoot_context`. It rates nothing and compares
nothing one frame at a time, so `[preview]`, `[run] rubric`, `[run] dedupe` and
`[outputs] json_scores` are not its business: a rating's preview size and a
comparison's are different payloads, and one key cannot be both. Key names are
always accepted; a key another command reads is simply not consulted by this one.

**Discovery.** Exactly one file is read. The first of these that exists wins
whole: `--config <FILE>`, `./imagecull.toml`,
`$XDG_CONFIG_HOME/imagecull/config.toml`,
`~/.config/imagecull/config.toml`, the built-in defaults. Files never layer,
and there are no environment-variable overrides. `--no-config` skips discovery
entirely.

**Precedence.** Flag > config file > built-in default, per key.

**What a file cannot set.** The shoot folder, `--dry-run`, `--force`,
`--allow-foreign`, `--print-sidecars`, `--limit`, `--jobs`, `--out`, `--quiet`,
`--verbose` — those describe the invocation, not the shoot. `--recursive`, the
three `--group-*` thresholds, `--max-edge` and `--jpeg-quality` do have keys,
and the `--recursive` flag takes an explicit `true`/`false` so a file that turned
recursion on can be turned off for one run. `--max-edge` and `--jpeg-quality`
name the preview of the command that was typed: `[preview]` for `score` and
`doctor`, `[group]` for `groups`.

**Strictness.** Unknown keys and unknown sections are fatal, values are
validated to the same ranges the flags enforce, an unreadable `--config` target
is fatal, and a malformed file is fatal with the parser's line and column. Exit
code 2 (`§10`), never a warning-and-continue.

**Paths.** Relative paths in the file resolve against the file's own directory.
A built-in's name is not a path and is not anchored: `rubric = "holiday"` means
the built-in wherever the file sits, and `rubric = "./holiday"` means a file.

**Provenance.** The file is read by `score`, `groups`, `doctor` and `models`,
so a preflight certifies the pipeline the run will actually use. When a file was
loaded, the run summary's provenance line prints it beside the rubric hash and
the model alias:
`rubric general/69139540 · model clef-flash-oQ4e · config /Volumes/Shoot/imagecull.toml`
— the name before the slash only when a built-in was selected by name, since a
file has no name to give and the hash already identifies it.
The file's content hash is **not** carried anywhere in a run's output; see ADR
0003's consequences for what that leaves open.

---

## 13. v3 — Tauri GUI (macOS)

No new scoring logic: `imagecull-gui` links `imagecull-core` and replaces the `Reporter` with Tauri events.

| pane | content |
|---|---|
| grid | thumbnail wall sorted by `stars` descending; each cell shows the rating and its confidence. Thumbnails come from the same embedded preview that was judged, so **what the model saw is what you see.** |
| filters | star range, `conf_stars < 0.5` |
| detail | preview + the five rating probabilities as a small bar chart; the single most useful debugging view in the whole app |
| cull | keyboard `0`–`5` overrides, `P`/`X` pick/reject, then *Write XMP* for the visible selection |
| run | folder picker, live progress row, cancel button, resume-on-relaunch |

Why this is cheap: no RAW decode anywhere (previews are already JPEGs), the core is synchronous and cancellable, and the cache of extracted previews that v1's `--dry-run` produces is exactly the GUI's thumbnail cache.

---

## 14. Performance budget

Published per-request latencies on an M5 Max (text, 1k tokens): **clef-flash 9B ≈ 0.31 s**, **clef 27B 4-bit ≈ 1.4 s**. An image adds tokens rather than flops-as-text, so budget **0.5–1.5 s/frame for Flash** and **2–4 s/frame for 27B**, plus ~40 ms local work (extract + downscale + encode).

| shoot | Flash | 27B |
|---|---|---|
| 100 NEFs | ~1–2 min | ~4–7 min |
| 400 NEFs | ~5–10 min | ~15–25 min |

Memory/VRAM is the server's problem, not the client's: Flash needs ~16 GB RAM minimum, 27B ~32 GB. `doctor` reports which is actually installed rather than assuming.

Bottleneck reality: **the server serialises requests**. A rating run is therefore sequential and client-side optimism buys little; the cull's `--cull-jobs` overlaps only the local extraction, downscale and encode with the previous group's server wait. The real lever is `--no-dedupe` off (burst dedupe) and rubric length, both handled above.

---

## 15. Determinism and caching

Decision models have no sampling temperature — the answer is a deterministic
function of `(weights, image bytes, question schema)`. Therefore:

- Re-running the same folder with the same rubric and model over the same raws
  **must** produce the same ratings, and therefore byte-identical sidecars except
  for `imagecull:scoredAt`. This is an acceptance test (§16), not an aspiration.
- **Dedupe** reuses an answer for a byte-identical preview within one run, keyed
  on `(preview_sha256, state, rubric_hash, model)`. The key covers the state the
  model reads — the task, the shoot context and the bytes — but not the filename,
  which is the photographer's label rather than something the picture shows (ADR
  0002). The cache is in-process; it dies with the run.
- **The sidecar is the only cross-run memory.** A frame whose sidecar already
  carries a rating is skipped, so a re-run costs nothing for it. There is no
  ledger, no resume flag and no re-derivation of an old run's numbers: a lost
  sidecar costs one request.

---

## 16. Test strategy

### 16.1 Fixtures (`tests/fixtures/`)

Committed: none by default (NEFs are large). A `fetch-fixtures.sh` + `README` describes what to place there:

| file | purpose |
|---|---|
| `landscape.nef` | baseline, orientation 1 |
| `portrait.nef` | orientation 6/8 — the rotation regression test |
| `high_iso.nef` | expect a low rating |
| `blurred.nef` | expect a low rating even when the moment is strong |
| `burst_a.nef`, `burst_b.nef` | identical preview bytes for dedupe (or use `--no-dedupe` fixtures) |
| `broken.nef` | truncated file → `unreadable`, no panic |

### 16.2 Layers

- **Unit:** orientation application (all 8 tags), largest-preview selection by area, downscale never upscales, base64 round-trip, CSV escaping for the measures report (filenames with commas/quotes/unicode), the sidecar merge (byte-preservation of foreign XML, in-place rewrite, idempotency, refusal), rubric canonicalisation/hash stability.
- **Contract:** `httpmock` serves recorded `/v1/systemone` bodies — including adversarial ones (`probabilities` as array *and* as object, `choice` as string, a key off the legend) — and asserts our deserialisation. No network in CI.
- **Golden:** the summary block and the frame lines for the fixture set against the mock server, plus the exact sidecar document a fixture frame produces.
- **Live (ignored by default, `--ignored`):** one real NEF against a real oMLX; asserts `status=ok` and that the rating lands in `1..=5`.
- **Acceptance:**
  1. 400-file run completes; every row has a status.
  2. `Ctrl-C` at 50% then a plain re-run makes **no requests** for the frames already rated and leaves their sidecars byte-identical.
  3. Two full runs are byte-identical.
  4. `--dry-run` previews are upright for portrait fixtures.
  5. Zero writes into the input folder when `--xmp` is absent (verified by mtime scan).

---

## 17. Risks and open questions

| # | risk | mitigation |
|---|---|---|
| R1 | **Which Clef checkpoint?** 27B scores higher; Flash is 4× faster and runs in 16 GB. Unknown which is installed. | `doctor`/`models` report the alias and size; rubric hash includes the model so results never silently mix. Ask before M4. |
| R2 | **Is Clef actually good at an absolute keeper rating?** One 5-way choice is a *new* use of a model trained on typed decisions. | M4 calibration against `xmp:Rating` the user already applied to past shoots (§11 `calibrate`). No tuning before evidence. |
| R3 | The rating may saturate (everything 3–4). | Watch the histogram in the run summary; if the spread is flat, the rubric's instructions and level glosses need work, not the thresholds — there are no thresholds. |
| R4 | `/v1/systemone` is new surface that may drift. | Defensive parsing, raw answers preserved, contract tests, version check in `doctor`. |
| R5 | Embedded preview ≠ the edit the user would make. | Deliberate: the preview is the camera's rendering and the thing a cull actually judges. Full RAW demosaic stays a non-goal. |
| R6 | Long runs on battery / sleep kills the server. | The skip rule finishes the shoot for the price of what is left; `doctor` preflight; `caffeinate -i` suggested in docs. |
| R7 | Sidecar writes are destructive if wrong. | §9.2 merge, §9.3 atomic write and backups, §9.6 preview, `xmp_foreign` detection, skip-by-default. |
| R8 | Rubric is the product; wording churn invalidates ratings. | Rubric in TOML, hashed into every sidecar, and `--force` is explicit — but expect several re-runs during M4. Budget for it. |

Open questions for the user:
1. Which Clef build is installed (27B / Flash, 4-bit / 8-bit), and which oMLX version?
2. Aesthetic target: photojournalistic keepers, or technically-clean deliverables? It changes the rubric's instructions.
3. Are there existing star-rated shoots we can use as calibration ground truth (with XMP already written)?
4. Should a burst's identical previews share one sidecar rating (cheap) or be rated one at a time (honest about each file)? Today the first frame pays and the rest reuse its answer within the run.

---

## 18. Milestones

| M | scope | done when |
|---|---|---|
| **M0** | workspace scaffold, `--version`, `doctor`, error taxonomy | `imagecull doctor` explains a missing/old oMLX clearly |
| **M1** | ingest + preview extraction + orientation + downscale + `--dry-run` | portrait fixture exports upright at 1568 px |
| **M2** | systemone client + rubric + single-file score | one NEF prints its rating; contract tests green |
| **M3** | sidecar writes, skip rule, progress, summary | acceptance tests 1–5 pass |
| **M4** | real-shoot validation + rubric tuning + optional `calibrate` | scores agree with the user's own stars on a known shoot; rubric frozen |
| **M5** | Lightroom handoff: the written sidecars verified in LrC | 20 sidecars reviewed with `--print-sidecars` and 20 more verified in the catalogue |
| **M6** | v3 Tauri GUI | grid + cull + write-XMP without a terminal |

Sequencing note: M4 is the long pole and it is **not** engineering — it is prompt/schema iteration against real photos. Getting to M3 fast is the point of deferring XMP.

---

## 19. Appendix — dependencies

| crate | role |
|---|---|
| `clap` 4 (derive) | CLI |
| `serde`, `serde_json` | wire types, `--json-scores` |
| `toml` | rubric config |
| `raw-files` | NEF preview extraction + orientation (primary) |
| `image` | JPEG decode/encode, `apply_orientation` |
| `fast_image_resize` | Lanczos3 downscale |
| `base64` | data URI payloads |
| `ureq` | blocking HTTP to `/v1/systemone` |
| `sha2` | preview/rubric hashing |
| `tempfile` | the sidecar writer's temp file and rename |
| `indicatif`, `tracing`, `tracing-subscriber` | progress and logs |
| `thiserror` | error taxonomy |
| *v3:* `tauri` | macOS GUI |
| *dev:* `httpmock`, `insta`, `assert_cmd`, `predicates`, `tempfile` | contract, golden, CLI tests |

Pin exact versions at M0 and commit `Cargo.lock`. `raw-files` and the `image`/`apply_orientation` API are the two places where an upstream change lands directly in our critical path.

## 20. Reference

- oMLX decision-model support: `jundot/omlx` PR #4315 — `POST /v1/systemone`, question types, response shape, `truncate` semantics, detection via `joint_head.safetensors`.
- Clef model card (`Cloudflare/clef`, `mlx-community/clef-4bit`, `Jundot/clef-oQ4e`): question schema, 16,384-token context, latency and memory tables, one-record-per-call.
- XMP: `xmp:Rating` / `xmp:Label` portability; `xmpDM:pick` + `xmpDM:good` flagging as written by LrC ≥ 13.2; darktable's `<raw>.xmp` history-file ownership.
