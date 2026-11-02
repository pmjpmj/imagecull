# 01 — Stream groups into the cull, and judge them on workers

Status: resolved (2026-11-28)

**What to build:** `imagecull groups` starts each group's comparison as soon as
the group can no longer gain a frame, instead of waiting for the whole folder to
be read, and judges up to `--cull-jobs` groups at once. It waits for every cull
job before printing the report and exiting.

The decision, the wire change and the rejected alternatives are in
[ADR 0011](../../../docs/adr/0011-the-cull-starts-when-a-group-is-final.md); the
acceptance list and the open questions are in the
[feature spec](../spec.md).

## What changes, by file

- `crates/imagecull-core/src/preview.rs` — `metadata(path)`: the header a group is
  ordered by, without extracting the preview.
- `crates/imagecull-core/src/group.rs` — `Moment`/`Grouper` (the shared sweep),
  `Found`/`Sink`, `stream_with` (metadata pass, preview pass, capture-order
  consumption, scan-order frame events), `Foundry` (the plan and the undated
  tail). `run_with` drains the stream; `assign` drives the same `Grouper`.
- `crates/imagecull-core/src/cull.rs` — `judge` (one group), `Culled::merge`,
  `run_streaming` (worker pool, preflight, deferred singletons, fatal error in
  `Streamed`), `DEFAULT_JOBS = 4`.
- `crates/imagecull-cli/src/cli.rs` — `--cull-jobs`, refused at zero and with
  `--no-cull`.
- `crates/imagecull-cli/src/groups.rs` — drive `run_streaming` with
  `group::stream_with`; write the CSV after the plan is whole; render `culling
  groups as they are found` when the total is not yet known.
- `crates/imagecull-core/tests/{groups,groups_marking}.rs` — the streaming sink,
  a refusing sink, streamed verdicts, overlap/wait, and the preflight path.

## Notes

- **The plan is still the contract.** Streaming is a scheduling change: the same
  folder under the same thresholds produces the same plan, CSV, groups and hash.
  The existing `groups` suite now proves that through `stream_with`.
- **The CSV is written after the cull joins.** The old order wrote it between the
  passes. Streaming cannot know the groups before the read finishes, so the plan
  is written once it is whole; a failed preflight still finishes the plan and
  writes the CSV before the fatal exit.
- **Determinism of progress lines is weaker on purpose.** Frame lines stay in the
  scan's order, but a verdict block can appear before a frame line whose scan
  index was still blocked. The report sorts verdicts by group number.
