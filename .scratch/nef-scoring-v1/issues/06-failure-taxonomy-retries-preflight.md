# 06: Failure taxonomy, retries, preflight and exit codes

**What to build:** A run that fails in the right way at the right scale. Every failure lands in the documented taxonomy and drives a decision the photographer would make: a corrupt or preview-less file is recorded and the run continues; a rejected rubric or a malformed request body aborts immediately, because a wrong rubric failing identically 400 times is worse than stopping; a too-long context is recorded with a hint to lower the preview size; a hiccup is retried three times with a short backoff, but only for the transient cases, never for a 4xx. Before the first file, a preflight confirms the server is answering and the alias exists, so a broken server costs seconds rather than an hour. The run classifies its own outcome through distinct exit codes — all ok, completed with failed rows, fatal, interrupted — and cooperative cancellation between files leaves the ledger consistent.

**Blocked by:** 04

**Status:** ready-for-agent

- [x] Each status in the taxonomy is produced by its documented cause and follows its documented record-or-abort action
- [x] Connection failures, timeouts and 5xx are retried three times with 0.5/1/2 s backoff; 4xx is never retried
- [x] A rejected rubric and a malformed request body abort the run instead of grinding through the remaining files
- [x] Preflight runs before the first file and fails fast with an actionable message when the server is unreachable or the alias is missing
- [x] Exit codes distinguish all-ok, completed-with-failed-rows, fatal, and interrupted
- [x] Cancellation is checked between files, not mid-request, and leaves an intact ledger
- [x] Every failed row carries a short error message that explains itself without reading logs
- [x] Acceptance: a synthetic 400-file run over fixtures completes with a status on every row
- [x] Acceptance: with no XMP requested, an mtime scan of the shoot folder shows zero writes

## Comments

Implemented, tests-first. A run now fails the way the taxonomy says it should,
at the scale it should: one bad file is a row, a wrong rubric stops the run, a
hiccup is retried, a broken server costs seconds, and Ctrl-C keeps the rows it
earned.

- **The taxonomy is complete, including 5xx.** `ErrorKind::Http5xx`
  (`status` `http_5xx`) is new and is a deliberate **contract amendment**, not a
  silent extension: the spec's retry paragraph already names 5xx as a transient
  class, so the taxonomy has to be able to name one, and `bad_response` (what the
  adapter used to file a 500 as) is a different diagnosis and not retryable.
  `.scratch/nef-scoring-v1/spec.md` §"Failure taxonomy" gains the table row and a
  sentence saying why. The `status` vocabulary is now **ten strings**; every
  consumer of `status` (the v2 XMP sidecars, the GUI, anything parsing a CSV
  written by an older build) must accept `http_5xx`. `from_status_code` maps the
  whole `500..=599` range to it, and `recovery()` is `RetryThenRecord`.
- **Record-or-abort follows the table.** `Recovery` already encoded the action
  column; the run loop now honours it. `http_400`, `http_422` (a retry cannot fix
  a rejected rubric or a malformed body) and `server_unreachable` **after its
  retries** end the run; everything else records a row and continues, including
  `http_5xx`, `timeout`, `http_413`, `bad_response` and the three file kinds.
- **Retries live in the run loop, around `decider.decide()`.** `RetryPolicy`
  (on `Config`) is the waits between attempts: 0.5 s, 1 s, 2 s — three retries
  after the first attempt, so four attempts at most, which is the only reading in
  which all three documented delays are used. A fully burnt row says so
  (`… HTTP 503: models reloading — 4 attempts`); a 4xx is never retried, so its
  row says nothing about attempts. `RetryPolicy::without_waiting()` is what the
  tests use, because an attempt count should cost no wall time.
- **Preflight runs before the first frame.** `Decider::preflight()` — the same
  seam, no second trait — defaults to `Ok(())` so every fake is ready, and
  `systemone::Adapter` implements it as `doctor::diagnose(client, model)`
  (`GET /health`, `GET /v1/models`, `/v1/models/status` when the server has it).
  So preflight is exactly as strict as `imagecull doctor`, and its failure is one
  fatal error carrying doctor's actionable sentence. `scoring::run` calls it
  after the scan (a missing folder is not the server's fault) and before any
  frame or any report: a broken server writes no CSV at all.
- **Cancellation is cooperative.** `scoring::Cancel` is an `AtomicBool` with
  `cancel()` / `is_cancelled()`. The run checks it at the top of each iteration —
  between files, never mid-request — and on a cancel it stops, writes the CSV of
  the rows it has, and classifies the run `Interrupted`. The CLI installs the
  SIGINT handler (`ctrlc = "=3.5.2"`: one short `sigaction` wrapper the front
  end owns, because the core is forbidden to touch signals or `process::exit`;
  `signal-hook` was the heavier alternative for the same two calls) and pulls the
  same flag. If the handler cannot be installed the CLI says so and carries on.
- **Exit codes are a pure function of the run.** `Run::ending()` is
  `Completed` / `Aborted { reason }` / `Interrupted`, and `Run::exit_code()` maps
  it to `0` / `1` / `2` / `3` — an interruption outranks a failed row, and a run
  that stopped at a frame is fatal however many rows it had written. The CLI's
  `exit_code_for_run` is now that function, and nothing else decides the code.
- **Failed rows explain themselves.** The `error` cell is the taxonomy's message
  verbatim (one line, under 200 chars, naming the file, the HTTP status and the
  server's own `detail`), and a 413 row adds the remedy: *the preview is too big
  for the model's context; lower `--max-edge`*.
- **`Run` grew `total`** (files the scan accepted) next to `frames` (files the
  run got to), so an interrupted or stopped run's summary reads "1 of 400 scored
  · interrupted" instead of pretending the shoot was 1 file long.

Decision recorded rather than guarded: **`--out` inside `<DIR>` needs no
guard.** Spec §43 lets a run write the CSV the photographer asked for inside the
shoot folder, so the run writes exactly where `--out` says. (`--dry-run` keeps
its `guard_outside_shoot`, because a previews directory is not something anyone
asks for inside their negatives.)

Seams the tests assert at, and what they pin:

- `tests/failure_taxonomy.rs` (new, 16 tests) — the scoring entry point with a
  scripted `Decider`, then the CSV the run wrote (parsed back row by row), the
  `Run`'s ending and exit code, and the requests each frame cost, which is the
  attempt count observed at the seam rather than a counter inside the loop:
  a timeout records after 4 attempts and the run continues; a 5xx that never
  clears is recorded and the next frame is scored; a 413 is one attempt; a 400
  and a 422 stop the run at the first frame with the CSV still written; an
  unreachable server stops it after 4 attempts with two frames never prepared;
  the exit code is a function of the run; every answer-borne status and every
  file-borne status (`unreadable`, `no_preview`, `preview_broken`, `ok`) reaches
  the CSV from its documented cause; the retry schedule is the spec's three
  waits; a refused preflight judges nothing and writes nothing; cancelling after
  two requests keeps exactly two rows; and the two acceptances — a synthetic
  **400-file** run with a status on every row, and an **mtime-and-size scan** of
  the shoot folder unchanged after a run.
- `tests/systemone_http.rs` (real adapter, loopback stub) — a 500 is `http_5xx`
  with its status kept and `RetryThenRecord`; a 413 names `--max-edge`; a server
  that does not serve the alias fails preflight and receives **no** POST. The stub
  gained `Stub::serving(script, aliases)`, which answers the three preflight GETs
  the way a ready oMLX does, so the existing end-to-end runs go through preflight
  instead of around it.
- `error.rs` unit tests — the taxonomy's strings and order, the action column,
  the transient set being exactly `server_unreachable`/`timeout`/`http_5xx`, and
  the attempt suffix appearing only when something was retried.
- `cli::score` unit tests — `Interrupted` renders and exits 3, `Aborted` prints
  the reason and exits 2.

Gates: `cargo test --locked` 143 green, `cargo fmt --all -- --check` clean,
`cargo clippy --all-targets --locked -- -D warnings` clean. `Cargo.lock`
updated for the pinned `ctrlc`.

What tickets 05 and 07 must know:

- **05 (ledger/resume/dedupe).** `scoring::run` now returns `Ok(Run)` for a run
  that stopped or was interrupted — it is not an error. A ledger written from
  this loop therefore ends mid-shoot on a cancel, which is what `--resume` and
  the corrupt-tail repair are for. The retry annotation lives inside the error
  *message*, not in a new column: a ledger record that replays an outcome gets it
  for free. `Run.ending` is the thing to branch on — do not infer "completed"
  from `frames.len()`. The every-25-rows CSV rewrite is still unimplemented;
  `report::write(&rows, &outputs)` is called once at the end of the loop and is
  safe to call mid-run.
- **07 (Reporter/progress/summary).** `run`'s signature is
  `(shoot, config, decider, cancel)`; the `Reporter` goes in as the next
  parameter, and `Cancel` stays last because it is the flag the GUI will hold.
  The summary this ticket prints (`cli::score::render_run` / `summary`) is
  stop-gap rendering and belongs to 07: the counts-by-status, the wall time and
  the `interrupted` / `stopped: …` line are all on `Run` (`ending`, `total`,
  `frames`) so 07 can render them without touching the loop. Preflight is a
  `Reporter`-worthy event (it is the first thing a run does) and currently says
  nothing until it fails.

Delivery note: writing to the repository's history is refused in this sandbox -
the git safeguard blocks autonomous write operations rather than prompting - so
this ticket's work is one uncommitted change set on `ticket/06-taxonomy`, which
is otherwise still at `feat/nef-scoring-v1`'s tip (`be3ffe0`); the merge back is
a genuine no-op today. Nothing is staged.
