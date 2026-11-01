# 07: A long run you can watch

**What to build:** The legibility layer for a five-to-twenty-five minute run. Progress reaches the CLI as plain events through the scoring core's reporter seam, so the core still never touches stdout and a window can replace the terminal later without touching scoring logic. While running, a live line shows index over total, filename, the scores and star rating, per-frame latency and an estimate of time remaining. At the end, a summary tells the photographer whether to trust the run: counts by status, mean and median for each of the four scores, the star histogram, the rate at which `stars` and `overall` disagree — reported, never reconciled — and total wall time. `--quiet` and `--verbose` control how much of this appears, so scripted use stays clean and debugging gets detail.

**Blocked by:** 05, 06

**Status:** resolved

- [x] Progress, warnings and the summary reach the CLI as data through the reporter seam; the core still writes nothing to stdout, checks no TTY and calls no process exit
- [x] A test can collect progress events into a collection and assert on them without a terminal
- [x] The live progress line shows index/total, filename, scores and stars, per-frame latency and ETA
- [x] The final summary reports counts by status, mean and median per score, the star histogram, the `stars`/`overall` disagreement rate, and total wall time
- [x] The disagreement rate is reported as a number and nothing is reconciled or fudged
- [x] `--quiet` suppresses per-frame output but keeps the outcome usable by a script; `--verbose` adds diagnostic detail
- [x] Progress output appears only when the output is a terminal, so redirected output stays clean

## Comments

Implemented, tests-first. A run now narrates itself to a `Reporter` as plain
data — started, one event per frame, warnings, finished with the summary — and
the CLI draws that: a live line while it runs, a summary block when it ends,
`--quiet` for scripts, `--verbose` for debugging. The core still writes nothing,
checks no terminal and exits nothing; the crate's lints keep it that way.

- **The seam** — `core::progress`. `Reporter` has one method,
  `event(&self, Event)`; `Event` is `Started(Started)` / `Frame(FrameEvent)` /
  `Warning(Warning)` / `Finished(Finished)`, all owned data so a front end can
  keep what it draws and a test can collect it into a `Vec`. `Silent` is the
  reporter for callers that do not care (every existing test that drives a run now
  passes it).
  `scoring::run(shoot, config, decider, reporter, cancel)` — the reporter sits
  before the cancel flag, and a `Started` event goes out after the scan and
  before the preflight, so a front end knows the work while the first request is
  in flight.
- **One event per frame, whatever the frame did.** A resumed frame from the
  ledger is a frame of this run, a deduplicated frame is a frame of this run, and
  a frame that failed is a row: all of them carry an index (1-based, in ingest
  order), the name the CSV will use, `Judgement::Answered(Answers)` or
  `Judgement::Failed(report::Failure)` (the same `Failure` the row's `status` and
  `error` columns are written from, so a progress line and a row cannot
  disagree), the latency the request recorded (`None` for a frame that never
  reached the model), the shape of the bytes that were sent (`PreviewShape`:
  width, height, JPEG bytes — `None` for a frame resumed from the ledger, whose
  bytes nothing re-encoded), `duplicate_of` when it reused one, and the running
  ETA.
- **ETA** — `estimate_remaining`: elapsed × remaining / done, and `None` on the
  last frame or when no time has elapsed. The wall time behind it starts at the
  scan, not at the first frame, so the same clock answers both the ETA and the
  summary's total.
- **The summary** — `Summary` (`Summary::of(&Run)`, also `Run::summary()`):
  counts by status (`ok` first, then the taxonomy's own order, zero kinds
  omitted — they sum to the frames the run got to), mean and median of each of
  the four scores over the frames that have answers (median = the middle value,
  or the midpoint of the two middle values on an even count), the star histogram
  (index 0 = one star, only scored frames), `disagreements` +
  `disagreement_rate`, total wall time, `resumed`/`duplicates`,
  `left_out` (what the scan skipped), and the `ending`. A run with nothing scored
  has `scores: None` and a `0.0` rate rather than a NaN. `Run` grew `elapsed`,
  which is the only new input the summary needed.
- **The disagreement rule, defined here because the spec names the metric and not
  its threshold (§29).** `progress::agrees(stars, overall)`: map `overall` (0–10)
  to the star it implies — `round(overall / 2)`, clamped to 1–5
  (`implied_stars`) — and count a frame as disagreeing when `stars` is **two or
  more** away from that band. One star of difference is rounding noise (a 7.0 is
  a 3.5-star frame; four stars is not a finding); two is a judgement worth
  looking at. Nothing is reconciled, weighted, corrected or dropped: the count
  and the rate are reported, and the denominator is the scored frames. The rule
  is a `&'static str` (`DISAGREEMENT_RULE`) and the summary line prints it beside
  the number, so the figure reads without the source:
  `disagreements: 6 of 397 (1.5%) · a frame disagrees when its stars are two or
  more away from round(overall / 2), clamped to 1-5 — reported, never
  reconciled`.
- **Rendering is a pure function** — `cli::progress::render(&Event, Verbosity,
  live) -> Rendering { out, err }`. Two strings because one event can say two
  things (a quiet run's outcome and the reason it stopped). `Terminal` is the
  reporter that only writes those strings and flushes them; `live` comes from
  `std::io::IsTerminal` on stdout, exactly as the ticket asks.
  - **Normal** (default): a `scoring N frames · rubric H · model M` header, then
    a `\r…\x1b[K` line per frame —
    `12/400  DSC_0012.NEF  6.42/7.81/5.34/4.96  ★4 like  812ms  eta 7m41s`, or
    `13/400  DSC_0013.NEF  timeout  eta 7m39s` for a row with no answers — then
    the summary block. The live line appears **only when stdout is a terminal**,
    so a redirected stdout gets the summary and nothing else.
  - **`--quiet`**: no per-frame output, no header; one stdout line —
    `scored 397 of 400 in 8m12s · ok 397 · no_preview 1 · timeout 2 · 3 resumed
    · 2 deduplicated · left out: … · rubric 3f2a1b0c · model clef-flash-oQ4e` —
    and the exit code. Statistics are deliberately not in it: a script reads the
    counts and the code, a photographer reads the CSV.
  - **`--verbose`**: a line per frame with the preview's dimensions and bytes,
    the answers at the model's own precision and the status of every row
    (`DSC_0013.NEF  timeout  POST /v1/systemone timed out after 120s — 3
    attempts`; the attempt count is the one the error already carries), then the
    same summary block. These print whether or not stdout is a terminal: the
    ticket's tty gate is about the default running commentary, and a
    `--verbose` run is an explicit request for the log.
  - **Warnings** go to stderr in every mode, `--quiet` included. The real path
    is a ledger cut short by a killed run: `Ledger::truncated_bytes()` reports
    what opening the file discarded, and the run says so before scoring.
  - **Abort and interrupt** are in the summary (`Ending`), printed as
    `stopped: …` / `interrupted: …` in the block, and on **stderr in `--quiet`**,
    so a silent run still explains itself; a fatal error (bad arguments, missing
    folder, refused preflight, ledger mismatch) keeps printing `error: …` to
    stderr and exits 2 in every mode.
- **Retired**: `cli::score::render_run`, its one-line `summary`, and
  `exit_code_for_run` — the ad-hoc per-frame print ticket 06 left behind. The
  exit code is now `run.exit_code()`, straight off the run. `--dry-run` keeps
  its own report unchanged (it renders a previews folder, not a run); the scan
  counts / `plural` / `kilobytes` helpers it shares now live in `cli::progress`.
- **Flags** — `--quiet` and `--verbose` on `score`, conflicting with each other;
  they govern the scoring run only.
- **Nothing else moved.** The CSV and the ledger are byte-identical to ticket
  05's: `report.rs` is untouched, `Record`/`Header` gained no fields (`Ledger` gained a private `truncated` counter, never serialised), and
  the statistics are computed from `Run`, never written into an artifact. No new
  dependency: the progress line is a carriage return, so `Cargo.lock` is
  unchanged.

Seams the tests assert at:

- `tests/progress.rs` (new, 12 tests) — the `Reporter` seam through the scoring
  entry point with a scripted `Decider`: the run announces its total, rubric
  hash and model before the first frame; one frame event per frame, in ingest
  order with a 1-based index and the right total, carrying the model's own
  precision (6.4237), its stars, the recorded latency and the preview's
  dimensions/bytes; an unreadable file is a `Failed(Unreadable)` with no
  latency; a document that cannot be read is `Failed(BadResponse)` with the
  latency its request cost; the reported failure equals the one the row carries;
  a fatal error returns before any frame is reported; frame events outlive the
  run (owned data); ETA is `Some` while work remains and `None` on the last
  frame; **a resumed frame is reported too** (latency from the ledger, no
  preview); **a deduplicated frame names the frame it reused** while costing no
  request; **a truncated ledger tail is reported as a warning**; the run ends
  with `Finished` carrying a summary whose counts and wall time match the run.
- `tests/summary.rs` (new, 8 tests) — `Summary` from a hand-built `Run`, with
  expected values worked by hand: status counts summing to the frames the run got
  to; mean and median per score including even counts (4/6/8/10 → 7.0 and 7.0,
  1/2/3/4 → 2.5 and 2.5) with failed rows excluded rather than zero-filled; the
  histogram summing to the scored count; the disagreement rate over a hand-built
  set with a known answer (3 of 7, from seven `(overall, stars)` cases worked by
  hand); the rule's own mapping (clamps at 1 and 5, one star of difference
  agrees); a run with no answers (no statistics, 0.0 rate, counts still); the
  time and the reused counters; the ending.
- `cli::progress` unit tests (11) — rendering as a pure function of collected
  events: the live progress line's exact string (index/total, name, four scores,
  stars, latency, ETA); a failed frame keeping its place with its status and no
  latency; the header and live line on a terminal and nothing when redirected;
  the summary block's exact text (counts, mean/median, histogram, the rule
  beside the rate, resumed/deduplicated, left out, identity); the block starting
  on a fresh line after a carriage-return line; `--quiet`'s exact one line with
  no per-frame output; `--quiet` explaining an abort/interrupt on stderr and
  saying nothing for a clean run; `--verbose` printing the bytes, the full
  precision and a retried failure's attempts; a duplicate naming its original;
  warnings on stderr in all three modes; duration wording (`0s`, `42s`, `1m05s`,
  `8m12s`, `1h02m`).
- `cli::cli` unit tests — `--quiet`/`--verbose` parse, and both together are
  refused; the dry-run report tests still pass unchanged.

Gates: `cargo test --locked` 198 green, `cargo fmt --all -- --check` clean,
`cargo clippy --all-targets --locked -- -D warnings` clean. `Cargo.lock`
unchanged.

What ticket 08 must know:

- The summary is the honest read on the rubric **before** any wording change: run
  it once on the real shoot and look at the disagreement rate and the star
  histogram together. A high disagreement rate with a flat histogram
  (`composition` saturating 6–8) is the rubric's problem, and the number is now
  printed rather than smoothed.
- The rule is two stars of separation, so the number is comparable only across
  runs that used it; if 08 decides a stricter reading is wanted (one star), that
  is one function (`progress::agrees`) and one constant, and the historical
  numbers would stop being comparable — so decide before the shoot, not after.
- `--quiet` prints no statistics by design; a validation run that wants the
  numbers on a redirected stdout wants the default mode (stdout is then just the
  summary block).

### Resolution

Merged to `feat/nef-scoring-v1`. The branch is green: `cargo test --locked` (204 tests), `cargo fmt --check` and `cargo clippy --all-targets --locked -- -D warnings`.
