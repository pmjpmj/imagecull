# 02: Best of group in the report

**What to build:** Grouping and measures become part of a `score` run, and the
report gains the answer the photographer asked for. Each frame's ledger record
carries its group, its rank within the group and the measures; the ledger header
carries `grouping_hash` beside `rubric_hash`, so a threshold change invalidates
group ranks as explicitly as a wording change invalidates scores. The CSV gains
`group`, `group_size`, `group_rank`, `is_best` and the measure columns, and gains
one column that explains itself: `group_note` naming the clause that decided the
frame's fate (`impact`, `only_survivor`, `excluded:focus`, `excluded:sharpness`,
`excluded:roll`). The ranking is a declared lexicographic rule over named
measures — never a weighted composite: exclude frames the camera reports out of
focus, exclude frames far below the group's best acutance, exclude frames whose
roll deviates from the group's median when the group has a horizon, then choose
the highest `impact` among survivors. The run summary prints the group count, the
largest group and the rule-disagreement rate — how often the group winner differs
from the frame with the highest `overall` — as signal, not as something to
reconcile. `--no-group` produces exactly what v1 produced.

**Blocked by:** 01

**Status:** needs-info — two inputs must come from issue 01's blind review before
this can be built, and writing them down first would be the invented weighting
SPEC §8.4 refuses:

- which acutance formulation wins (p95 of local block variance, or plain global
  variance normalised by contrast), and
- where `sharp_floor`, `roll_tolerance` and `af_ratio` belong.

## Acceptance

- [ ] Every frame has a group; every group has exactly one `group_rank = 1` winner even when every frame was excluded, and `group_note` says which clause decided
- [ ] `group_note` distinguishes exclusion reasons from selection reasons, and a group of one says so as `only_survivor`
- [ ] The rule is exposed as an ordered list of criteria over named columns; no composite score is computed anywhere
- [ ] `grouping_hash` is in the ledger header; resuming under a changed `grouping_hash` is a hard, actionable error unless `--fresh`
- [ ] The CSV keeps one row per frame with group columns added; no filename disappears and no row is collapsed
- [ ] `--no-group` yields a CSV identical to v1's for the same shoot
- [ ] The summary prints group count, largest group size and the rule-disagreement rate
- [ ] Frames whose camera records no focus confirmation are treated as unknown throughout, never as excluded
- [ ] Measures are never presented as a score out of ten, and no measure changes any of the six answers
- [ ] Determinism: two runs over the same folder produce identical group and measure columns apart from the timings they measured

## Comments
