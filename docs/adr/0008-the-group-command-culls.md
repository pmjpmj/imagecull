# 0008 — The group command culls; `score` rates

Status: accepted · 2026-11-23 · superseded in part by
[ADR 0012](0012-green-marks-the-keeper-and-yellow-the-unsure.md), which writes
Green on the keeper and Yellow on a low-confidence group's non-keepers ·
amended by [ADR 0020](0020-the-cleanup-drops-doctor-and-the-measures-csv.md),
which drops the measures CSV ·
Feature `burst-grouping-v1` (ticket 03) ·
supersedes [ADR 0007](0007-the-red-label-marks-the-non-keepers.md)'s command
split, its rating-based shortlist and its "never remove a label" rule ·
keeps ADR 0007's label semantics and ADR 0006's rating-in-the-sidecar decision

## Context

ADR 0007 put the red mark in `score` because the sidecar write already lived
there, and left `imagecull groups` read-only and model-free as the diagnostic
that had shipped first. The split reads well in the ADR and badly in the hand:
the command whose entire job is deciding *what a group is* is the one command
that never decides *which frame of it to keep*. A photographer runs `groups` over
a burst-heavy shoot, reads "group 3 · 12 frames · DSC_0410.NEF … DSC_0421.NEF",
filters for Red in Lightroom, and finds nothing — because the reds were written,
or not written, by a different command under a rule nobody can see from the
report in front of them.

The rule is worse than the split. `score`'s group pass runs only over the frames
*that run rated*. So the natural sequence — rate the shoot, then look at how it
grouped — produces no marks at all: every frame already carries `xmp:Rating`, the
skip rule leaves them alone, and the group pass finds an empty folder. The one
folder a photographer most wants culled is the one where the tool declines to
cull it, and the only remedy is `--force`, which re-pays every rating to write a
label the ratings had nothing to do with.

The shortlist has the same shape of mistake. `keeper::compared` narrowed a group
larger than the request's 20-frame cap by **rating**, ties broken by acutance, so
the per-frame question — *is this a keeper on its own?* — was allowed to veto
which frames the group question was asked about. That is backwards: the
per-frame rating is the weaker judgement precisely because it cannot see the
alternatives, and the glossary already refuses to read one against the other. The
ranking rule that ADR 0007 retired as an output survived as an input, deciding who
is compared. A group of 21 frames therefore had one frame dropped before clef had
looked at it, on the strength of a number clef produced without looking at the
other 20.

The decision that follows from both failures is one sentence: **the command that
finds the group is the command that culls it, and clef decides every frame it
culls.**

## Decision

**`imagecull groups <DIR>` detects groups, asks clef which frame of each group to
keep, and writes the verdict.** After the model-free pass — scan, group, measure —
each group of two or more gets one comparative request; the frame it names is the
keeper, every other frame of that group gets `xmp:Label="Red"`, and the verdict's
provenance goes into the sidecar beside it. A group of one gets no request and no
label. `score` rates frames and does nothing else: its group pass, its `Groups`
counts and its group summary lines are deleted.

**The comparison ignores `xmp:Rating`.** A frame's rating is neither read nor
written by the group pass, and no rating shortlists, filters or orders the frames
clef is shown. The rating is the per-frame judgement; the keeper is the group
judgement; neither is evidence for the other.

**Every frame of a group is shown to clef.** The 20-frame cap is the measured
shape of one request, not a limit on who may be compared: a group larger than it
is compared in **rounds** — batches of at most 20 frames in capture order, split
as evenly as the count allows so that a remainder never stands alone as a request
about one frame, each batch answered by one request, then one final request among
the batch winners, repeated until one frame is left. Nothing narrows on a measure
or a rating, and `groupNote="not_compared"` therefore ceases to exist. The cost
is one request per batch: a 60-frame burst is four requests (three batches and a
final), where ADR 0007's narrowing made it one.

**The verdict records the shakiest call it depended on.** `pickConf` is the
lowest top-option probability on the winner's own path — not the final round's,
and not a losing batch's, because a call the winner did not survive is not
evidence about it. A tournament's last call can be a near-certainty about two
frames whose membership in the final was itself a coin flip, and a losing batch's
near-tie must not drag a certain winner's confidence down. A group below
`pick_conf_floor` is marked anyway (ADR 0007 measured near-ties at 0.55 against
0.43, so a gate would silence the feature exactly where it is needed) and counted
on one summary line. `pickRounds` records how many requests stood between the
group and its verdict.

**The label rules of ADR 0007 survive, except one.** Red still says "not the
keeper of this group", never "bad"; the keeper still gets no label, its rating
being its mark; a label that is absent or ours (`imagecull:labeledBy="imagecull"`)
is ours to write; a photographer's Yellow, Blue or Green is never overwritten and
the frame stays out of the delete set, counted; a comparative call that fails,
times out or answers nothing marks nothing and reports the group undecided; and
no label is ever written by a run that could not finish its group.

The exception: **a red this tool wrote is retired when our own verdict promotes
that frame to its group's keeper** (or leaves it in no group of two or more). ADR
0007 refused to remove any label because removing one means guessing whether we
wrote it — and `labeledBy` exists so that no guess is needed. With `score`
writing the mark once, never removing was defensible: the mark was a first-pass
artefact. With `groups` re-runnable on purpose, it is not: re-run the cull after
changing the gap, and ADR 0007's rule leaves the new keeper wearing a red that
this very run just wrote a `groupKeeper="true"` verdict next to. A red without
`labeledBy` proof is never touched, and the removal is a removal of the label only
— the verdict and its provenance stay.

**`groups` writes a sidecar but never a rating.** It merges only the group
verdict, the label and its `labeledBy` proof into whatever the frame's sidecar
already holds (creating one where there is none), so `xmp:Rating`,
`imagecull:confStars`, `imagecull:model` and `imagecull:rubricHash` remain the
rating's own provenance, written by `score` alone. `--allow-foreign` governs a
darktable sidecar exactly as it does for a rating.

**The model-free pass is kept as a flag, not as a command.** `--no-cull` runs the
scan, the grouping, the measures, the per-frame lines, the report and the CSV
exactly as ticket 01 shipped them, with no request and no write. `--print-sidecars`
is the review step: it asks clef, prints what each sidecar would become, and
writes nothing. `groups` therefore gains the server flags `score` has
(`--endpoint`, `--model`, `--timeout`, `--config`/`--no-config`), and the alias it
uses defaults to `clef-flash-oQ4e` because no rubric is selected here.

**The report becomes the cull's own account.** Each group line names its keeper
and how many frames it marked; the summary counts groups, red, unmarked (label not
ours), undecided, low-confidence and rounds. The CSV stays what it is — the
measures, for judging the measures — and gains no keeper column: the sidecar is
where a verdict lives.

## Considered options

- **Keep ADR 0007's split: `score` marks, `groups` reports.** It is the status
  quo, and the status quo is the bug: the command that names the duplicates does
  nothing about them, and a folder that was rated before this feature can never be
  marked without re-paying every rating.
- **Let both commands mark.** Doubles the writers of one fact. A `groups` run
  after a `score` run re-asks the same question with a fresh sample from a
  coin-flip distribution (ADR 0007's own measurement), so the two commands can
  disagree about the keeper of a near-tie and the folder ends up carrying both
  verdicts' reds.
- **Mark from the ranking rule, with no model.** `groups` would stay model-free
  and the command would answer. But it answers with the measure-based ranking rule
  that ADR 0007 already demoted, on a shoot whose whole point is that the
  photographer cannot see twelve frames at once. The model was asked the relative
  question because it is better at it than a weighted shortlist of acutance and
  roll.
- **Narrow groups over 20 frames by rating, as ADR 0007 did.** Lets the weaker
  per-frame judgement decide whom the stronger group judgement may look at, and
  drops frames from the cull that the photographer never sees excluded. It also
  makes the mark depend on whether `score` ran first, which is exactly the coupling
  this ADR removes.
- **Mark a group too large for one request `undecided`.** Honest, and it marks
  nothing rather than guessing; but it silences the feature on long bursts, which
  is the one case where a photographer needs it most. Rounds cost an extra request
  and stay inside the measured request shape.
- **Never remove a label, as ADR 0007 decided.** Correct while the mark was
  written once, by a command with a skip rule. Wrong once the cull is repeatable:
  the flip between near-ties is normal, and the new keeper would wear the old red
  the run had just contradicted.
- **Green on the keeper.** Still wrong, for ADR 0007's reason: it claims the
  photographer's label on the frame they are least likely to want labelled, and
  the keeper is already identifiable by stars plus `groupKeeper="true"`.

## Consequences

- **Culling is two commands, or one.** `score` rates, `groups` culls. A shoot
  that needs both runs both; a folder already rated — including every folder
  scored before this feature existed — runs `groups` alone and needs no `--force`.
  This is the price of one owner per fact, and it is the cheap direction: grouping
  and comparing cost one request per group, rating costs one per frame.
- **`groups` is no longer read-only and no longer model-free by default.** The
  ticket 01 acceptance line "`imagecull groups` remains read-only and makes no
  request" becomes false and is replaced; `--no-cull` is what keeps that evidence
  path alive.
- **`score` loses its group pass.** Its `Config.grouping`/`Config.comparison`, the
  `GroupPass`, `Groups`, `Moment` and `Marking` machinery move out of `scoring`
  into a cull module that `groups` drives. The rating's own summary loses its red
  counts and gains nothing: a rating run has no opinion about groups.
- **A group's cost is now paid by the command that asks the group question.**
  ADR 0007 priced the comparative call as an addition to a scoring run; it is now
  the whole of what `groups` spends, and a `--no-cull` pass spends nothing.
- **Long bursts cost rounds, and a round is a new place for a coin flip.** The
  tournament is unmeasured: it is the only way to keep every frame in front of
  clef inside the request shape ticket 02 verified, and if a real shoot shows the
  rounds disagreeing with the photographer more than a single call does, the
  fallback is to raise the measured cap, not to reintroduce a shortlist. The
  ticket carries that as its open question.
- **A red written by a photographer over our own red is indistinguishable from
  ours.** `labeledBy` records who wrote the label, not who last edited it. Removing
  our own red on a promotion can therefore remove a colour the photographer
  re-applied. The alternative — never removing — leaves a red on the keeper, which
  is worse and silent. `--print-sidecars` is the review step that shows the
  removal before it happens.
- **A mark that cannot be written is a failed row, not the end of the run.**
  `score` aborts on `xmp_write_failed` because every remaining frame would fail
  identically and each was already paid for; a cull's remaining verdicts are not
  paid for yet, and writing them beats leaving the delete set half-applied with no
  way to see which half. The run says so per frame, counts them, and exits `1`.
- **`imagecull:groupNote` and `not_compared` retire.** A build that no longer
  writes them retires them from sidecars it touches, the same rule every other
  retired property follows.
- **`SPEC.md` §9's label paragraph and the burst-grouping spec's ticket table are
  now describing the wrong command** and are corrected with the code, not before
  it: the docs describe what a build does.
