# 0007 — The red label marks the non-keepers, never the keeper

Status: accepted · 2026-11-19 · Feature `burst-grouping-v1` (ticket 02) ·
amends [ADR 0006](0006-the-rating-lives-in-the-sidecar.md)'s "never write
`xmp:Label`" · amends the ranking rule of
`.scratch/burst-grouping-v1/spec.md` · supersedes `SPEC.md` §9's label paragraph

## Context

Grouping and measuring landed model-free (ticket 01). The spec's answer to "which
one of these twelve" was a ranking rule: exclude on the camera's focus report, on
acutance, on roll; then let the highest rating among survivors win, and report
`group_rank`, `is_best` and `group_note` as CSV columns beside a ledger record.

Both of those containers are gone. ADR 0006 deleted the CSV and the ledger, and
the sidecar — the rating plus its provenance — is now the run's only lasting
record of a frame. So a group's verdict has exactly one place left to live, and
the question is what property carries it.

The obvious property is wrong. A group rank is a fact about the *shoot*: the
glossary already refuses to compare ranks between groups, and a rank read from a
sidecar in year two, without the folder, the grouping hash and the other eleven
frames, is unreadable. A property that cannot be read on its own is decoration.

What the photographer needs is narrower and more durable: **the frames they will
not open**. The keeper is already the frame with the best stars, so a mark on it
earns nothing. The other eleven are the ones the cull has to get past, and in
Lightroom the only portable way to say "these" is the colour label.

That collides head-on with the rule the sidecar feature wrote down: `xmp:Label`
is the photographer's, and the tool never touches it. The collision is real and
this ADR resolves it rather than working around it.

## Decision

**A group of two or more gets one comparative request, and its non-keepers get
`xmp:Label="Red"`.**

- **The keeper gets no label.** Its rating is the mark. Writing Green on it would
  be a second opinion on the frame the photographer is most likely to label
  themselves.
- **Red is written on every other frame of the group**, whether it lost on
  focus, on acutance, on roll, or on clef's comparison. Red says "not the keeper
  of this group", **not** "bad": a 4★ frame that lost to a 5★ is correctly red,
  and a red frame may be the second-best picture of the shoot.
- **Every other frame's rating is untouched.** The comparative call never writes
  `xmp:Rating`, and no measure changes a rating. The rating stays a per-frame
  judgement; the mark is a group judgement.
- **A label is only written when it is ours to write**: absent, or written by a
  previous run (recorded as `imagecull:labeledBy="imagecull"`). A photographer's
  Yellow, Blue or Green is never overwritten, and the run counts the frames it
  left unmarked for that reason. The delete set is therefore sometimes incomplete,
  and says so.
- **A label is never removed.** A frame promoted to keeper by a later run keeps
  the red a previous run wrote; the summary warns that the folder carries marks
  its own grouping disagrees with. The tool does not clean up the photographer's
  field, and it does not pretend a red it wrote is now not there.
- **Nothing is marked on no answer.** A comparative call that fails, times out or
  returns no choice marks nothing and reports the group as undecided. A singleton
  group makes no call and marks nothing.
- **`imagecull groups` stays read-only and model-free.** Marking happens in
  `score`, after that frame's rating has been written, and only when the group is
  complete.
- **The verdict and its provenance live in our namespace**, so a red can always
  be explained: `imagecull:groupKeeper` (`true`/`false`), `imagecull:groupStart`,
  `imagecull:groupSize`, `imagecull:groupingHash`, `imagecull:pickBy`,
  `imagecull:pickConf`, `imagecull:pickedAt`, `imagecull:labeledBy`.

## Considered options

- **Green on the keeper.** The mark lands on the frame that needs it least, and
  it claims the photographer's label on the one frame they are most likely to
  want to label. The keeper is already identifiable by its stars; the losers are
  not.
- **Write nothing to the sidecar; print the picks.** The cull does not happen in
  the terminal. A verdict the photographer has to retype is the failure ADR 0006
  exists to end.
- **`imagecull:groupRank` in our namespace only.** Legible to exiftool and to us,
  invisible in Lightroom's grid and filters, and stale the moment the folder
  changes. It records the decision without ever delivering it.
- **`xmpDM:pick` / `xmpDM:good`.** The nearest standard field, and the pick flag
  the photographer uses for exactly this. Taking it is a larger claim than taking
  one of five colours, and it destroys the photographer's own pick.
- **`xmp:Rating="-1"` on the losers.** The destructive reject, explicitly refused
  since the sidecar feature: a low rating is never silently a reject.

## Consequences

- **One extra request per group of two or more.** A 400-frame shoot in 30 groups
  pays 400 per-frame calls plus up to 30 group calls. The group call carries
  several previews, so it needs its own context budget — a cap on how many
  candidates are compared, and a preview size the model can hold. Until that cap
  is measured, the feature is not buildable for large groups.
- **The verdict is not attributable to one frame.** A multi-image call is a
  judgement about several frames at once, which is why `pickBy` and `pickConf`
  are recorded: without them a red cannot be explained or audited, and the
  sidecar feature's promise that a number never travels without its provenance
  would be broken by the first group.
- **Labels are a first-pass artefact.** The skip rule leaves an already-rated
  frame alone, so a second run over the same folder writes no labels at all. The
  reds are what the first pass thought; only `--force` (which re-pays every
  group) or a hand correction changes them.
- **Stale reds are accepted, and warned about.** A frame that a later grouping
  would promote still wears Red, and a photographer filtering Red will see it.
  The mitigation is a summary line, not a delete, because deleting a label means
  guessing whether we wrote it.
- **The photographer's labels outrank the tool's.** A Yellow on a non-keeper
  keeps its Yellow and stays out of the delete set. That is the cost of never
  overwriting, and it is the right cost.
- **`SPEC.md` §9's "`xmp:Label` is never written" is now false** and is replaced
  by a pointer to this ADR; the xmp-sidecar feature's story 5 and its Out of
  Scope line keep their force for every other case.
- **The ranking rule loses its output columns.** `group_rank`, `group_note` and
  the exclusion clauses stop being a report and become the internal shortlist
  that decides who is compared — if the context cap needs one.
