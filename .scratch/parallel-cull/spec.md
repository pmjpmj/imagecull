# Feature — the cull starts when a group is final

`imagecull groups` runs two passes: a model-free grouping pass and a cull. In the
shipped shape the cull waited for the grouping pass to finish, then compared
groups one after another. A group's comparison is a round trip of seconds, so a
shoot of many groups spent most of its wall clock waiting.

This feature overlaps them. The grouping pass streams: a group is final the
moment the next frame in capture order is beyond its gap, and the cull starts that
group on a worker while later previews are still being decoded. `--cull-jobs`
groups are culled at once, and the command waits for every job before printing the
report and exiting.

The decision is recorded in
[ADR 0011](../../../docs/adr/0011-the-cull-starts-when-a-group-is-final.md).

## What changes

- **`group` gains a metadata pre-pass and a streaming sweep.** Every header is
  read first (capture time, body, exposure), which is what places frames in
  capture order without their signatures. Previews are still read in the scan's
  order; their readings are consumed in capture order, and a group closes using
  the next frame's header before that frame's preview exists.
- **`group::Sink` / `group::Found` / `group::stream_with`.** The pass hands each
  finished group over as it is final. `run_with` keeps its signature and its plan
  by draining the stream into an ignoring sink; `assign` and the streaming sweep
  share one `Grouper`.
- **`cull::judge` and `cull::run_streaming`.** One public per-group judge, and a
  streaming pass that runs up to `--cull-jobs` (default 4) groups on workers,
  preflights once before the first dispatch, defers singletons until the folder has
  shown whether a server is needed, and returns the whole plan alongside the
  counts.
- **A preflight that fails no longer loses the measures.** The grouping pass
  finishes, the caller writes the CSV, then the run exits fatal.
- **`--cull-jobs <N>`** on `groups`; conflicts with `--no-cull`.
- **`SPEC.md` §7.1/§7.3/§14 and `omlx.rs`** stop promising that every request is
  one at a time: `score` still is, the cull's degree is the flag. The report is
  sorted by group number however the workers finish.

## Non-goals

- `score` stays sequential: per-frame ratings are one request at a time.
- No batching of several frames into one per-frame decision.
- No change to what a group is, what a keeper is, or which labels are ours.
- No change to the CSV columns or to the final report's shape.

## Acceptance

- [x] A group is handed over as soon as the next frame's capture time is beyond
      its gap, and the plan, CSV, groups and hash are the same as the batch
      `assign` produced — `groups::a_streaming_pass_hands_every_group_to_the_sink_in_the_plans_own_order`,
      and the existing `groups` suite (which now runs through `stream_with`)
- [x] Frame lines still arrive in the scan's order at any degree —
      `groups::the_pass_says_each_frame_as_it_reads_it_in_the_passes_own_order`,
      `groups::the_lines_are_the_same_at_any_degree_of_parallelism`
- [x] A sink that refuses a group stops the pass —
      `groups::a_sink_that_refuses_a_group_stops_the_streaming_pass`
- [x] Streaming judges every group, writes the same verdicts, and reports them in
      group order — `groups_marking::a_streamed_cull_judges_every_group_and_writes_its_verdicts`
- [x] Several groups' comparisons are in flight together, the preflight is asked
      once, and the pass waits for every one —
      `groups_marking::a_streamed_cull_has_several_groups_in_flight_and_waits_for_every_one`
- [x] A failed preflight stops the cull, leaves the plan whole, and marks nothing —
      `groups_marking::a_streamed_preflight_failure_leaves_the_plan_and_writes_nothing`
- [x] A streamed folder of singletons asks no server, and a singleton's stale red is
      retired once the preflight has passed —
      `cull::tests::a_streamed_folder_of_singletons_asks_the_server_nothing`,
      `groups_marking::a_streamed_cull_retires_a_singletons_red_after_the_preflight`
- [x] `--cull-jobs` parses, refuses zero, and conflicts with `--no-cull` —
      `cli::groups_takes_its_own_thresholds_and_scan_flags`,
      `cli::a_gap_and_a_similarity_outside_their_range_are_refused`,
      `cli::groups_takes_the_server_and_the_cull_flags`
- [x] `--no-cull` still runs the whole technical pass with no server, and an empty
      folder still says `culling 0 groups` — `groups_command::no_cull_runs_the_technical_pass_with_no_server_anywhere`,
      `groups_command::a_folder_with_no_group_asks_no_server_for_a_question_it_cannot_ask`

## Open questions

- **Does the extra header open cost more than the overlap saves on a spinning
  disk or a network share?** The design assumes a header parse is small against a
  preview extraction. A real shoot on an SD card should be measured before the
  pre-pass is treated as free.
- **What `--cull-jobs` default suits a real server?** Four was chosen to overlap
  local rendering with a wait without holding several large groups' previews at
  once. A serialised oMLX will not reward more; a server that batches might. The
  flag exists so a shoot can answer this rather than a guess.
