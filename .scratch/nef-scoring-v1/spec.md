# Spec — imagecull v1: score a NEF shoot with a local Clef decision model

Status: ready-for-agent
Date: 2026-11-08
Source: `SPEC.md` (draft v0.1) — this spec is the build contract for **v1 only** (M0–M4).

---

## Problem Statement

A shoot lands as 300–800 Nikon mirrorless NEFs. Almost all of them are technically fine; roughly 10% are the keepers. Judging that today means opening every frame in a RAW editor, hitting `1`–`5`, and moving on — one to two hours of pure triage before any actual editing starts, repeated for every shoot.

The work is *decision*, not generation: each frame needs a handful of numbers and a verdict. Doing it by hand is expensive, tedious, and inconsistent (a photographer's star rating drifts over a 400-frame session). Doing it in the cloud is unacceptable — the library is personal, large, and on a laptop.

Nothing in the current repo addresses this: the repo contains a design document and agent instructions, no code.

---

## Solution

A single Rust command, `imagecull score <DIR>`, that walks a shoot folder, extracts the camera's own embedded JPEG preview from each RAW, downscales it, and sends it to a **local** oMLX server hosting a Cloudflare **Clef** decision model with six typed questions in **one forward pass**: four independent raw scores (`overall`, `technical`, `composition`, `impact`), a 1–5 star `choice`, and a like/neutral/dislike `verdict`. Results land in `scores.csv` next to a resumable JSONL **ledger**, so a 400-file run survives Ctrl-C, sleep, or a server restart.

v1 is strictly **read-only** with respect to the photo library: no XMP, no RAW decode, no writes into the folder that was scored. It exists to prove the numbers on a real shoot before anything is allowed to write into a Lightroom library.

The scoring pipeline lives in a library crate with no stdout, TTY, or process-exit assumptions, so the eventual macOS GUI is a new front end over the same core rather than a rewrite.

---

## User Stories

### Point at a folder, get numbers

1. As a photographer, I want to point at a shoot folder and get one row per RAW, so that I can triage an entire shoot without opening a single file.
2. As a photographer, I want non-RAW files in the folder to be skipped and counted rather than fail the run, so that a shoot folder containing JPEGs, movies, or stray documents still works.
3. As a Nikon Z photographer shooting `.nef`, a Nikon 1/Coolpix shooter with `.nrw`, or a Z shooter configured for `.dng`, I want my format accepted, so that the tool matches how my camera is actually set up.
4. As a photographer, I want folder scanning to be non-recursive by default with an explicit opt-in for subfolders, so that a run covers exactly what I aimed it at.
5. As a photographer, I want files sorted by name in natural, Unicode-normalised order, so that the CSV row order is stable and reproducible across runs and machines.
6. As a macOS user, I want `._*` AppleDouble files, `.DS_Store` and `__MACOSX` contents ignored, with a count of how many were ignored, so that Finder noise never appears in my results.
7. As a photographer, I want hidden directories never descended into, so that caches and tool state aren't scored.
8. As a photographer, I want `--limit N` to score only the first N files, so that I can smoke-test a run on one shoot before committing an hour to it.

### What the model sees

9. As a photographer, I want the tool to use the JPEG the camera already embedded in the RAW, so that scoring needs no RAW decode, no colour pipeline, and no demosaic — and runs at preview speed.
10. As a photographer, I want the embedded preview chosen by largest pixel area rather than largest byte count, so that a smaller but higher-quality `Medium` preview is never picked over the `Full` one.
11. As a photographer shooting portrait frames, I want EXIF orientation applied *before* downscaling, so that the model judges an upright frame and not a sideways one.
12. As a photographer, I want the longest edge capped at a configurable size (default 1568 px), so that the payload stays small and inside the model's context without visible quality loss.
13. As a photographer, I want to tune `--max-edge` and `--jpeg-quality` when a shoot needs more detail or a run needs less bandwidth, so that the trade-off is mine to make.
14. As a photographer, I want `--dry-run` to write the exact bytes that would be sent into a previews folder, so that I can eyeball orientation, crop, and detail loss before burning a 400-file run — and so the same previews can seed the future GUI's thumbnail cache.
15. As a photographer, I want a file with no embedded preview to get a row marked `no_preview` and the run to continue, so that one odd file doesn't cost me the other 399.
16. As a photographer, I want a truncated or unreadable NEF to get an `unreadable` row without a panic, so that a corrupt card copy is visible and survivable.
17. As a photographer with small previews, I want the downscaler to never upscale, so that a 1024 px preview isn't blown up into fake detail the model then scores.

### The decision

18. As a photographer, I want four independent raw scores plus a star rating and a like/dislike verdict from a single forward pass, so that I get a whole cull in the time of one model call per frame.
19. As a photographer, I want `overall`, `technical`, `composition` and `impact` recorded separately and **never combined**, so that a blurry but decisive frame and a sterile but sharp frame stay distinguishable.
20. As a photographer, I want scores kept as the model's full-precision expected values, never rounded for the CSV, so that I can apply my own thresholds later without losing information.
21. As a photographer, I want the star rating to come from a 5-way choice question the model answers, not from thresholds I invented over the scores, so that the rating reflects the model's own judgement.
22. As a photographer, I want the like/neutral/dislike verdict to come from the model itself for the same reason.
23. As a photographer, I want a per-answer confidence (top-option probability), so that "the model is unsure about this frame" is visible without re-running anything.
24. As a photographer, I want to pass my own `--shoot-context` free text into the request state, so that a dim, high-ISO street shoot is judged as that and not as a failed studio shoot.
25. As a photographer with a bigger machine, I want `--model` to select a different Clef alias (the 27B build when I have the VRAM), so that quality can go up without a rebuild.
26. As a photographer, I want the rubric — all six questions' instructions and criteria — editable in a TOML file without rebuilding, so that I can tune wording against real photos as fast as I can edit text.
27. As a photographer, I want every row to record the effective rubric hash and model alias, so that I can tell later whether two CSVs are comparable, and reproduce one deliberately.
28. As a photographer, I want a hard, explicit error when the ledger was built with a different rubric or model, so that scores from two rubrics never end up silently in one CSV.
29. As a photographer, I want the `stars`/`overall` disagreement rate reported rather than reconciled, so that a bad rubric shows up as a number instead of being fudged away.

### Surviving a long run

30. As a photographer, I want Ctrl-C to leave a usable CSV and an intact ledger, so that an interrupted run isn't wasted.
31. As a photographer, I want `--resume` to pick up where it stopped with no duplicate rows, so that a 400-file run can be spread across a lunch break.
32. As a photographer, I want a file whose size or mtime changed since it was scored to be re-scored on resume, so that a re-export or edit isn't silently recycled.
33. As a photographer, I want `--fresh` to discard the ledger deliberately, so that starting over is an explicit choice.
34. As a photographer shooting bursts, I want identical previews to cost one request while every NEF still gets its own row with `duplicate_of` set, so that the CSV stays one-row-per-file and the run still gets faster.
35. As a photographer, I want `--no-dedupe` to score every frame independently, so that I can check whether dedupe is hiding anything.
36. As a photographer watching a long run, I want a live progress line showing index, filename, scores, per-frame latency and ETA, so that I know it is working and when it will be done.
37. As a photographer, I want a final summary with counts by status, mean/median per score, a star histogram, the disagreement rate and total wall time, so that I can judge the run and the rubric at a glance.
38. As a photographer, I want `--quiet` and `--verbose` to control how much of that I see, so that scripted use stays clean and debugging gets detail.
39. As a photographer scripting this, I want distinct exit codes for all-ok, completed-with-failures, fatal, and interrupted, so that a shell can act on the outcome.
40. As a photographer, I want the CSV rewritten periodically during the run as well as at exit, so that even a killed run leaves something readable in a spreadsheet.
41. As a photographer, I want `--absolute` to emit absolute paths, so that a CSV can be used from another directory.
42. As a photographer debugging odd results, I want `--json-scores` to write the full raw model answer per file, so that nothing about a surprising score is lost.

### Trust and safety

43. As a photographer, I want v1 to write nothing into the photo folder except the CSV I asked for, so that trying the tool cannot damage a library.
44. As a photographer, I want my NEFs opened read-only and never modified, so that XMP-in-RAW stays a non-feature.
45. As a photographer, I want re-running the same folder with the same rubric and model to reproduce the same CSV apart from the timings it measured, so that I can trust the output as a cache and as a record.
46. As a photographer, I want `doctor` to tell me when oMLX is missing, too old, unreachable, or is serving a Clef checkpoint as a plain VLM, so that I don't discover it as 400 identical failures.
47. As a photographer, I want `doctor <FILE>` to self-test preview extraction on one of my own files, so that I know the pipeline works on my camera's output before a long run.
48. As a photographer, I want `models` to list available aliases and flag which are decision models, so that I know exactly what to pass to `--model`.
49. As a photographer, I want a 4xx response to abort the run immediately and 5xx/timeouts retried a few times with backoff, so that a wrong rubric doesn't fail identically 400 times and a hiccup doesn't lose a run.
50. As a photographer, I want each failed row to carry a short error message, so that a CSV sitting in front of me explains itself without reading logs.
51. As a photographer, I want a preflight check before the first file, so that a broken server costs seconds rather than an hour.

### Building on it

52. As the developer of the eventual macOS app, I want the scoring pipeline in a library crate that never touches stdout, a TTY, or process exit, so that the GUI is a new front end rather than a port.
53. As the developer of the eventual macOS app, I want progress delivered through a trait I can reimplement, so that a window replaces the terminal without touching scoring logic.
54. As a maintainer, I want the `/v1/systemone` wire contract behind one adapter with payload-driven tests, so that API drift breaks a small, obvious set of tests instead of the pipeline.
55. As the developer of the v2 XMP writer, I want every field a sidecar needs (all six answers, confidences, rubric hash, model, timestamp) recorded now, so that v2 needs no re-scoring.

---

## Implementation Decisions

### Shape of the code

- Rust workspace, Cargo, `Cargo.lock` committed, exact dependency pins. Target: macOS on Apple Silicon; the core crate stays platform-neutral.
- A **core library crate** holds all scoring logic: ingest (scan, filter, sort, ignore rules), preview (extraction + orientation), preprocess (downscale, JPEG encode, data URI), the systemone client and wire types, rubric (schema, TOML load, hashing), scoring (orchestration: one file → one outcome), ledger (append-only JSONL, resume, repair), report (CSV), and an error taxonomy.
- A **thin CLI binary** holds argument parsing, the `Reporter` implementation (progress bar, logs), and the exit-code mapping. Nothing else.
- **Hard rule:** the core crate takes progress through a `Reporter` trait and never writes to stdout, never checks `isatty`, never calls `std::process::exit`. This is what makes the GUI cheap later.
- Cancellation is cooperative: the pipeline checks a cancellation flag between files (not mid-request), so Ctrl-C leaves a consistent ledger.

### The seam

One seam, at the top of the core:

- A scoring entry point in the core takes: a directory path, an effective config (endpoint, model alias, rubric, preprocess parameters, resume/dedupe/timeout settings), a `&dyn Decider`, a `&dyn Reporter`, and a cancellation flag; it returns a run summary (counts, statistics, wall time) or a fatal error.
- **`Decider`** is the image-plus-rubric → raw-answers boundary. Its request carries the encoded preview bytes, the rubric, the state (task, filename, shoot context) and the model alias; its response is the raw answer document plus usage numbers. The oMLX HTTP client is the only production implementation; tests supply fixed and adversarial answers.
- **`Reporter`** receives progress events (started, per-file outcome, finished, warning) as plain data. The CLI renders them with a progress bar and tracing; tests collect them into a vector.
- No other seam is introduced. Preview extraction, orientation, downscaling, dedupe and hashing are all observable **through** `Decider` — a test asserts on the exact bytes, dimensions and questions the pipeline hands it. Ledger, CSV and retry behaviour are observable through the entry point's files and summary.

### Preview and preprocessing

- Embedded preview extraction uses the pure-Rust `raw-files` crate: open read-only, take `largest_preview()` selected by **pixel area**, read its bytes, read its size and orientation. No `exiftool` shell-out, no libraw linkage in v1 (both are install steps and a signing/licensing tax on the eventual `.app`).
- Orientation (EXIF 1–8) is applied **before** downscaling, using `image`'s orientation application, so a portrait frame reaches the model upright. This is the highest-risk correctness item in v1 after the wire layer.
- Downscale with `fast_image_resize` (Lanczos3) in a single pass to the longest edge; never upscale.
- Encode JPEG q88 (`ycbcr`, 8-bit) with no EXIF/APP1 in the output, then base64 into a full `data:image/jpeg;base64,…` data URI.
- Defaults: `max_edge` 1568, `jpeg_quality` 88, both overridable.
- `--dry-run` renders the exact bytes that would be sent into a previews directory in the working directory and performs no HTTP.

### Ingest

- Accepted extensions, case-insensitive: `.nef`, `.nrw`, `.dng`. Anything else increments a `skipped_extension` count — never an error.
- `.dng` is accepted but **not decodable in v1**: the pinned `raw-files` has no DNG reader, so a DNG gets an `unreadable` row whose `error` cell names that cause and the way out (export a NEF or TIFF, or score the JPEG). The extension stays accepted so a DNG-only shoot is counted and explained rather than silently skipped; a DNG/TIFF reader is later work and needs a fixture to prove it against.
- Non-recursive by default; `--recursive` walks subdirectories, never descending into hidden directories.
- Ignore `._*`, `.DS_Store`, and anything under `__MACOSX`, and report how many were ignored.
- Sort by natural, NFC-normalised filename so row order is stable.

### Wire contract

- `POST {endpoint}/v1/systemone`, JSON body containing: `model` (alias), `state` (non-null: task framing, filename, optional shoot context), `images` (exactly one data URI), `truncate: true`, and the rubric's `questions`. One image per request even though the API allows several — one frame per decision keeps every result attributable.
- Requests are strictly sequential. Never concurrent: oMLX serialises decision requests, so concurrency only manufactures timeout failures.
- Client: `ureq` (blocking) + `serde_json`; no async runtime in v1. Connect timeout 5 s, read timeout 120 s (`--timeout`), because a 27B model on a long prompt can legitimately take tens of seconds.
- Response contains one answer per question, typed as: `noul` (probability of yes), `choice` (`choice`, `probabilities`, `confidence`), `score` (`score`, `probabilities`, `confidence`, `legend`), plus `usage.input_tokens` and, on the reference server, `usage.latency_ms`.
- Parsing is defensive, because this endpoint is new surface:
  - `choice` arrives as a **string** key from the rubric (stars parse to an integer for output).
  - `score` arrives as a **float** (the expected value over the legend) and is recorded at that precision.
  - `probabilities` may arrive as an ordered array or an object keyed by option: deserialise as an opaque JSON value and treat it as **display-only**. `score`/`choice`/`confidence` are the source of truth; the recorded answer is never derived from `probabilities`.
  - A missing answer key is `bad_response` for that file; the raw answer document is preserved for `--json-scores`.

### The rubric

- Six questions in one forward pass, encoded as TOML so wording can be tuned without a rebuild. An embedded default ships in the binary; `--rubric <FILE>` overrides it. The rubric also names its default model alias.
- Four `score` questions, each with an ordered criteria array `0`–`10`, so `score` is a float in `[0, 10]` and `legend` is the ordered descriptions. The reported answer is never rescaled to 100 and the four are never combined.
- The `overall` question frames a keeper in **photojournalistic** terms: a distinctive moment, gesture, expression or quality of light matters more than clinical perfection. `technical`, `composition` and `impact` each carry instructions that explicitly tell the model to ignore the other two axes, so the four stay independent instead of collapsing into one taste.
- `stars` is a `choice` question with five keys `1`–`5` described in star-rating terms (`reject` … `hero frame`).
- `verdict` is a `choice` question with keys `dislike`, `neutral`, `like`.
- `choice` criteria are an **ordered array of `{key, description}` pairs**, not a table, because option order is part of the model's legend and tables do not guarantee order.
- **Wire rendering:** the rubric is authored in that one ordered form and rendered per question type — the ordered legend of level descriptions for a `score` (the index is the key), an object keyed by option, in declaration order, for a `choice` — because those are the two shapes the Clef heads read. `docs/adr/0001-rubric-renders-two-criteria-shapes.md` records why and what it costs.
- **Rubric hash:** SHA-256 over the canonicalised question schema (ids, types, instructions, criteria, in declaration order) plus the model alias. Recorded in the ledger header, in every CSV row, and (v2) in the XMP sidecar. Calibration and resume both key off it: changed wording invalidates cached scores by design.
- `stars` and `overall` will sometimes disagree. They are both recorded and **never reconciled**; the run summary prints the disagreement rate. A high rate means the rubric wording needs work, not a fudge factor.

### Ledger and resume

- A JSONL ledger lives beside the output CSV. First record is a versioned header (rubric hash, model, endpoint, start time); each subsequent record is a row with file name, size, mtime in nanoseconds, sent-preview SHA-256, status, the full outcome, and a timestamp.
- Every record is appended and flushed immediately. Recovery scans for the last parseable line and truncates anything after it, so a `SIGKILL` mid-write cannot poison the file.
- Resume key is `(file, size, mtime_ns)`: any change means re-score.
- Dedupe key is the SHA-256 of the **sent preview bytes**, plus everything else the model reads — task framing, shoot context, rubric hash, model alias — but **not the filename**: an identical preview judged in the same state reuses the earlier outcome, since the model is deterministic for identical bytes, while two frames of one burst (which always have different names) still dedupe. Disabled with `--no-dedupe`. `docs/adr/0002-dedupe-keys-the-state-but-not-the-filename.md` records why the filename is excluded.
- **Burst policy:** every NEF gets its own CSV row; deduplicated frames carry the first frame's scores plus `duplicate_of`. The ledger saves the requests; the CSV stays one row per file so a filename is never missing.
- `--fresh` discards the ledger. A rubric-hash or model mismatch without `--fresh` is a hard error with an actionable message.
- The CSV is rewritten from the ledger every 25 rows (configurable) and at exit.

### CSV

RFC 4180, UTF-8, header row, `\n` line endings, one row per input file, in ingest order.

| column | content |
|---|---|
| `file` | Path relative to the input directory, or absolute with `--absolute`. |
| `overall`, `technical`, `composition`, `impact` | Model's expected value, full precision, empty when the row failed. |
| `stars` | Integer 1–5 from the `choice` key. |
| `verdict` | `like` / `neutral` / `dislike`. |
| `conf_overall`, `conf_stars`, `conf_verdict` | Top-option probability; the cheapest available uncertainty signal. |
| `latency_ms` | Server-reported when present, else client-measured. |
| `input_tokens` | Prompt size from usage. |
| `status` | Taxonomy below. |
| `error` | Short message when status isn't `ok`. |
| `duplicate_of` | Filename of the frame whose identical preview was reused. |
| `rubric_hash` | First 8 hex characters. |
| `model` | The alias actually used. |

`--json-scores <FILE>` additionally writes the full raw answer document per file.

### Failure taxonomy

| status | cause | action |
|---|---|---|
| `ok` | — | — |
| `unreadable` | file open/parse error | record, continue |
| `no_preview` | no embedded JPEG | record, continue |
| `preview_broken` | JPEG decode fails | record, continue |
| `http_400` | invalid criteria / bad question schema | **abort** (rubric is wrong) |
| `http_413` | context too long | record, continue, suggest a smaller `--max-edge` |
| `http_422` | malformed body | abort (client bug) |
| `http_5xx` | the server failed on its own side | retry, then record, continue |
| `server_unreachable` | refused / DNS | abort after retries |
| `timeout` | exceeded read timeout | retry, then record |
| `bad_response` | answer missing or wrong type | record raw document, continue |

Retries: three retries (four attempts in all) with 0.5/1/2 s backoff, **only** for `server_unreachable`, `timeout`, and 5xx. Never retry 4xx.

`http_5xx` exists because the retry rule above already names 5xx as a transient
class, so the taxonomy has to be able to name one: a run retries it and, if it
never clears, records the row and carries on, because one server hiccup must not
cost the photographer the shoot. It is the tenth `status` string, so every
consumer of `status` (the XMP sidecars in v2, the GUI) accepts it.

Preflight before the first file: `GET /v1/models`, confirm the alias exists, and if the server exposes per-model status, confirm the model is a decision model. Fail fast with an actionable message rather than 400 files of the same 400.

Exit codes: `0` all ok · `1` completed with failed rows · `2` fatal (bad arguments, missing directory, unreachable server, rejected rubric) · `3` interrupted (SIGINT, ledger intact).

### CLI surface

- `score <DIR>` with: `--out` (default `./scores.csv`), `--endpoint` (default loopback oMLX), `--model`, `--rubric`, `--shoot-context`, `--max-edge`, `--jpeg-quality`, `--recursive`, `--resume` / `--fresh`, `--json-scores`, `--no-dedupe`, `--limit`, `--dry-run`, `--absolute`, `--timeout`, `--quiet` / `--verbose`.
- `models` — lists aliases and flags decision models.
- `doctor [FILE]` — server reachability and version, decision-model detection, and, given a file, a preview-extraction self-test.
- `--version` prints the version and the embedded rubric hash.
- Progress (TTY only): index/total, filename, scores and stars, per-frame latency, ETA. Final summary: counts by status, mean/median per score, star histogram, disagreement rate, total wall time.
- **No `calibrate` subcommand in v1** (decided in this session: there are no star-rated shoots available as ground truth).

### Determinism

A decision model has no sampling temperature: the answer is a deterministic function of weights, image bytes and question schema. Re-running the same folder with the same rubric and model **must** produce the same CSV in every cell that reports a decision — the six answers, their confidences, `input_tokens`, `status`, `error`, `duplicate_of`, `rubric_hash` and `model`. This is an acceptance criterion, not an aspiration, and the reuse key — sent bytes, request state, rubric hash and model (ADR 0002) — encodes it.

The one exception is `latency_ms`, and it is not a weakening of the criterion but a statement of what the column is: it measures the run (`usage.latency_ms` — the server's own number when it reports one, otherwise this side's wall clock), not an answer from the model. No two runs can produce the same measurement. So the acceptance test compares two full runs cell by cell after blanking that one column, and asserts that every remaining cell — including the ones a future change is most likely to disturb, like `input_tokens` and `duplicate_of` — is byte-identical.

### Defaults locked in this session

- Default model alias: the **Flash 9B** checkpoint (fits ~16 GB, roughly 0.5–1.5 s/frame); the 27B build is opt-in via `--model`, which is also how the alias reaches the rubric hash.
- Aesthetic target: **photojournalistic keepers** — `overall` is instructed around distinctive moments, gestures and light rather than clinical perfection.
- Bursts: **one row per NEF**, `duplicate_of` populated, dedupe on by default.
- Calibration against existing star ratings: **out of scope for v1**.
- Test seams: **core seam only** — no CLI end-to-end suite in v1; CLI argument parsing is tested in isolation.

---

## Testing Decisions

**What makes a good test here:** a test drives either the core scoring entry point with a fake `Decider` and a real directory on disk, or the wire adapter against canned HTTP responses — then asserts only on externally visible results: the bytes/files produced (CSV, ledger, previews), the run summary, the exit classification, and the requests the pipeline made. No test reaches into a private helper. If a behaviour can't be observed through the seam, either it isn't worth testing or the seam is missing it.

**The seam and what it buys:**

- `Decider` is the assertion point for everything the model would see: exact image bytes, dimensions, data URI framing, the six questions with their order and criteria, `state` contents (filename, shoot context, task framing) and the model alias. Orientation, downscale, quality, EXIF stripping, dedupe and request shape are all verified here — no separate test hooks.
- `Reporter` collects progress events into a vector, so progress, warnings and the summary are assertable without a terminal.
- The run summary plus on-disk files cover resume, dedupe records, CSV rewriting, statistics and exit classification.

**Modules (and what is tested where):**

- Ingest: extension filtering, ignore rules, hidden directories, recursion, order stability, `skipped_extension` count — all via the entry point over a temp directory.
- Preview + preprocess: orientation for all eight EXIF variants (including the portrait regression case), largest-by-area selection, never-upscale, cap respected — all via the bytes handed to `Decider`, and via `--dry-run` output.
- Rubric: canonicalisation and hash stability (same schema in different TOML whitespace/ordering ⇒ same hash; any instruction or criteria change ⇒ different hash), and hash mismatch against a ledger ⇒ hard error.
- Scoring orchestration: dedupe reuse with `duplicate_of`, `--no-dedupe`, per-file status mapping, cancellation between files, disagreement statistics.
- Ledger: append/flush, corrupt-tail repair, resume key semantics (size/mtime change re-scores), `--fresh`.
- Report: RFC 4180 escaping for filenames containing commas, quotes and non-ASCII, empty-value handling on failure rows, stable column order.
- Wire adapter: canned and adversarial responses through a localhost mock HTTP server — `probabilities` as array *and* as object, float `score`, string `choice`, missing answer key, 400/413/422/5xx, timeout — asserting the resulting outcome or error classification and the retry behaviour. No real network, no real model, in CI.
- CLI: argument parsing in isolation (defaults, conflicts such as `--resume` with `--fresh`, invalid enumeration values). Exit-code mapping tested as a pure function of the run summary.

**Fixtures:**

- Committable tiny fixtures (a few bytes each) cover the no-RAW-needed paths: a garbage file with an accepted extension for `unreadable`, a filename containing a comma and a quote for CSV escaping, an empty directory for the header-only run.
- Real-capture fixtures are **not** committed (NEFs are large). A documented fetch script describes what to place in the fixtures directory: landscape (orientation 1), portrait (orientation 6/8), high-ISO, blurred, a burst pair with identical preview bytes, and a truncated file.
- Tests that require real fixtures **skip loudly** — a single explicit notice naming the fixture and the fetch script — never silently pass.

**Golden and acceptance:**

- CSV snapshots for the fixture set against a scripted `Decider`, so column order, formatting and full-precision numbers are pinned.
- Acceptance 1: a long run (fixtures repeated into a synthetic 400-file directory) completes with a status on every row.
- Acceptance 2: cancelling mid-run then resuming produces the same CSV as one uninterrupted run, with no duplicate rows.
- Acceptance 3: two full runs agree byte for byte apart from `latency_ms`, which measures the run rather than the model.
- Acceptance 4: `--dry-run` previews are upright for portrait fixtures.
- Acceptance 5: with no XMP requested, an mtime scan of the input directory shows zero writes.

**Prior art:** none — this is the first code in the repo. This spec establishes the pattern: every test enters through the scoring entry point (fake `Decider`, temp directory) or through the wire adapter (mock server), and asserts on artifacts rather than internals.

---

## Out of Scope

- **XMP sidecar writing (v2)** — `xmp:Rating`, `xmpDM:pick`, namespace, merge and safety rules all live in `SPEC.md` §12 and get their own spec. v1 only records the fields they will need.
- **Tauri GUI (v3)** — `SPEC.md` §13.
- **`calibrate` against existing star ratings** — decided out of scope in this session; there is no rated ground truth to calibrate against.
- **CLI end-to-end test suite** — decided in this session: the core seam is the only seam, argument parsing is unit-tested.
- RAW demosaic, editing, colour pipeline, output rendering.
- Catalog database, library management, import into Lightroom.
- Video (the endpoint doesn't accept it).
- Cloud calls, telemetry, model download, starting/stopping oMLX, server management.
- Batching or parallel requests; any attempt to "speed up" by concurrency.
- Formats other than `.nef`, `.nrw`, `.dng`.
- XMP embedded in RAW.
- Windows/Linux support; the target is macOS on Apple Silicon (though the core crate stays portable).

---

## Further Notes

**Milestones (build order):**

| M | scope | done when |
|---|---|---|
| M0 | workspace scaffold, `--version`, `doctor`, error taxonomy, pins + lockfile | `doctor` explains a missing/old oMLX clearly |
| M1 | ingest + preview extraction + orientation + downscale + `--dry-run` | a portrait fixture exports upright at the cap |
| M2 | systemone client + rubric + single-file score | one NEF yields six answers; adapter tests green |
| M3 | ledger, resume, CSV, progress, summary | acceptance 1–5 pass |
| M4 | real-shoot validation and rubric tuning | the photographer agrees with the scores on a real shoot; rubric frozen |

M4 is the long pole and it is not engineering — it is wording iteration against real photos. Getting to M3 quickly is the whole point.

**Performance budget:** Flash 9B ≈ 0.5–1.5 s/frame, 27B ≈ 2–4 s/frame, plus ~40 ms of local work per frame. A 400-file shoot is roughly 5–10 minutes on Flash, 15–25 on 27B. The run is serialised by the server, so the only real levers are burst dedupe and rubric length — both already handled.

**Dependency choices (roles, not versions; pin exact versions at M0):** argument parsing, serde/JSON, TOML, `raw-files` for NEF previews, `image` for JPEG decode/encode and orientation, `fast_image_resize` for Lanczos3, base64, `ureq` for blocking HTTP, a CSV writer, sha2, a progress bar, `tracing`, `thiserror`. Dev: a mock HTTP server, snapshot assertions, temp directories. `raw-files` and `image`'s orientation API are the two upstreams whose changes land directly in the critical path.

**Risks carried into the build:**

- Which Clef checkpoint scores better is untested; the rubric hash includes the model so a shoot can never mix 27B and Flash rows silently.
- These four questions are a new use of a model trained on typed decisions. No tuning before evidence.
- Scores may saturate (everything 6–8). Watch the histogram; if `composition` variance is low, the criteria descriptions need work, not the thresholds.
- `/v1/systemone` is new surface and may drift: defensive parsing, preserved raw answers, adapter tests, `doctor` version check.
- The embedded preview is the camera's rendering, not the photographer's edit. Deliberate: a cull judges the frame, not the grade.
- Long runs on battery, or sleep, can kill the server: ledger plus resume, plus `doctor` preflight.
- Rubric wording churn invalidates caches by design — expect several re-runs during M4 and budget for them.

**Not yet decided (deliberately):** the exact 27B alias name (discovered via `models`), and whether M4 ends with a rubric frozen as the embedded default or a per-genre family of rubrics.
