# 05: Ledger, resume and burst dedupe

**What to build:** A 400-file run that can be interrupted and picked up again without losing work or producing a lying CSV. Every scored frame is appended to a JSONL ledger beside the output — a versioned header recording rubric hash, model, endpoint and start time, then one record per frame carrying the file's identity, the hash of the bytes that were actually sent, the status and the full outcome. Each record is flushed as it is written and a corrupt tail from a hard kill is repaired on recovery. `--resume` skips what is already recorded, matching on file plus size plus modification time so that a re-exported file is re-scored rather than silently recycled; `--fresh` discards the ledger deliberately; a ledger built under a different rubric or model is a hard, actionable error rather than a silent mix. A frame whose sent preview bytes match an earlier one reuses that outcome — the model is deterministic for identical bytes — while still getting its own CSV row with `duplicate_of` naming the first frame, and `--no-dedupe` turns that off. The CSV is rewritten from the ledger periodically during the run and at exit, so a killed run still leaves something readable.

**Blocked by:** 04

**Status:** ready-for-agent

- [x] The ledger is written beside the output CSV with a versioned header and one record per scored frame, appended and flushed immediately
- [x] A deliberately truncated/garbage final ledger line is repaired on recovery and does not poison the run
- [x] `--resume` completes a cancelled run with no duplicate rows and no re-scored files
- [x] A file whose size or modification time changed since it was scored is re-scored
- [x] `--fresh` discards the ledger; a rubric-hash or model mismatch against an existing ledger without `--fresh` is a fatal, actionable error
- [x] Identical sent preview bytes reuse the earlier outcome and set `duplicate_of` to the first frame's filename; every NEF still gets its own row
- [x] `--no-dedupe` scores every frame independently
- [x] The CSV is rewritten from the ledger every 25 rows and at exit
- [x] Acceptance: cancelling mid-run then resuming produces the same CSV as one uninterrupted run
- [x] Acceptance: two full runs over the same folder with the same rubric and model produce byte-identical CSV

## Comments

Implemented, tests-first. A run now writes a JSONL ledger beside its CSV, renders
the CSV from that ledger, can be stopped and picked up again with no re-scoring
and no duplicate rows, refuses a ledger built under a different rubric or model,
and reuses an identical preview's outcome without losing the frame's own row.

- **The ledger** — `core::ledger`. `scores.csv` → `scores.ledger.jsonl`
  (`ledger::path_for`), so `--out march.csv` gets `march.ledger.jsonl`. Line 1 is
  a `Header` (`version`, full `rubric_hash`, `model`, `endpoint`, `started` in
  RFC 3339 UTC); every line after it is a `Record`: `file`, `size`, `mtime_ns`,
  `preview_sha256`, `dedupe_key`, `status`, `error`, `answers`, `usage`,
  `document`, `duplicate_of`, `recorded`. Each record is `write_all`'d and
  `sync_data`'d as it is produced, so a hard kill loses at most the frame in
  flight. `serde` derive does the schema; no new dependency.
- **Recovery** — loading reads line by line and stops at the first line that is
  not a whole record (`Record::is_whole`: a status the taxonomy names, answers
  when it claims `ok`, an identity either whole or absent), truncating the file
  at the start of that line. A truncated or garbage tail from a `SIGKILL`
  therefore cannot poison the run; the next append lands on clean bytes.
- **Resume key** — `(file, size, mtime_ns)` (`FileIdentity` + `ResumeKey`). A
  frame whose bytes or modification time changed is scored again; a re-scored
  frame is never treated as a duplicate of its own earlier record.
- **Dedupe key** — `scoring::dedupe_key`: SHA-256 over the sent preview's
  SHA-256, the task framing, the shoot context (or a marker for none), the full
  rubric hash and the model alias — **not** the filename. That is a correction to
  ADR 0001's last paragraph, recorded as
  `docs/adr/0002-dedupe-keys-the-state-but-not-the-filename.md` and cross-referenced
  from 0001: the filename is the photographer's label, and a key covering it could
  never fire on a burst. The key is stored per record, so dedupe also applies to
  outcomes a resumed run restores, and `duplicate_of` names the first frame that
  paid for the request. A record whose status is not `ok` is never reused: a
  timeout is not an answer.
- **The CSV is a view of the ledger.** `report::Row` is now built by
  `Record::row` (for fresh and restored frames alike), so the two cannot drift and
  the CSV is genuinely rewritten from the ledger — every
  `Config::csv_rewrite_every` rows (default `DEFAULT_CSV_REWRITE_EVERY` = 25)
  and at exit.
- **Default mode, documented as asked: a plain `score` is a new run.** Neither
  `--resume` nor `--fresh` (`LedgerMode::Restart`) still makes the same
  rubric-hash and model check `--resume` makes — a mismatch is fatal and
  actionable, so two rubrics never share a ledger — and then truncates the ledger
  and starts over rather than appending to it. `--fresh` (`Discard`) skips the
  check and throws the file away; `--resume` (`Resume`) keeps what is there.
  Consequence: `score` twice over one folder is deterministic and never grows an
  ever-longer ledger, and continuing a run is always something the photographer
  said out loud.
- **`Run` grew `resumed` and `duplicates`**, so a front end can say "212 of 400
  scored · 188 resumed · 6 deduplicated" without reaching into the ledger. The
  CLI's interim summary prints them when they are non-zero.
- **CLI** — `--resume` (conflicts with `--fresh`), `--fresh`, `--no-dedupe`;
  `ScoreArgs::ledger_mode()` maps the flags, `Config` carries
  `ledger`/`dedupe`/`csv_rewrite_every`/`endpoint`. `Config.endpoint` is the
  server as the photographer typed it, recorded in the header for provenance; a
  test's fake has none.
- **`ErrorKind::from_status`** reads a stored status back to the taxonomy, so a
  ledger's strings and a live run's cannot drift.

Known limitation, deliberately not widened: a resume reuses recorded rows
whatever `--shoot-context` the new run passes, because the spec's resume key is
`(file, size, mtime_ns)` and its header check covers rubric and model only. The
dedupe key *does* cover the context. Guarding resume on the context too would
mean putting the state in the header and refusing to continue when it changed —
a decision for whoever owns the next ledger change, not a silent one here.

Seams the tests assert at:

- `tests/ledger.rs` (new, 18 tests) — the scoring entry point, a scripted
  `Decider` (request count = "this frame was not scored again"), and the files:
  the header names the run (version, full hash, model, endpoint, RFC 3339 start);
  a record carries the file's identity, the SHA-256 of the bytes the decider
  actually received (hashed independently in the test), the status, the answers,
  the usage and the raw document; an unreadable frame records its status and no
  digest; **acceptance 2** — cancel after two of five frames, resume, and the CSV
  is byte-identical to one uninterrupted run with three requests and five rows;
  **acceptance 3** — two full runs are byte-identical, and the ledger restarted
  rather than doubling; a half-written final line is truncated and the whole
  frames are read; a changed file (bytes, and separately mtime only) is scored
  again; a different model or rubric is fatal in both `Restart` and `Resume` with
  `--fresh` in the message and the CSV untouched, and `--fresh` then works; a
  future ledger version is refused; a file that is not a ledger is refused; a
  burst's identical preview costs one request, every NEF keeps a row, values match
  the first frame and `duplicate_of` names it; `--no-dedupe` asks three times;
  dedupe reaches outcomes restored from a resumed ledger; the CSV is rewritten
  from the ledger as rows arrive (a decider that reads the CSV mid-run sees
  0/1/2 rows with the interval at 1, and nothing before exit at the spec's 25);
  a resumed run restores failures and `--json-scores` documents byte for byte; a
  refused preflight writes no ledger.
- `cli` unit tests — the three flags, the mode mapping, and `--resume --fresh`
  being refused.
- `ledger.rs` unit tests — the RFC 3339 formatter against known timestamps, and
  the ledger's path beside the CSV.

Gates: `cargo test --locked` 165 green, `cargo fmt --all -- --check` clean,
`cargo clippy --all-targets --locked -- -D warnings` clean. `Cargo.lock` needed
no change beyond the `sha2` dev-dependency the tests hash with (`sha2` was
already a dependency).

What tickets 07 and 08 must know:

- **07.** Progress and summary must come from `Run` (`frames`, `resumed`,
  `duplicates`, `counts`, `ending`) — the ledger is the run's own record, not a
  second event stream. A resumed frame *is* a `Frame` in `Run.frames` (with the
  answers the ledger holds), so statistics over frames cover the whole shoot, and
  a deduplicated frame appears too, carrying the outcome it reused. `report::Row`
  no longer has `Row::of`; rows come from `Record::row` inside `scoring::run`.
  The CLI's interim per-frame print and summary are still `cli::score::render_run`
  and are yours to replace; `Run`'s two new counters are already rendered there
  when non-zero.
- **08.** Re-running a shoot after a rubric wording change is safe and explicit:
  the new rubric hash makes the old ledger a fatal mismatch, so the re-run needs
  `--fresh` (or a different `--out`) — that is the error, not a silent mix. The
  ledger keeps the full raw document per frame, so a later XMP writer or a
  re-read of a scored shoot needs no re-scoring.
