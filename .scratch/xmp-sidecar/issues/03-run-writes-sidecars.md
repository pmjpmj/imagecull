# 03: The run writes sidecars instead of a report

**Type:** task
**Status:** ready-for-agent
**Blocked by:** 01, 02
**Spec:** [../spec.md](../spec.md) §"Modules and interfaces", §"What a run does,
per frame", §"Statuses and exit behaviour"

## What to build

The core run loop stops producing a report and starts writing ratings into
sidecars. This is the ticket that deletes the ledger and the CSV, so it is the
biggest one in the slice and the one that must leave the tree coherent.

In `imagecull-core`:

- `report` becomes `sidecar`: keep the per-frame `Row`, drop `COLUMNS`, the cells
  function, the RFC 4180 writer and `Writer::open`/`append`; `Outputs` shrinks to
  `json_scores`.
- Delete the ledger module and everything that exists for it: `Record`,
  `Provenance`, `FileIdentity`, `ResumeKey`, `Header`, `LedgerMode`, the ledger
  warnings, `Run::resumed`, `Summary::resumed`. Build `Row` from a `Frame` plus
  the run's provenance instead of from a ledger record.
- Dedupe's memory becomes an in-process map from dedupe key to the earlier frame's
  answers. The key is unchanged. Dedupe no longer reaches across processes.
- `run` takes the sidecar `Writer` as a second port beside `Decider`.
- The per-frame order is the spec's, exactly: skip check (sidecar read only, no
  preview extraction) → foreign guard → prepare → dedupe → request → read → write
  → frame event.
- Statuses `xmp_write_failed` (aborts the run), `xmp_unreadable`, `xmp_foreign`
  in the taxonomy, with the exit-code behaviour the existing kinds already give.
  A skipped frame is not a failure and is not in the status clause.
- `Config` grows `force` and `allow_foreign`; `Config::ledger` disappears.
- The config file's `[outputs]` keeps `json_scores` only; `csv` and `absolute`
  are removed from the schema, and a stale key fails with a message naming what
  moved.

In the tests: delete `tests/ledger.rs`; rewrite the ledger and CSV assertions in
`tests/report.rs` (becomes `tests/sidecar.rs`), `tests/summary.rs`,
`tests/config.rs` and `tests/failure_taxonomy.rs`. `--json-scores` keeps working.

## Acceptance

- A clean run over a tempdir shoot folder leaves one `.xmp` per accepted frame, no
  `.bak`, and no other new file; a frame the model could not answer leaves no
  sidecar at all.
- A second run over the same folder makes **no requests** and writes **no files**,
  and counts every frame as skipped.
- The skip check precedes preview extraction: a decider that would panic sees
  nothing, and a frame with an unreadable preview but an existing rating of 1–5 is
  skipped rather than failed.
- `--force` re-rates; a sidecar carrying `darktable:` is `xmp_foreign` without
  `allow_foreign` and written with it; a sidecar with no `rdf:Description` is
  `xmp_unreadable` and byte-identical afterwards.
- A frame whose write fails carries `xmp_write_failed` and stops the run: the
  frames after it make no requests.
- Dedupe fires within one run (two identical previews, one request) and does not
  fire across runs.
- The ledger file, its flags and its warnings are gone from the tree and from the
  public API; `--json-scores` still writes every raw document.
- A stale `[outputs] csv` in a config file is a fatal error naming the change.

## Notes

- Read `imagecull-core/src/scoring.rs` `run`/`Account`/`judge` first: the loop is
  already shaped for this, and `Row` is already rendered from a record.
- The `Written` type from ticket 02 is what decides a `.bak`; the run does not
  decide backup policy itself.
- Do not touch the CLI (ticket 04) or the docs (ticket 05), except where the core
  API change forces a compile fix in `crates/imagecull-cli` — in that case keep it
  minimal and note it in the ticket.
