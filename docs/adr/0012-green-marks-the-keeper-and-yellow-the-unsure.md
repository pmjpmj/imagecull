# 0012 — Green marks the keeper, and a low-confidence cull is yellow

Status: accepted · 2026-11-24 · Feature `burst-grouping-v1` (follow-up) ·
supersedes [ADR 0007](0007-the-red-label-marks-the-non-keepers.md)'s "the keeper
gets no label" and its rejection of Green on the keeper · supersedes
[ADR 0008](0008-the-group-command-culls.md)'s "the keeper still gets no label"
and its "Green on the keeper. Still wrong" · keeps ADR 0008's command split,
rounds, label ownership and repeatable rewrite

## Context

ADR 0007 and ADR 0008 both refused to label the keeper, on the argument that its
rating is its mark and that Green would claim the photographer's field on the
frame they are most likely to label themselves. In the hand that leaves the
grid half-answered: a `groups` run writes Red on every frame the cull would drop
and nothing on the one frame it would keep. A photographer filtering Red sees the
delete set, but the keep set has to be reconstructed from the group, the star
rating and `imagecull:groupKeeper` — three things that are not a colour.

The second half is the confidence floor. ADR 0007 measured near-ties at 0.55
against 0.43 and decided, correctly, that a floor must not silence the cull: a
group below the floor is marked anyway and counted on a summary line. But
"marked anyway" meant Red, the same label a certain loss gets. The one summary
line that says the comparison was a coin flip is not in front of the photographer
when they are filtering in Lightroom, and a filter that treats every Red as
"safe to delete" silently deletes the near-tie cases the model was least sure
about. A distinction the run knows and the file does not is not a distinction the
cull can use.

## Decision

**A group's keeper gets `xmp:Label="Green"`; its non-keepers get Red, or Yellow
when the comparison was below `pick_conf_floor`.**

- **Green is the comparison's answer, made visible.** It says "the frame this
  group kept", not "the best picture of the shoot" and not "a second opinion on
  the rating". The keeper is green in the confident and the low-confidence case
  alike: the pick is still the pick.
- **Yellow replaces Red for a low-confidence group's non-keepers.** It says what
  Red says — not this group's keeper — and adds the one fact the run has and the
  colour otherwise hides: the comparison was a near-tie. Red stays the delete
  set; Yellow is the review set.
- **The threshold is the existing one.** `pick_conf_floor` is the confidence
  below which a group is already counted as low-confidence; the mark and the
  count are driven by the same number, so a report can never say "3 low
  confidence groups" while the folder shows no yellow.
- **The label rules do not change.** A colour is written only where it is absent
  or carries our `labeledBy` proof. A photographer's Yellow, Blue, Green or Red is
  never overwritten, and the frame is counted as unmarked — for a keeper,
  `keeperUnmarked` — because a grid without the colour is explained. A label
  without our proof is never removed.
- **A decided group rewrites, it does not remove.** A frame promoted from
  non-keeper to keeper has its red or yellow replaced with Green in place; a
  keeper demoted to non-keeper has its Green replaced with Red or Yellow. The
  verdict and its provenance are rewritten beside the colour. Removal is still
  only for a frame that is no longer in a group of two or more, and only for a
  mark this tool wrote.
- **Nothing is marked on no answer.** An undecided group writes no colour, as
  before.

## Considered options

- **Keep ADR 0007/0008's "the keeper gets no label".** It avoids the claim on the
  photographer's field, and it is what shipped. It also leaves the grid with no
  keep set, and asks the photographer to read a colour, a rating and a
  proprietary property together to reconstruct one.
- **Yellow on the whole low-confidence group, keeper included.** Honest about the
  uncertainty, but it withholds the answer the comparison did give: a near-tie
  still names one frame, and the photographer needs to know which one. It also
  makes the keeper indistinguishable from the frames the run would drop.
- **Leave low-confidence non-keepers Red and rely on the summary line.** The
  status quo. It puts the uncertainty in a terminal that is gone by the time the
  photographer is filtering, and leaves the delete set claiming a certainty the
  run does not have.
- **A cyan or a second property for the near-tie.** A colour outside the labels
  the photographer already uses is not portable to the grid, and a new
  `imagecull:` property is legible to exiftool but invisible to a Lightroom
  filter — the failure ADR 0007 already rejected for `groupRank`.
- **Write Green only when the group is confident.** It would make the grid say
  "no keeper" about a group that has one, and conflate the pick with the
  certainty of the pick.

## Consequences

- **A red is no longer the only mark this tool writes.** The sidecar grows
  `Green` and `Yellow` as possible `xmp:Label` values, all attributed by the same
  `labeledBy` proof. A photographer's own Green and Yellow remain theirs and stay
  in the unmarked counts.
- **The delete set is narrower than "everything not green".** Red is the delete
  set; Yellow is the review set. A photographer who filters Red excludes the
  near-ties, which is the point; one who wants every non-keeper filters on
  `xmp:Label` being Red or Yellow.
- **The report and the summary carry the new counts.** A group line says `12 red`
  or `12 yellow`; the summary counts red, yellow, green, unmarked non-keepers and
  unmarked keepers, and the low-confidence line says the groups were marked
  yellow. `non_keepers` remains `red + yellow + unmarked`.
- **A low-confidence verdict is no longer visually identical to a sure one.**
  This is the change's whole value and its cost: the photographer can no longer
  treat every Red as the model's certain answer, because the model's uncertain
  answer is a different colour.
- **Green occupies a colour the photographer may use.** The rule that a foreign
  colour is never overwritten means a shoot already using Green keeps it, and the
  keeper is counted `keeperUnmarked` instead. This is the same trade ADR 0007
  made for Red, applied to the other end of the cull.
