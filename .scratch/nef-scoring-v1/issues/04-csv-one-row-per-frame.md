# 04: The CSV — one row per frame, self-explaining

**What to build:** The v1 deliverable: a `scores.csv` the photographer can open in a spreadsheet and act on. One row per input file in ingest order, RFC 4180 and UTF-8, carrying the four raw scores at full precision, the star rating, the verdict, the three confidences, latency and input tokens, the status, a short error message on failed rows, the empty `duplicate_of` column that dedupe will fill later, the rubric hash and the model alias. Paths are relative to the input folder unless `--absolute`. `--json-scores <FILE>` writes the full raw model answer per file alongside, so a surprising score loses nothing. Golden snapshots pin column order and number formatting, and filenames containing commas, quotes and non-ASCII characters are escaped correctly.

**Blocked by:** 03

**Status:** resolved

- [x] `imagecull score <DIR> --out <FILE>` writes a CSV covering every accepted input file, one row each, in ingest order
- [x] Column set is exactly the spec's: file, the four scores, stars, verdict, the three confidences, latency, input tokens, status, error, duplicate_of, rubric hash, model
- [x] Scores are the model's full-precision expected values; failed rows leave score cells empty rather than zero or a placeholder
- [x] RFC 4180 escaping verified for filenames containing commas, double quotes and non-ASCII characters
- [x] `--absolute` emits absolute paths; default is relative to the input folder
- [x] `--json-scores <FILE>` writes the raw answer document per file
- [x] `--limit <N>` scores only the first N files, so a run can be smoke-tested first
- [x] Golden snapshot pins header, column order and formatting for the fixture set against a scripted decider

## Comments

Implemented. The v1 deliverable exists: `imagecull score <DIR>` writes the CSV,
one row per accepted file in ingest order, and `--json-scores` keeps every raw
answer beside it.

- **The writer** — `core::report`. `Row` is one line of the deliverable as data:
  the source file, the six answers at the precision the model gave them, the
  usage numbers, the raw document, and a `Failure` (`ErrorKind` plus message)
  when there are no answers. `Outputs` says where the two files go and how paths
  are spelled. `report::write` renders RFC 4180 — UTF-8, `\n` endings, a cell
  quoted and its quotes doubled only when it holds `,`, `"`, CR or LF — then
  writes the CSV and, when asked, the JSON.
- **A hand-rolled writer, not a crate.** Seventeen fixed columns, quoting needed
  for four characters, `\n` endings rather than RFC 4180's `\r\n`: twenty lines
  in `report::push_field` versus another exactly-pinned dependency on the
  critical path. If the column set ever becomes dynamic, revisit.
- **Accumulation, not reconstruction.** `scoring::run` pushes a `report::Row` as
  each frame is produced and writes at the end; nothing is rebuilt from
  `Run.frames` afterwards. `Config` grew `outputs: report::Outputs` (`--out`,
  `--json-scores`, `--absolute`), so the run owns where its report lands.
- **Full precision, empty not zero.** A score cell is `f64::to_string()`, the
  shortest text that round-trips: `6.4237` stays `6.4237`, and `8.0` is written
  `8`. Failed rows leave the four scores, the stars, the verdict and the three
  confidences empty; `latency_ms` and `input_tokens` are empty only when no
  document came back at all, so a `bad_response` row still reports what the
  request cost.
- **`--json-scores`** is one JSON object keyed by the same file spelling the CSV
  uses, each value the raw `document` exactly as the server sent it, `null` for
  a frame the model never answered. A `bad_response` row keeps its document.
- **`--limit`** rides the existing `ScanOptions.limit`, so the CSV covers exactly
  the truncated, naturally ordered set the model was asked about.
- **`--absolute`** resolves the frame's path with `std::path::absolute`, so the
  column is absolute even when `<DIR>` was given relatively.
- **`Failure::of`** classifies an unclassified error as `unreadable`. A frame's
  error is always classified (a run-level one stops the run before it could
  become a row), so the fallback is a total-function guard, not a policy.

Seams the tests assert at: the scoring entry point (`tests/report.rs` — a real
temp folder of synthetic NEFs, a scripted `Decider`, then the CSV bytes and the
JSON file the run wrote) and CLI argument parsing in isolation (`cli.rs`).

Tests: nine in `tests/report.rs` — header-only run over a folder with no RAW
files; one scored row; failed rows' empty cells; escaping of comma, quote, line
break and non-ASCII; relative versus absolute; `--limit`; `--json-scores`
including a `bad_response` document and a `null` for a frame never answered; the
whole-file golden for the fixture set; and "nothing but the CSV is written unless
asked". One CLI test for the new flags. 121 tests in the workspace.

What tickets 05, 06 and 07 must know:

- **`report::Row` is the unit to build from a ledger record** — `duplicate_of` is
  already a field, empty until dedupe fills it. `Row::of` reads a `Frame`; a
  ledger record needs its own constructor, and that is the one place to change.
- **The CSV is written once, at the end of `run`.** The every-25-rows rewrite is
  ticket 05's; `report::write(&rows, &outputs)` is already callable mid-run.
- **The output paths are not guarded against the shoot folder.** `--out` or
  `--json-scores` inside `<DIR>` would write there. `--dry-run` refuses a
  previews directory inside the shoot (`dry_run::guard_outside_shoot`); whether a
  real run should do the same is ticket 06's call and was deliberately not
  decided here.
- **The `error` cell is the taxonomy's own message**, which names the file again
  (`DSC_0003.NEF embeds no JPEG preview`). Short and self-explaining, so it is
  used verbatim; wording is ticket 06's.
- **Nothing is printed about the CSV.** The per-frame lines and summary still
  come from `cli::score::render_run`, which ticket 07 replaces with the
  `Reporter` seam; `report` never touches stdout, and the core still passes the
  crate lints against it.
- **The tree was reformatted with the toolchain's rustfmt** (1.9.0, style edition
  2024) because `cargo fmt --check` was already failing on six files from earlier
  tickets under it. That change is mechanical and separable; if the project pins
  an older rustfmt, drop it.

### Resolution

Merged to `feat/nef-scoring-v1`. The branch is green: `cargo test --locked` (204 tests), `cargo fmt --check` and `cargo clippy --all-targets --locked -- -D warnings`.
