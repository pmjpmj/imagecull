# 0015 — The comparison judges the frame whole

Status: accepted · 2026-11-25 · amends
[ADR 0008](0008-the-group-command-culls.md)'s comparison wording (the mechanism
is unchanged: one relative question, one keeper, the label rules as 0008 and 0012
left them) · supersedes `keeper::INSTRUCTIONS`'s ranked-gates decision and its
"facing the camera" gate

## Context

`score` and `groups` answer two questions about one folder — *which frames are
worth keeping* and *which frame of this moment is the one to keep* — and the
wording made them disagree. `score` sends a rubric; `general`, the default, judges
the frame whole, assumes no genre, and says out loud that "a distinctive moment,
gesture, expression or quality of light counts for more than clinical
perfection, and a frame that is slightly soft but holds something beats a sharp
frame in which nothing happens". The comparison sent the opposite rule.

The comparison's instruction was a ranked list of five gates whose first rule
decided alone: "no frame that fails this is the one to keep, however good its
light, moment, gaze or composition, so a sharp frame beats a softer frame with the
better expression", with a fallback of "keep the sharpest and stillest" for a
burst that was soft throughout. Rule 3 preferred the subject facing the camera,
which `keeper.rs` itself documented as "a shoot-type preference rather than a
universal one" and named as the gate a per-shoot target setting would one day
undo. The trade was recorded honestly at the time, and it was still the wrong
trade: run a burst folder through both commands and the comparison marks the
sharp, dull frame red while the rating calls the soft one with the expression the
best of the moment. Two judgements, one folder, opposite answers.

The wording's defence was measurement, and the measurement does not cover it. The
evidence (`.scratch/burst-grouping-v1/evidence/comparison-budget.md`) measured a
shorter, earlier sentence — "the sharpest and the one whose subject, expression
and light hold together best" — and measured it for *position bias*, at 0.55
against 0.43 between orderings, not against a photographer's own picks. The
five-gate text was already labelled a candidate in `keeper.rs`: "this wording is
a candidate until the same three frames are re-run at every edge and in every
ordering". Nothing since promoted it.

There is also a structural reason the ranked list could not have held. A
comparison sees frames of one moment, and those frames are not always one
intent: a turn, a zoom or a re-frame inside a burst changes what the frame is
aiming at. A gate list that begins "sharp and still, above all else" answers a
question about technical delivery; the cull is asked for a keeper.

## Decision

**The comparison asks the general rubric's stance, relativised to the group.** The
instruction now reads each frame for what it is trying to be, holds it to that and
not to a genre it never aimed at, judges it on three things in the order that
decides them — is there something worth looking at, is it caught, is the frame
made well enough to carry it — and states the trade the general rubric states: a
slightly soft frame that holds something beats a sharp frame in which nothing
happens.

**The stance keeps a floor.** "A frame that cannot carry the picture — the subject
smeared by motion, the focus on the background, the highlights blown to white — is
not the one to keep, however good its moment." Without that sentence the change
would not be a trade but a surrender: the mark would send a photographer to a
frame they cannot deliver, and the red means "not this group's keeper", which
still has to be actionable.

**The ranked gates and the facing-the-camera preference retire.** Sharpness is no
longer a rule that decides on its own, the five-gate ordering and its "rules 2 to
5 only separate frames that are already sharp" framing are gone, and direct
address is now one cue a frame may win on rather than a gate ranked above light
and composition. A comparison that prefers the subject looking at the lens is a
press target; `general` names no target, and `groups` selects no rubric to name
one with.

**The measured position rule survives the rewrite.** "Do not prefer a frame
because it is first, brighter or warmer" stays: it is the one clause in the old
text whose evidence is about this question rather than about a different wording.

**Nothing else about the comparison moves.** One relative `choice` question keyed
by position, every frame of the group shown, rounds above twenty, `FLOOR` at
0.60, preview at `[group] max_edge`, the verdict and label rules of ADR 0008 and
ADR 0012, and the sidecar's provenance all stay as they are. This ADR changes the
words the model is asked and nothing about the machinery around them.

## Considered options

- **Keep the ranked gates and sharpen the trade-off in the doc comment.** The
  current state. It is defensible as a *deliverable* cull and it is the wrong
  question: the photographer's own keepers are the ones the gate list discards,
  and the tool's two commands then disagree about the same folder in writing.
- **Keep the gates, move the moment to rule 1.** Fixes the ranking and keeps a
  mechanical list, which reads as more deterministic than a stance. But the list
  is what made the wording prescriptive: once the order is written down the
  comparison stops reading the frame and starts scoring it against five axes, and
  the general stance — "judge it as what it is trying to be" — cannot be expressed
  as an ordered list without adding a genre back.
- **Send the selected rubric's own `stars` instruction beside the question.** The
  most literal reading of "use the score wording here", and it would guarantee the
  two commands never diverge. Rejected for now: `groups` selects no rubric (ADR
  0008 defaults the alias, not a rubric), so there is nothing to concatenate; a
  rating instruction ends by asking for a number and a comparison must ask for a
  key; and it would make the cull change whenever a rubric does, which is a
  coupling ADR 0008 spent its length removing. If SPEC §11's aesthetic target
  ever becomes a per-shoot setting, this is the seam to revisit — the honest
  version passes a target, not a rating instruction.
- **Keep facing-the-camera as a tiebreak.** Cheap, and it keeps press work
  familiar. It still encodes one genre's preference in the default cull, and a
  tiebreak is where the model has no other signal to weigh it against — the
  clause would decide more often than "tiebreak" suggests.
- **Rewrite the wording and re-measure first.** Correct order for a change with
  evidence behind the old text; there is none behind the gates, and the
  disagreement is reproducible on ordinary shoots. The re-measurement is carried
  as the consequence below instead of as a gate on the change.

## Consequences

- **Verdicts will move, and on soft frames they will move a lot.** A group whose
  sharpest frame has the duller expression can now keep the softer one, which is
  the point and is also the loudest thing a photographer will notice on the first
  re-run. `--print-sidecars` shows the flip before it is written; `--no-cull`
  remains the pass that writes nothing.
- **`groups` and `score` now agree about which frames are worth keeping**, which
  is the property that made this worth doing: a folder rated 4 for a soft frame
  that holds the moment no longer has that frame marked red by the cull of its own
  group.
- **The cull no longer prefers direct address.** A press shoot that wants the
  subject looking at the lens loses that preference. It was already documented as
  the bias to undo first, and a per-shoot target is the fix — this ADR does not
  add one.
- **`keeper::INSTRUCTIONS` is unmeasured text again, and says so.** The evidence
  file measured an earlier sentence; the next measurement must record the swap
  rate between orderings and the count of groups under `FLOOR` for *this* wording
  at every preview edge, before any run is described as sending a measured
  question. If a real shoot shows the comparison now keeping frames the
  photographer threw away for focus, the floor sentence is where to look; if it
  shows the old behaviour — sharp and dull over soft and alive — the re-measured
  swap rate is the thing that will say whether the model is reading the stance at
  all.
- **The `read`'s position check and the round machinery are untouched**, so the
  existing tests for chunking, `shakiest`, and an out-of-range answer still
  describe the code. Only the wording assertions in `tests/keeper.rs` move.
- **`tests/keeper.rs` now asserts the absences as well as the presences.** A test
  that only checked the new clauses would let the ranked list grow back beside
  them; the retired phrases are asserted missing, with the reason in the failure
  message.
