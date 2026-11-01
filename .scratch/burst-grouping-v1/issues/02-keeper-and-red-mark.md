# 02: The group's keeper, and the red mark on everyone else

**What to build:** `score` stops judging frames one at a time and judging groups
not at all. After a group's frames have been rated, one comparative request asks
clef which frame of the group to keep; the keeper gets no label at all, and every
other frame of the group gets `xmp:Label="Red"` — the only field a cull can filter
on that is portable between applications. A red frame is not a bad frame: it is a
frame that lost its group, and a 4★ red is correct when the group's keeper is a
5★. No rating is ever changed by this pass, and no measure changes a rating.
[ADR 0007](../../../docs/adr/0007-the-red-label-marks-the-non-keepers.md) holds
the decision; `.scratch/xmp-sidecar/spec.md` still governs everything else about
the sidecar, including its rule that a label we did not write is never overwritten
and never removed.

The verdict is recorded beside the rating in our own namespace, so a red can
always be explained without the folder:
`imagecull:groupKeeper` (`true`/`false`), `imagecull:groupStart`,
`imagecull:groupSize`, `imagecull:groupingHash`, `imagecull:pickBy`,
`imagecull:pickConf`, `imagecull:pickedAt`, and `imagecull:labeledBy="imagecull"`
on the frames whose label this tool wrote. `groupKeeper` is not a copy of the
label: the label can be missing (not ours to write) or foreign (the photographer's
Yellow), and the verdict still exists.

**Blocked by:** 01

**Status:** resolved (2026-11-21) — four commits on `feat/best-of-group`
(`8250350`, `b651034`, plus the measurement and the ADR), the whole acceptance
list pinned by test, and both code-review findings fixed. Every box below names
the test that proves it.

## Measured — the needs-info, answered

Against `clef-flash-oQ4e` on `http://127.0.0.1:8000`; the harnesses, raw rows and
the request/response pair are recorded in
`.scratch/burst-grouping-v1/evidence/comparison-budget.md` (and the shape-only
pair beside it).

- **The cap is tokens, not frames: 16,384 image tokens, hard.** Tokens scale with
  pixel area — 1,639/frame at 1568 px, 696 at 1024, 201 at 512 — so one request
  holds **9 frames at 1568 px, 23 at 1024, 80 at 512**.
- **Over the cap is a 413, never a silent drop.** Images are outside `truncate`
  (only `state` is truncatable), so a group either gets a whole-group answer or
  an error the run can report as undecided. The feared "truncated burst answers
  about a subset" does not exist.
- **The answer follows the image, not the position**, and 512 px is decisive on a
  real difference (sharp vs. soft: 0.98, both orders). On a genuine burst
  near-tie it is a coin flip: the pick flips between orderings and preview
  sizes at 0.55 against 0.43.
- **The server serialises `/v1/systemone`**, confirmed; a parallel group fan-out
  buys nothing (SPEC §7.1 stands).

Consequences taken:

- **Comparison previews are rendered at 512 px** (`comparison_edge`, default 512)
  and up to **20 candidates** (`comparison_max`) go in one call — 4,154 tokens,
  25% of the budget, 11.5 s, measured. The cap is not reached at 512 px until 81
  frames.
- **No measure-based shortlist for context reasons.** There is no shortlist; a
  group of 21+ frames is narrowed to 20 by the rating, ties broken by acutance —
  the only place this ticket lets a measure choose anything, and it chooses who
  is *compared*, never a rating.
- **A low-confidence comparison marks anyway.** `pickConf` is recorded on every
  member; a group below `pick_conf_floor` (0.60) is marked and reported on one
  summary line. The measurement is the reason the floor is a warning and not a
gate: the near-tie case is the common case, and refusing to mark a burst would
  make the feature useless exactly where a burst is.

## Acceptance

- [x] A group of two or more gets exactly one comparative request; a group of one gets none — `marking::a_group_of_two_or_more_gets_one_comparative_request_and_a_lone_frame_gets_none`
- [x] The comparative request never exceeds the measured budget: `comparison_edge` (default 512) and `comparison_max` (default 20), enforced before the request is built, never by reading a 413 — `keeper::the_budget_is_the_measured_configuration_and_not_a_guess`, `marking::a_group_larger_than_the_request_can_carry_is_narrowed_…`
- [x] A group of 21+ frames is narrowed to 20 by rating, ties broken by acutance, and every frame left out is red with `groupNote="not_compared"` — `keeper::an_equal_rating_is_broken_by_acutance_…`, `marking::a_later_groups_left_out_frame_is_the_one_that_says_so` (the review's blocking finding: two groups, the second starting above frame 0)
- [x] `pickConf` is recorded on every member of a marked group; a group below `pick_conf_floor` (0.60) is still marked and counted on a summary line (`low confidence: 2 groups`) — `marking::the_keeper_wears_no_label_and_every_other_frame_of_its_group_wears_red`, `marking::a_group_the_model_was_unsure_about_is_marked_anyway_and_counted`
- [x] A 413 is a failed frame with its own status, never a panic and never a silently smaller request — `systemone_http::a_group_that_413s_names_its_status_in_the_warning_a_photographer_reads`, `marking::a_group_the_model_cannot_hold_is_reported_with_its_own_status_and_marks_nothing`
- [x] The keeper carries no `xmp:Label` write; every other frame of the group carries `Red` — `sidecar_document::a_keepers_verdict_leaves_the_red_a_previous_run_wrote_exactly_where_it_was`
- [x] A frame whose label is the photographer's is left alone, its verdict still recorded, and counted on a summary line (`unmarked: 3` — label not ours) — `marking::a_label_the_photographer_wrote_is_left_alone_and_the_verdict_is_still_recorded`
- [x] A frame whose label was written by a previous run is rewritten only if that run recorded `imagecull:labeledBy="imagecull"` — `sidecar_document::a_label_that_is_not_ours_to_write_is_never_overwritten`
- [x] No label is ever removed: a frame promoted to keeper by a later grouping keeps its red, and the summary warns that the folder disagrees with the current grouping — `marking::a_frame_promoted_to_keeper_keeps_its_red_and_the_run_warns_the_folder_disagrees`
- [x] A comparative call that fails, times out or answers nothing marks nothing and reports the group as undecided — `marking::a_comparison_that_fails_marks_nothing_and_reports_the_group_as_undecided`, `marking::a_comparison_that_answers_nothing_is_undecided_rather_than_a_guess`
- [x] `xmp:Rating` is identical before and after the group pass for every frame — `marking::no_comparative_answer_changes_a_rating`
- [x] A frame's rating survives a group pass that excludes it; excluded frames are red and still rated — `marking::a_group_larger_than_the_request_can_carry_…`
- [x] `--print-sidecars` shows the label line per frame — `xmp:Label  Red  (was: none)` — and writes nothing — `printer::a_marked_frame_shows_the_label_line_with_the_colour_that_was_there_before`, `marking::a_forced_preview_shows_the_verdict_beside_the_rating_this_run_gave`
- [x] `imagecull groups` remains read-only and makes no request; its test suite still contains no HTTP — untouched: `cargo test --test groups` 18/18, and no socket in `cli/src/groups.rs`
- [x] Marks are written after the frame's rating, so the terminal never reports a mark whose rating is not on disk — `marking::every_frame_is_rated_on_disk_before_any_group_is_judged`

## Notes

- **Why the losers and not the keeper.** The keeper is already the frame with the
  best stars; the non-keepers are the frames the photographer has to get past, and
  Red on them turns the cull into a filter rather than a search.
- **Why `groupKeeper` is not a second copy of the label.** The label is one
  rendering of a verdict that may be missing or foreign; recording the verdict
  separately is what lets the summary say "12 non-keepers, 9 red, 3 unmarked".
  The ADR 0006 objection ("two properties holding one number") does not apply:
  these are a judgement and its rendering, and the run warns when they diverge.
- **The stale-red warning is the only remedy.** Removing a label means guessing
  whether we wrote it, and a wrong guess deletes the photographer's own mark. So
  the run reports `12 red, 2 now the keeper` and leaves the files alone.
- **Cost.** One request per group on top of the per-frame pass: ~430 requests for
  400 frames in 30 groups. A `--force` re-run re-pays both.
- **First-pass artefact.** The skip rule leaves already-rated frames alone, so
  labels are written by the run that rated the folder and never refreshed by a
  cheap re-run. That is accepted in ADR 0007 and has to appear in the docs as a
  fact, not a surprise.
- **The ranking rule survives as a shortlist.** If the comparative call's context
  cap binds, its inputs become the measures' exclusions (camera focus report,
  acutance floor, roll) plus the per-frame rating — the same ordered criteria,
  now choosing candidates rather than a winner. That is why issue 01's blind
  review is still on the critical path.

## Comments

### Follow-ups — smells the review found and deliberately left (2026-11-21)

Both are real, and both are larger than the review-fix pass that recorded them.
Neither is a bug today: the index ambiguity they share caused exactly one defect
(`groupNote="not_compared"` written from place-space into member-space, fixed
and pinned in `crates/imagecull-core/tests/marking.rs`).

- **`Moment` holds `rated` and `readings` in parallel.** A group's place becomes
  an index into each of them (`Moment::member`), so every read of a frame is a
  double index and the two spaces can be confused again. One per-frame struct
  — source, stars, the document the rating wrote, the facts, the measures —
  would carry the group's membership once and remove the conversion entirely.
- **`Groups` is nine bare `usize` fields.** They are counted in one loop, printed
  in one clause, and nothing stops a new count from being added to `Groups` and
  forgotten by the summary. Typed counters, or a `Counted` wrapper that the
  printer reads, would make "every count appears on the line" a property of the
  type rather than of a reviewer's attention.

### Resolved (2026-11-21)

Landed on `feat/best-of-group`: `keeper` (the measured budget, the shortlist, the
comparison, the strict reading of an answer), the `GroupPass` in `scoring`, the
`imagecull:` verdict properties and the label write in `sidecar`, and the
group/label/low-confidence/marked clauses in the terminal.

What the run does at the end of a scored folder: one comparative request per
group of two or more (512 px, at most 20, built to fit — never a 413 discovered),
the keeper unmarked, every other frame `Red` where the label is ours to write,
`pickConf` on every member, and one line each for the non-keepers, the labels
left to the photographer, the groups the model was unsure about, and the frames
the skip rule kept out of every group.

Two-axis code review found one blocking defect, fixed in `b651034`: the group
note compared a frame's place in its group against its index in the run, which
agrees only for the folder's first group. Both reviewers found it independently
and no test did, because every test had one group starting at frame 0. The fix
routes both the note and the summary's count through one question, and the test
that would have caught it (`a_later_groups_left_out_frame_is_the_one_that_says_so`)
now builds two groups with the second starting above frame 0.

Evidence: `.scratch/burst-grouping-v1/evidence/comparison-budget.md` (the
measurement, including the near-tie the marking rule accepts).
