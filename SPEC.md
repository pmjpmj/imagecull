# imagecull — Specification

**Status:** Draft v0.1 · **Target platform:** macOS on Apple Silicon · **Language:** Rust
**One-liner:** Score a folder of Nikon mirrorless NEFs with a local Clef decision model over oMLX, and (eventually) write the cull result into XMP sidecars so Lightroom Classic opens pre-culled.

---

## 1. Problem

A shoot lands as 300–800 NEFs. All of them are technically fine; maybe 10% are the keepers. Judging that today means opening every frame in a RAW editor and tapping stars — an hour or two of pure triage, before any editing starts.

The task is *decision*, not generation: each frame gets a few numbers and a verdict. That maps exactly onto Cloudflare's **Clef**, served locally by **oMLX** on `POST /v1/systemone` — it reads a state plus a schema of typed questions and returns a probability for every allowed option in **one forward pass**, with no generated text to parse.

## 2. Goals

| # | Goal |
|---|---|
| G1 | Point at a folder; get one row per Nikon mirrorless RAW with scores, no manual work. |
| G2 | Preview-quality input: never decode RAW sensor data; use the JPEG the camera embedded. |
| G3 | Raw scores recorded separately and **never combined** — `overall`, `technical`, `composition`, `impact`. |
| G4 | Star rating and like/dislike come from the model itself, not from post-hoc thresholds. |
| G5 | Interruptible and resumable; a 400-file run survives Ctrl-C, sleep, or a server restart. |
| G6 | Deterministic: same image + same rubric + same model ⇒ same numbers, so results are cacheable. |
| G7 | v1 is read-only. Nothing is written into the photo folder except the CSV the user asked for. |
| G8 | The core is a library crate, so the eventual macOS GUI is a new front end, not a rewrite. |

## 3. Non-goals (v1)

- No RAW demosaic, no editing, no colour pipeline, no output rendering.
- No catalog database, no library management, no import/export to Lightroom's catalog.
- No XMP writing (v2, §11), no GUI (v3, §12).
- No video (`/v1/systemone` does not accept video over HTTP).
- No cloud calls, no telemetry, no server management (starting/stopping oMLX).
- No batching. oMLX serializes decision requests; pretending otherwise is a lie in the UX.

## 4. Locked decisions

| Decision | Choice | Why |
|---|---|---|
| Language | **Rust** (clap CLI → Tauri GUI) | Strict types for the systemone request/answer contract; one fast binary; Tauri reuses the same core crate for the Mac GUI. |
| Preview source | **Largest embedded JPEG** in the NEF | Camera-rendered, already sharpened; no colour pipeline to build. |
| Scoring shape | **4 independent `score` questions, no combination** | Different failure modes stay diagnosable; no invented weighting. |
| Stars / verdict | **Extra `choice` questions in the same forward pass** | Clef is a decision model; a 5-way and a 3-way choice are its native mode. Cost ≈ 0 latency. |
| Sidecar target (v2) | **Lightroom Classic / ACR** — `DSC_0001.xmp` | LrC 13.2+ honours both `xmp:Rating` and `xmpDM:pick` from sidecars. |
| Run model | **Sequential + resumable JSONL ledger** | oMLX serializes requests anyway; the win is crash-safety, not parallelism. |
| v1 output | **CSV only** | Prove the numbers on a real shoot before anything writes into the library. |

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
                    │            6 questions, 1 forward pass)      │
                    └───────────────┬──────────────────────────────┘
                                    ▼
                    ┌──────────────────────────────────────────────┐
                    │ record     ledger.jsonl (append, fsync)      │
                    │            → scores.csv (rewrite at end)     │
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
      src/ledger.rs             # append-only JSONL, resume, corruption repair
      src/report.rs             # CSV writer
      src/error.rs              # ErrorKind taxonomy
    imagecull-cli/              # bin: imagecull
      src/main.rs
      src/progress.rs           # indicatif + tracing
    imagecull-xmp/              # (v2) sidecar merge/write
    imagecull-gui/              # (v3) Tauri app, depends on core only
  rubric.default.toml
  tests/fixtures/               # see §13.1
```

**Hard rule:** `imagecull-core` takes a `Reporter` trait for progress and never touches stdout, TTY, or process exit. That is what makes the Tauri port cheap.

---

## 6. Input handling

### 6.1 File selection

- Extensions (case-insensitive): `.nef`, `.nrw` (Nikon 1 / Coolpix), `.dng` (Nikon Z can be set to DNG). Anything else is skipped with a `skipped_extension` count, never an error.
- Non-recursive by default; `--recursive` walks subdirectories.
- Sorted by filename (natural/NFC-normalised) so runs are reproducible and the CSV row order is stable.
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
- Requests are serialized server-side; the client sends **one at a time**, never a concurrent burst.

### 7.2 Response

Per the oMLX implementation, one answer per question:

| type | answer fields |
|---|---|
| `noul` | probability of yes |
| `choice` | `choice`, `probabilities`, `confidence` |
| `score` | `score`, `probabilities`, `confidence`, `legend` |

Plus `usage.input_tokens` (and `usage.latency_ms` on the reference server).

Parsing rules (defensive — this contract is new surface):

- `answer.choice` is a **string** key from `criteria`; parse to `u8` for stars.
- `answer.score` is a **float** (expected value over the legend). Never round it into the CSV; keep the model's precision.
- `probabilities` may arrive as an ordered array or as an object keyed by option. Deserialize as `serde_json::Value` and only *display* it; **never** derive the recorded answer from it. `score`/`choice`/`confidence` are the source of truth.
- The full raw answer JSON is preserved for `--json-scores`, so nothing is lost if the shape shifts.
- Treat an answer key that is missing from `answers` as `status=bad_response` for that file.

### 7.3 Client

`ureq` (blocking) + `serde_json`. No tokio in v1: the workload is strictly serial, and a blocking client keeps `imagecull-core` trivially portable into Tauri (which can call it from a background thread). Timeouts: connect 5 s, read 120 s (`--timeout`), overridable because 27B Clef on a long prompt can take tens of seconds.

---

## 8. The rubric

Six questions, one forward pass. Encoded as TOML so wording can be tuned without a rebuild; an embedded default ships via `include_str!`, and `--rubric` overrides it. The effective rubric is hashed (§8.3).

### 8.1 The four scores

Each is an ordered `criteria` array `"0"…"10"`, so `score` is the legend index and the raw answer is a float in `[0, 10]`. **These are never combined and never rescaled to 100.**

| id | instructions (draft) |
|---|---|
| `overall` | "You are a photo editor culling a shoot. Rate this photograph as a keeper. 0 = delete, unusable. 5 = keep as a record. 8 = good enough to deliver to a client. 10 = portfolio-grade, the best frame of the shoot." |
| `technical` | "Judge technical image quality only: focus on the intended subject, motion blur, noise, exposure, white balance, lens and sensor artifacts, clipped highlights or crushed shadows. Ignore whether the subject is interesting." |
| `composition` | "Judge framing and graphic design only: subject placement, balance, leading lines, negative space, level horizon, distracting background or edge intrusions, crop. Ignore technical quality and the appeal of the subject." |
| `impact` | "Judge emotional and visual impact only: does the frame capture a distinctive moment, gesture, expression or quality of light that makes someone stop and look? Ignore technical quality and rule-based composition." |

Splitting them is what makes the output actionable: a blurry but decisive moment (`technical` 3, `impact` 9) is a different decision from a sterile, perfectly sharp frame (`technical` 9, `impact` 3), and a single blended number erases that distinction.

### 8.2 Stars and verdict

```jsonc
"stars": {
  "type": "choice",
  "instructions": "Applying a typical star rating to this photograph, how many stars?",
  "criteria": [
    {"key": "1", "description": "reject — unusable, out of focus, or a duplicate"},
    {"key": "2", "description": "poor — technically fine but nothing worth keeping"},
    {"key": "3", "description": "ok — keep as a record of the moment"},
    {"key": "4", "description": "strong — good enough to deliver to a client"},
    {"key": "5", "description": "excellent — hero frame, portfolio or print"}
  ]
},
"verdict": {
  "type": "choice",
  "instructions": "Would you personally keep this photograph?",
  "criteria": [
    {"key": "dislike", "description": "I would not keep it"},
    {"key": "neutral", "description": "I could keep it or not"},
    {"key": "like",    "description": "I would keep it and show it"}
  ]
}
```

`criteria` for `choice` is an **array of `{key, description}`** in TOML rather than a table, because TOML tables do not guarantee key order and the option order matters to the model's legend.

On the wire the same authored form is rendered per question type: a `score` sends the ordered list of level descriptions (index = key), a `choice` sends an object keyed by option. Those are the shapes the Clef heads read; see `docs/adr/0001-rubric-renders-two-criteria-shapes.md`.

### 8.3 Rubric hash

`sha256` over the canonicalised question schema (ids, types, instructions, criteria, in declaration order) + `model` alias. Recorded in the ledger header, in the CSV's `rubric_hash` column, and (v2) in the XMP sidecar. `calibration` and `resume` both key off it: changing wording invalidates cached scores by design, and the run refuses to silently mix rubrics.

### 8.4 Disagreement is signal, not error

`stars` and `overall` will occasionally conflict. **Do not reconcile them.** Both are recorded; a `--report` summary prints the disagreement rate. If it is high, the rubric wording needs work (§13.2), not a fudge factor.

---

## 9. Output

### 9.1 CSV (v1 deliverable)

`--out scores.csv`, RFC 4180, UTF-8, header row, `\n`.

| column | example | notes |
|---|---|---|
| `file` | `DSC_0421.NEF` | Relative to the input dir unless `--absolute`. |
| `overall` | `6.42` | Model's expected value, full precision. Empty on error. |
| `technical` | `7.81` | "" |
| `composition` | `5.34` | "" |
| `impact` | `4.96` | "" |
| `stars` | `4` | Integer 1–5. |
| `verdict` | `like` | `like` / `neutral` / `dislike`. |
| `conf_overall` | `0.41` | Top-option probability. A low value here means "the model is unsure", which is the cheapest failure signal available. |
| `conf_stars` | `0.63` | "" |
| `conf_verdict` | `0.88` | "" |
| `latency_ms` | `812` | Server-side if provided, else client-measured. |
| `input_tokens` | `1327` | Prompt size. |
| `status` | `ok` | See §10 taxonomy. |
| `error` | `` | Short message when `status != ok`. |
| `duplicate_of` | `DSC_0418.NEF` | Set when the identical preview was already scored. |
| `rubric_hash` | `9f3c…` | First 8 hex chars. |
| `model` | `clef-flash-oQ4e` | Whatever the server alias is. |

Additions over the original ask (4 scores) are justified: `stars`/`verdict` are the v2 XMP payload and cost nothing now; confidences are the only honest measure of trust; `status`/`error` keep a 400-file CSV self-explaining.

`--json-scores scores.jsonl` writes the full raw `/v1/systemone` answer per file for debugging and future re-derivation.

### 9.2 Ledger (`--resume`)

`scores.csv.ledger.jsonl` next to the output file:

```jsonc
{"v":1,"kind":"header","rubric_hash":"9f3c…","model":"clef-flash-oQ4e","endpoint":"http://127.0.0.1:8000","started_at":"2026-11-08T10:12:44Z"}
{"v":1,"kind":"row","file":"DSC_0421.NEF","size":48211344,"mtime_ns":1762593132211000000,"preview_sha256":"a1b2…","status":"ok","outcome":{…},"at":"…"}
```

- Append + `flush` per record. Recovery scans for the last parseable line and truncates garbage after it — a `SIGKILL` mid-write must not poison the ledger.
- Resume key: `(file, size, mtime_ns)`. Any change ⇒ re-score, because the file changed.
- Dedupe: `preview_sha256` over the *sent* JPEG bytes. Identical preview ⇒ reuse the earlier outcome (deterministic model, §6.4), so a 20-frame burst costs one request. Disable with `--no-dedupe`.
- `--fresh` discards the ledger. A rubric-hash mismatch without `--fresh` is a hard error with a clear message, never a silent mix.
- The CSV is rewritten from the ledger after every N rows (default 25) and at exit, so an interrupted run still leaves a usable CSV.

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
| `bad_response` | answer missing/typed wrong | record raw JSON in the JSONL, continue |

Retries: 3 attempts with 0.5/1/2 s backoff, only for `server_unreachable`, `timeout`, and 5xx. Never retry 4xx (a wrong rubric will fail identically 400 times).

**Preflight** (before the first file): `GET /v1/models`, check the alias exists, and if the server exposes `/v1/models/status` confirm `model_type == "decision"`. Fail fast with an actionable message rather than 400 files of the same 400.

Exit codes: `0` all ok · `1` completed with failed rows · `2` fatal (bad args, dir missing, server unreachable, rubric rejected) · `3` interrupted (SIGINT, ledger intact).

---

## 11. CLI surface (v1)

```
imagecull score <DIR> [OPTIONS]
    -o, --out <FILE>            default: ./scores.csv
        --endpoint <URL>        default: http://127.0.0.1:8000
    -m, --model <ALIAS>         default: from rubric.toml
        --rubric <FILE>         default: embedded
        --shoot-context <STR>   free text passed into state.shoot_context
        --max-edge <PX>         default 1568
        --jpeg-quality <1-100>  default 88
        --recursive
        --resume | --fresh
        --json-scores <FILE>
        --no-dedupe
        --limit <N>             smoke test on the first N files
        --dry-run               write previews to ./imagecull-previews, no HTTP
        --absolute              absolute paths in the CSV
        --timeout <SECS>        default 120
        --quiet | --verbose
        --config <FILE>         read this config file instead of the discovered one
        --no-config             ignore any config file; built-in defaults only

imagecull models               # GET /v1/models, flag decision models
imagecull doctor [FILE.nef]    # server reachability + preview extraction self-test
imagecull calibrate <DIR>      # (M4, optional) compare scores vs existing xmp:Rating
```

`--config` and `--no-config` are on all three subcommands (`§11.1`).

Progress (TTY only): `[142/400] DSC_0555.NEF  overall 6.4★4 · 0.8s · eta 3m10s`, plus a final summary — counts by status, mean/median per score, star histogram, disagreement rate, total wall time.

`--version` prints version + rubric hash.

### 11.1 Config files

Values that stay true across runs can come from a TOML file instead of being
retyped as flags. Keys are grouped into five sections that map one-for-one onto
the core types they fill: `[server]` (`endpoint`, `model`, `timeout`),
`[preview]` (`max_edge`, `jpeg_quality`), `[scan]` (`recursive`), `[outputs]`
(`csv`, `json_scores`, `absolute`), `[run]` (`rubric`, `shoot_context`,
`dedupe`, `csv_rewrite_every`). `imagecull.toml.example` is the commented
reference; ADR 0003 records why this shape.

**Discovery.** Exactly one file is read. The first of these that exists wins
whole: `--config <FILE>`, `./imagecull.toml`,
`$XDG_CONFIG_HOME/imagecull/config.toml`,
`~/.config/imagecull/config.toml`, the built-in defaults. Files never layer,
and there are no environment-variable overrides. `--no-config` skips discovery
entirely.

**Precedence.** Flag > config file > built-in default, per key.

**What a file cannot set.** The shoot folder, `--dry-run`, `--resume`,
`--fresh`, `--limit`, `--quiet`, `--verbose` — those describe the invocation,
not the shoot.

**Strictness.** Unknown keys and unknown sections are fatal, values are
validated to the same ranges the flags enforce, an unreadable `--config` target
is fatal, and a malformed file is fatal with the parser's line and column. Exit
code 2 (`§10`), never a warning-and-continue.

**Paths.** Relative paths in the file resolve against the file's own directory.

**Provenance.** The file is read by `score`, `doctor` and `models`, so a
preflight certifies the pipeline the run will actually use. When a file was
loaded, the run summary's provenance line prints it beside the rubric hash and
the model alias: `rubric 69139540 · model clef-flash-oQ4e · config /Volumes/Shoot/imagecull.toml`.
The file's content hash is **not** in the ledger header; see ADR 0003's
consequences for what that leaves open.

---

## 12. v2 — XMP sidecars for Lightroom Classic

**Not built in v1.** Specced now because it constrains v1's data model (all six questions, confidences, rubric hash and model must be recorded even though v1 only prints CSV).

### 12.1 What actually survives the handoff

| field | XMP property | LrC reads it | other apps |
|---|---|---|---|
| stars | `xmp:Rating` `0`–`5` | yes, at import or *Read Metadata from File* | yes (Bridge, dt, digiKam) |
| rejected | `xmp:Rating` `-1` | yes | yes |
| like tint | `xmp:Label` (`Green`/`Red`/none) | yes | partially (one colour label) |
| pick / reject flag | `xmpDM:pick` `1`/`0`/`-1` + `xmpDM:good` | yes, LrC ≥ 13.2 only | mostly ignored |
| the four raw scores | our own namespace | ignored (preserved) | ignored (preserved) |

Design consequence: **stars are the portable channel, the flag is a bonus, and the scores live in a namespace nobody else touches** so nothing is lost or mangled on the round trip.

### 12.2 Filename

`DSC_0001.NEF` → **`DSC_0001.xmp`** (base name, extension replaced, `.NEF`/`.nef` both handled). Never `DSC_0001.NEF.xmp` — that name belongs to darktable, and writing it would put our metadata inside another app's history file.

### 12.3 Merge, not overwrite

A sidecar may already carry ACR develop settings (`crs:`), keywords, GPS or darktable history. Rule: **byte-preserve everything we do not own.** Implementation: parse for validation (`roxmltree`), then perform targeted edits — remove only our own properties, then insert ours into the first `rdf:Description` — rather than parse-and-reserialise the whole document (which would reorder and re-quote foreign XML).

```xml
<x:xmpmeta xmlns:x="adobe:ns:meta/" x:xmptk="imagecull 0.1">
 <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Description rdf:about=""
    xmlns:xmp="http://ns.adobe.com/xap/1.0/"
    xmlns:xmpDM="http://ns.adobe.com/xmp/1.0/DynamicMedia/"
    xmlns:imagecull="https://ns.imagecull.app/1.0/"
    xmp:Rating="4"
    xmp:Label="Green"
    xmpDM:pick="1"
    xmpDM:good="True"
    imagecull:overall="6.42"
    imagecull:technical="7.81"
    imagecull:composition="5.34"
    imagecull:impact="4.96"
    imagecull:stars="4"
    imagecull:verdict="like"
    imagecull:confStars="0.63"
    imagecull:model="clef-flash-oQ4e"
    imagecull:rubricHash="9f3c1a77"
    imagecull:scoredAt="2026-11-08T10:14:02Z"
    imagecull:version="0.1.0" />
 </rdf:RDF>
</x:xmpmeta>
```

Mapping: `stars` → `xmp:Rating`; `verdict=like` → `xmp:Label=Green` + `xmpDM:pick=1` + `xmpDM:good=True`; `verdict=dislike` → `xmp:Label=Red` + `xmpDM:pick=-1`; `neutral` → no label, `pick=0`. **Never** set `xmp:Rating=-1` automatically — that is the user's destructive reject, and it belongs to them.

### 12.4 Safety rules

1. **Opt-in only:** requires `--xmp`; default is unchanged from v1.
2. **`--dry-run`** prints a unified diff of the sidecar change; nothing is written.
3. **Atomic write:** temp file in the same directory + `rename`.
4. **Backup:** if a sidecar already existed, `DSC_0001.xmp.bak` (only with `--backup`, and never overwriting an older `.bak`).
5. **Never clobber a human rating:** if the existing `xmp:Rating` differs from ours and is non-zero, skip and report `skipped_existing_rating`. Override with `--overwrite-rating`.
6. **Foreign-app guard:** if the sidecar contains a `darktable:` namespace or `darktable:history`, refuse to write unless `--allow-foreign` — darktable owns that file and will rewrite it.
7. **Read-only raws:** the NEF is opened read-only and never modified. XMP-in-RAW is a non-feature.

### 12.5 First-run protocol

Run `--xmp --dry-run` over the shoot, review 20 sidecars by eye, then run for real. State this in the README; the failure mode being guarded against is a wrong legend quietly labelling an entire library.

---

## 13. v3 — Tauri GUI (macOS)

No new scoring logic: `imagecull-gui` links `imagecull-core` and replaces the `Reporter` with Tauri events.

| pane | content |
|---|---|
| grid | thumbnail wall sorted by `overall` descending; each cell shows 4 scores, stars, verdict, and a confidence delta. Thumbnails come from the same embedded preview that was scored, so **what the model saw is what you see.** |
| filters | star range, verdict, min `impact`, "disagreements only", `conf_stars < 0.5` |
| detail | preview + the 6 probability distributions as small bar charts; the single most useful debugging view in the whole app |
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

Bottleneck reality: the run is **serialized by the server**, so client-side optimisation (prefetch, parallelism) buys little. The real lever is `--no-dedupe` off (burst dedupe) and rubric length, both handled above.

---

## 15. Determinism and caching

Decision models have no sampling temperature — the answer is a deterministic function of `(weights, image bytes, question schema)`. Therefore:

- Re-running the same folder with the same rubric and model **must** produce a byte-identical CSV. This is an acceptance test (§16), not an aspiration.
- The cache key is `(preview_sha256, rubric_hash, model)`. The ledger currently keys on the file identity; the rubric hash guards schema drift.
- Recording `model` + `rubric_hash` in every row means any CSV can be reproduced later, or invalidated deliberately.

---

## 16. Test strategy

### 16.1 Fixtures (`tests/fixtures/`)

Committed: none by default (NEFs are large). A `fetch-fixtures.sh` + `README` describes what to place there:

| file | purpose |
|---|---|
| `landscape.nef` | baseline, orientation 1 |
| `portrait.nef` | orientation 6/8 — the rotation regression test |
| `high_iso.nef` | expect low `technical` |
| `blurred.nef` | expect low `technical`, maybe high `impact` |
| `burst_a.nef`, `burst_b.nef` | identical preview bytes for dedupe (or use `--no-dedupe` fixtures) |
| `broken.nef` | truncated file → `unreadable`, no panic |

### 16.2 Layers

- **Unit:** orientation application (all 8 tags), largest-preview selection by area, downscale never upscales, base64 round-trip, CSV escaping (filenames with commas/quotes/unicode), ledger truncation repair, rubric canonicalisation/hash stability.
- **Contract:** `httpmock` serves recorded `/v1/systemone` bodies — including adversarial ones (`probabilities` as array *and* as object, float `score`, `choice` as string) — and asserts our deserialisation. No network in CI.
- **Golden:** `insta` snapshots of the CSV for the fixture set against the mock server.
- **Live (ignored by default, `--ignored`):** one real NEF against a real oMLX; asserts `status=ok` and that all four scores land in `[0,10]`.
- **Acceptance:**
  1. 400-file run completes; every row has a status.
  2. `Ctrl-C` at 50% then `--resume` produces the same CSV as one uninterrupted run, with no duplicate rows.
  3. Two full runs are byte-identical.
  4. `--dry-run` previews are upright for portrait fixtures.
  5. Zero writes into the input folder when `--xmp` is absent (verified by mtime scan).

---

## 17. Risks and open questions

| # | risk | mitigation |
|---|---|---|
| R1 | **Which Clef checkpoint?** 27B scores higher; Flash is 4× faster and runs in 16 GB. Unknown which is installed. | `doctor`/`models` report the alias and size; rubric hash includes the model so results never silently mix. Ask before M4. |
| R2 | **Is Clef actually good at absolute aesthetic scores?** These four questions are a *new* use of a model trained on typed decisions. | M4 calibration against `xmp:Rating` the user already applied to past shoots (§11 `calibrate`). No tuning before evidence. |
| R3 | Composition/scoring may saturate (everything 6–8). | Watch the histogram in the run summary; if `composition` variance is low, the criteria descriptions need work, not the thresholds. |
| R4 | `/v1/systemone` is new surface that may drift. | Defensive parsing, raw answers preserved, contract tests, version check in `doctor`. |
| R5 | Embedded preview ≠ the edit the user would make. | Deliberate: the preview is the camera's rendering and the thing a cull actually judges. Full RAW demosaic stays a non-goal. |
| R6 | Long runs on battery / sleep kills the server. | Ledger + resume; `doctor` preflight; `caffeinate -i` suggested in docs. |
| R7 | Sidecar writes are destructive if wrong (v2). | §12.4 guards, dry-run, backups, foreign-app detection. |
| R8 | Rubric is the product; wording churn invalidates caches. | Rubric in TOML, hashed, `--fresh` explicit — but expect several re-runs during M4. Budget for it. |

Open questions for the user:
1. Which Clef build is installed (27B / Flash, 4-bit / 8-bit), and which oMLX version?
2. Aesthetic target: photojournalistic keepers, or technically-clean deliverables? It changes the `overall` instructions.
3. Are there existing star-rated shoots we can use as calibration ground truth (with XMP already written)?
4. Should bursts be collapsed in the CSV (one row per distinct frame, with `duplicate_of`) or always one row per NEF?

---

## 18. Milestones

| M | scope | done when |
|---|---|---|
| **M0** | workspace scaffold, `--version`, `doctor`, error taxonomy | `imagecull doctor` explains a missing/old oMLX clearly |
| **M1** | ingest + preview extraction + orientation + downscale + `--dry-run` | portrait fixture exports upright at 1568 px |
| **M2** | systemone client + rubric + single-file score | one NEF prints six answers; contract tests green |
| **M3** | ledger, resume, CSV, progress, summary | acceptance tests 1–5 pass |
| **M4** | real-shoot validation + rubric tuning + optional `calibrate` | scores agree with the user's own stars on a known shoot; rubric frozen |
| **M5** | v2 XMP writer for Lightroom Classic | dry-run diff reviewed, 20 sidecars verified in LrC |
| **M6** | v3 Tauri GUI | grid + cull + write-XMP without a terminal |

Sequencing note: M4 is the long pole and it is **not** engineering — it is prompt/schema iteration against real photos. Getting to M3 fast is the point of deferring XMP.

---

## 19. Appendix — dependencies

| crate | role |
|---|---|
| `clap` 4 (derive) | CLI |
| `serde`, `serde_json` | wire types, ledger, JSON scores |
| `toml` | rubric config |
| `raw-files` | NEF preview extraction + orientation (primary) |
| `image` | JPEG decode/encode, `apply_orientation` |
| `fast_image_resize` | Lanczos3 downscale |
| `base64` | data URI payloads |
| `ureq` | blocking HTTP to `/v1/systemone` |
| `csv` | RFC 4180 output |
| `sha2` | preview/rubric hashing |
| `indicatif`, `tracing`, `tracing-subscriber` | progress and logs |
| `thiserror` | error taxonomy |
| *v2:* `roxmltree` (+ targeted string edit) | XMP merge |
| *v3:* `tauri` | macOS GUI |
| *dev:* `httpmock`, `insta`, `assert_cmd`, `predicates`, `tempfile` | contract, golden, CLI tests |

Pin exact versions at M0 and commit `Cargo.lock`. `raw-files` and the `image`/`apply_orientation` API are the two places where an upstream change lands directly in our critical path.

## 20. Reference

- oMLX decision-model support: `jundot/omlx` PR #4315 — `POST /v1/systemone`, question types, response shape, `truncate` semantics, detection via `joint_head.safetensors`.
- Clef model card (`Cloudflare/clef`, `mlx-community/clef-4bit`, `Jundot/clef-oQ4e`): question schema, 16,384-token context, latency and memory tables, one-record-per-call.
- XMP: `xmp:Rating` / `xmp:Label` portability; `xmpDM:pick` + `xmpDM:good` flagging as written by LrC ≥ 13.2; darktable's `<raw>.xmp` history-file ownership.
