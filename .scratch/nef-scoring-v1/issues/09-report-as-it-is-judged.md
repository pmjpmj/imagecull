# 09: The report is written as it is judged, and a frame gets a line

**What to build:** Two changes to what a run leaves behind while it is still
running. The CSV stops being a snapshot rewritten every N rows and becomes a
render of the ledger: opened and truncated once the run owns it, appended one
row per judged frame, never truncated again mid-run — so after frame N the file
holds exactly the ledger's first N rows, and a spreadsheet or `tail -f` can
follow it. And the terminal stops drawing a line that overwrites itself: the
default mode prints one newline-terminated line per frame, whether or not
stdout is a terminal, so `score > log.txt` is a record instead of silence. The
interval that the rewriting needed (`csv_rewrite_every`, default 25) has nothing
left to mean and is deleted.

Supersedes the CSV cadence of ticket 04 and the TTY-gated progress line of
ticket 07.

**Blocked by:** none (04, 05, 07 are resolved)

**Status:** resolved

- [x] The CSV is truncated and given its header once, after preflight and before the first frame, next to where the ledger is opened
- [x] Each judged frame appends its row and no rewrite happens later, mid-run or at exit
- [x] A resumed frame gets its row too, so `--resume` rebuilds the CSV from the ledger rather than appending onto the old file
- [x] The ledger append happens before the CSV append: the durable record leads, the report is its rendering
- [x] The CSV is never behind the ledger: at the moment frame N+1 is asked, the file holds N rows
- [x] `--json-scores` still written once, at exit (`Q4`: a JSON object is not appendable line by line; the ledger already holds every document crash-durably)
- [x] Default mode prints a newline-terminated line per frame — index/total, name, four scores, stars and verdict, latency, ETA — and a duplicate names the frame it reused
- [x] Per-frame lines print whether or not stdout is a terminal; `--quiet` is the switch for no per-frame output
- [x] `--verbose` keeps its extra detail (preview bytes, full precision, failure messages)
- [x] `render` no longer takes a `live` flag, and nothing reads `is_terminal` any more
- [x] `csv_rewrite_every` is gone from `[run]`, from `Config`/`Resolved`/`Effective`, from the example TOML, and from the tests; a config file that still sets it fails as an unknown key
- [x] `SPEC.md` §5, §9.1, §9.2, §11, §11.1 and `.scratch/nef-scoring-v1/spec.md` (lines 174, 231, story 36) describe the new behaviour
- [x] `GLOSSARY.md` gains **Frame line**

## Settled decisions

- **Append, not rewrite-per-row.** Writing the whole file per row keeps the
  truncation window (a spreadsheet sees it empty and refill) and rewrites ~1 GB
  over a 4000-frame shoot. No per-row `fsync`: the ledger is the durable record,
  and `--resume` rebuilds the CSV from it.
- **Truncate at run start.** One rule for both outputs — the run owns its report
  from the moment it owns its ledger. A run cancelled before frame 1 leaves a
  header-only CSV, which at least says "this run did nothing"; leaving the
  previous run's file in place is indistinguishable from a complete one.
- **The line keeps index/total and ETA.** ETA is the only live pacing signal in
  the tool — the summary reports elapsed, not remaining — and index/total says
  where a surprising line sits in the shoot.
- **Duplicates are marked, resumed frames are not.** `duplicate_of` already
  rides on `FrameEvent`, so the clause only moves from `--verbose` to the
  default line: no change to the core's event shape. A resumed frame keeps the
  latency its judgement cost in the earlier run, and the summary's `resumed`
  count is where that is accounted for.
- **No ADR.** The CSV format does not change at all, only when it appears, and
  the reasoning fits in the spec sentences that had to change anyway.
- **`Frame line`** is the term: the per-frame record a run prints, as distinct
  from the summary block. `Progress` stops being accurate the moment the line
  survives a redirect.

## Comments

Implemented. `report::Writer` (`open`, `append`) replaced `report::write` for
the CSV: the run truncates and writes the header right after `Ledger::open`,
then appends one row per accounted frame — resumed frames included, through the
same `Account::record` door — with the ledger append first. `report::write_json`
is what remains at exit, for `--json-scores` alone. `csv_rewrite_every` is gone
from `[run]`, from `Config`/`Resolved`/`Effective`, from `imagecull.toml.example`
and from the tests; a config file that still sets it fails as an unknown key, and
`the_deleted_rewrite_interval_is_an_unknown_key` pins that. On the CLI side
`render(event, verbosity)` lost its `live` flag, `Terminal::attached` became
`Terminal::new`, `progress_line` became `frame_line` (newline-terminated, with
`duplicate of …` before the ETA), and nothing calls `is_terminal` any more.

Tests: `tests/ledger.rs::the_csv_is_never_a_row_behind_the_ledger` (the old
rewrite-interval test, now asserting `[0, 1, 2]` with no knob to turn) and a new
`a_run_cancelled_before_the_first_frame_leaves_a_header_only_csv`, which pins
that the report belongs to the run rather than to the folder it lands in.
`cli::progress`'s tests assert the exact frame line and that no `\r` or `\x1b`
reaches stdout, that a duplicate is named on the default line, and that the
summary block needs no separator. 236 tests green; `cargo fmt --check` and
`cargo clippy --all-targets --locked -- -D warnings` clean.

Documentation: `SPEC.md` §5 diagram, §9.1, §9.2, §11, §11.1; the scratch spec's
lines 174/231 and user story 36; `GLOSSARY.md` gained **Frame line** under a new
"What a run prints" heading, and stray "progress line" wording in the core's and
the CLI's doc comments became "frame line". ADR 0003 carries an amendment
recording the removed key. Two incidental fixes: the code's `spec §9.3`
references (7 of them, in `report`, `scoring` and `ledger`) pointed at a section
that does not exist — the ledger and dedupe are §9.2 — so they now say §9.2.
