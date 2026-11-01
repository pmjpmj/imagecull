# 03: The group command finds the group, asks clef, and culls it

**What to build:** `imagecull groups` stops being a diagnostic that reads a shoot
and becomes the command that culls it. After the model-free pass it already runs —
scan, group, measure — every group of two or more gets one comparative request per
round, the frame clef names is the keeper, and every other frame of that group gets
`xmp:Label="Red"`. `score` stops grouping and stops writing labels: it rates frames
and nothing else.

The comparison reads no `xmp:Rating` and writes none. A group larger than one
request can carry (20 frames, the measured shape) is compared in rounds of at most
20 and then among the chunk winners, so no frame is dropped from the comparison by
a measure or a rating. [ADR 0008](../../../docs/adr/0008-the-group-command-culls.md)
holds the decision and supersedes the command split, the rating shortlist and the
never-remove rule of [ADR 0007](../../../docs/adr/0007-the-red-label-marks-the-non-keepers.md).
The label semantics of ADR 0007 survive: Red means "not this group's keeper", a
photographer's label is never overwritten, an undecided group marks nothing, and
`imagecull:labeledBy="imagecull"` is the proof a later run needs before touching a
label we wrote.

**Blocked by:** nothing — 02 is resolved and ADR 0008 is accepted.

**Status:** resolved (2026-11-23) — the cull moved out of `score` into a core pass
that `imagecull groups` drives, and every box below names the test that proves
it.

## What changes, by file

- `crates/imagecull-core/src/scoring.rs` — delete `GroupPass`, `Groups`, `Moment`,
  `Marking`, `carried_by_comparison`, `Config.grouping` and `Config.comparison`;
  delete the group lines from the run summary. `score` keeps `persist` and the
  rating sidecar write.
- `crates/imagecull-core/src/cull.rs` *(new)* — the cull pass: given the group
  pass's `Plan`, a `Decider`, a `sidecar::Writer` and a `Budget`, it asks each
  group's rounds, merges the verdict into each member's existing sidecar (or a new
  one), writes the label and its proof, and returns counts. This is where the
  `Marking` logic and the mark/unmark decision now live, so `groups` has no
  sidecar knowledge of its own.
- `crates/imagecull-core/src/keeper.rs` — delete `Candidate`, `compared`,
  `sharpness`; add `rounds(members, max)` (chunking in capture order, and the
  winner set of the next round) and `pick_conf` (the lowest confidence along the
  winner's path). `Comparison`, `Choice`, `read`, `INSTRUCTIONS`, `BY_COMPARISON`
  and `Budget` stay.
- `crates/imagecull-core/src/sidecar.rs` — `Mark` gains `rounds`, loses `note`;
  `groupNote` and `NOT_COMPARED` become retirable; a red with
  `labeledBy="imagecull"` is removable when the verdict is `keeper`.
- `crates/imagecull-cli/src/cli.rs` — `GroupsArgs` gains `#[command(flatten)]
  common: CommonArgs`, `--no-cull`, `--print-sidecars`, `--allow-foreign`; it keeps
  `--out`, `--recursive`, `--limit`, `--group-gap`, `--group-similarity`, `--jobs`,
  `--quiet`, and gains `--verbose`.
- `crates/imagecull-cli/src/groups.rs` — drive the cull pass, render a group line
  that names the keeper and the red count, and a summary that counts red, unmarked,
  undecided, low-confidence and rounds.
- `.scratch/burst-grouping-v1/spec.md`, `SPEC.md` §9, `GLOSSARY.md` — corrected in
  the same commit as the code (see Notes).

## Acceptance

- [x] `imagecull groups <DIR>` makes one comparative request per group of two or
      more and marks every non-keeper `Red`; a group of one gets no request and no
      label — `groups_marking::one_request_decides_the_group_and_every_other_frame_wears_the_red`, `groups_marking::a_frame_of_its_own_is_compared_with_nothing_and_marked_with_nothing`
- [x] A frame already carrying `xmp:Rating` is compared and marked; a folder rated
      by `score` culls with `groups` alone and no `--force` — `groups_marking::the_cull_reads_no_rating_and_writes_none`
- [x] The comparison reads no rating: a group whose ratings are 1s and 5s is
      compared in full, and `xmp:Rating` before and after is identical for every
      frame — `groups_marking::the_cull_reads_no_rating_and_writes_none`
- [x] A group of 21+ frames is compared in rounds of at most 20, every frame is
      carried by exactly one first-round request, and the winner is the survivor of
      the final round — `keeper::a_group_over_one_request_is_split_in_capture_order_and_drops_no_frame`, `groups_marking::a_group_larger_than_one_request_is_compared_in_rounds_and_drops_no_frame`
- [x] `pickConf` for a tournament group is the lowest confidence on the winner's
      path, and `pickRounds` says how many requests stood between the group and its
      verdict — `keeper::the_lowest_confidence_on_a_winner_path_is_the_one_recorded`, `groups_marking::a_group_larger_than_one_request_is_compared_in_rounds_and_drops_no_frame`, `sidecar_document::a_verdict_names_how_many_rounds_it_took`
- [x] A comparison that fails, times out or answers nothing marks nothing and is
      reported undecided — `groups_marking::a_comparison_that_fails_marks_nothing_and_reports_the_group_as_undecided`, `groups_marking::a_comparison_that_answers_nothing_is_undecided_rather_than_a_guess`, `systemone_http::a_group_that_413s_is_undecided_and_marks_nothing`
- [x] A photographer's Yellow on a non-keeper is left alone, the verdict is still
      written, and the summary counts it — `groups_marking::a_label_the_photographer_wrote_is_left_alone_and_the_verdict_is_still_recorded`, `sidecar_document::a_label_that_is_not_ours_to_write_is_never_overwritten`
- [x] A red this tool wrote is removed when a later run promotes the frame to its
      group's keeper (or leaves it in no group), and a red without `labeledBy`
      proof never is — `groups_marking::a_promoted_keeper_loses_the_red_this_tool_wrote`, `groups_marking::a_frame_that_is_no_longer_in_a_group_loses_the_red_we_wrote`, `groups_marking::a_red_without_our_proof_survives_a_promotion`, `sidecar_document::a_promoted_keeper_loses_the_red_this_tool_wrote`
- [x] `groups` writes no `xmp:Rating`, `confStars`, `model` or `rubricHash`: those
      remain `score`'s provenance, and a `groups` run on an unrated folder creates a
      sidecar that says only what it decided — `groups_marking::one_request_decides_the_group_and_every_other_frame_wears_the_red` (the keeper's new sidecar holds no rating property)
- [x] `--no-cull` runs the whole model-free pass: no request, no sidecar write, the
      same per-frame lines, report and CSV as ticket 01 —
      `groups_command::no_cull_runs_the_technical_pass_with_no_server_anywhere`, `groups::a_pass_that_left_the_cull_out_says_nothing_about_one`
- [x] `--print-sidecars` asks clef, writes nothing, and shows a removal as a
      removal — `printer::a_removal_is_shown_with_the_colour_that_was_there_before`, `groups_marking::a_preview_run_asks_the_group_and_writes_nothing`
- [x] A group whose member preview cannot be rendered marks nothing rather than
      sending a smaller request — `cull::tests::an_unrenderable_member_leaves_the_group_undecided_without_a_request`
- [x] A group below `pick_conf_floor` is marked and counted on the low-confidence
      summary line — `groups_marking::a_group_the_model_was_unsure_about_is_marked_anyway_and_counted`, `groups::the_summary_names_the_cost_the_cull_the_hash_and_the_file`
- [x] `score` makes no comparative request and writes no label: its test suite
      contains no `compare` and no `xmp:Label` write — `cargo test -p imagecull-core --test scoring --test summary`, and `scoring.rs` no longer mentions `keeper`
- [x] `groups` fails fatally on a server that cannot pass the preflight, before the
      first frame's sidecar is written; a folder with no group is judged without a
      server at all — `groups_marking::a_server_that_fails_the_preflight_marks_nothing`, `cull::tests::a_folder_with_no_group_is_culled_without_a_server`, `groups_command::a_folder_with_no_group_asks_no_server_for_a_question_it_cannot_ask`
- [x] The CSV columns are unchanged, and a group line names the keeper and the
      number of frames it marked — `groups::a_group_line_names_the_keeper_and_the_reds`
- [x] ADR 0008 is `accepted`; ADR 0007 carries a superseded note pointing at it;
      `SPEC.md` §9, `GLOSSARY.md` and the burst-grouping spec's ticket table
      describe `groups` as the cull and `score` as the rating

## Open questions

- **Is a tournament's keeper as good as a single call's?** Ticket 02 measured one
  request's behaviour (16,384 image tokens, 512 px decisive on a real difference,
  a coin flip on a true near-tie) and nothing about rounds. Before this ships as
  accepted, run a real 30–60-frame burst through both shapes — 20-frame rounds and
  the same group narrowed in one call — and record agreement in
  `.scratch/burst-grouping-v1/evidence/`. If rounds disagree with the photographer
  more often, raise the measured cap instead of reintroducing a shortlist.
- **Does a 20-image request cost what ticket 02 measured when the images are
  near-identical?** Ticket 02's 11.5 s figure was for a mixed set; a burst is the
  best case for the server and the worst case for the answer, so a group's latency
  belongs beside its confidence in the evidence.
- **Should `groups` offer `--force`?** Under this ADR every run re-culls, so there
  is nothing to force. If the tournament is dropped, a "reuse a verdict under the
  same grouping hash" rule would need one, and would reintroduce the first-pass
  artefact this ADR removes.

## Notes

- **Culling is two commands, or one.** `score` rates, `groups` culls. The sequence
  a photographer runs is `score` (one request per frame), `groups` (one request per
  group, plus rounds). A folder already rated runs `groups` alone.
- **The deletion is the point.** `scoring.rs` loses ~350 lines of group machinery
  and its tests; `marking.rs`'s tests move to `groups_marking.rs` with the cull.
  Splitting the cull out is what makes `groups` the only writer of a mark and
  `score` the only writer of a rating — one owner per fact.
- **Exit codes.** A group that is undecided or a mark that could not be written is a
  failed row for `groups` (`exit 1`), as a failed measure already is; a run that
  cannot reach the server is fatal (`exit 2`) before any mark is written.
- **First-pass artefact, retired.** ADR 0007 accepted that a cheap re-run writes no
  labels; this ticket makes the opposite true on purpose. What survives is the
  reason the old rule existed — never touch a label whose provenance is not ours —
  now enforced by `labeledBy` rather than by never removing anything.
- **`SPEC.md` §7.1 and §9.** The systemone serialisation note still holds (one
  group request at a time). §9's "`xmp:Label` is written in exactly one case"
  keeps its meaning but points at ADR 0008, and the open-question row about
  multi-image requests in the burst spec is answered by this ticket rather than
  deferred.

## Comments

### What landed, and where it differs from the ticket as written (2026-11-23)

- **`--no-cull`, not `--dry-run`.** `score`'s `--dry-run` means "prepare the bytes
  a request would carry"; here the distinction is "do not ask the group question
  at all", and there is no preview directory to write. `--print-sidecars` keeps
  the other axis (`score`'s own name for "ask, but do not write").
- **A comparison is retried on the spec's transient schedule**, like a per-frame
  request. The old group pass did not retry; sharing `scoring::retrying` was
  cheaper than explaining why the two paths disagree about a timeout.
- **`imagecull:pickRounds` was added, and `imagecull:groupNote` retired.** The
  round count is the fact a reader needs to know a verdict was not one request's
  certainty; `not_compared` is impossible now that every frame is compared.
- **`Mark` carries `retire_label`, and the sidecar module checks provenance
  itself.** A caller can ask for a retirement, but `render_mark` removes
  `xmp:Label` only when the document also carries our `labeledBy`. The rule lives
  where `label_is_ours_to_write` lives, not in a caller that could forget it.
- **No skip rule, no `--force` on `groups`.** Every run re-culls: there is no
  rating to skip on, and a cull that could not be repeated would leave the stale
  red ADR 0007 had to accept.
- **`score` lost the group pass entirely**, not just its writes: `Config.grouping`,
  `Config.comparison`, `Run.groups`, `Frame.rating_document`, `Groups`, `Moment`,
  `Marking` and `GroupPass` are gone, and the summary's group clause with them.
  The cull lives in `imagecull-core/src/cull.rs` and is driven by `groups`.
- **One open question remains open.** The tournament is still unmeasured; see
  below. The ticket is resolved for the behaviour the ADR decided, and the
  evidence work is a follow-up on a real burst.
