# 08: Real-shoot validation and rubric freeze

**What to build:** Evidence that the numbers are worth trusting, and a rubric frozen on the back of it. Run a real shoot end to end against the checkpoint the photographer actually has installed, then compare the scores and star ratings against the photographer's own judgement rather than against a threshold. Where the four scores collapse into each other or saturate into a narrow band, the fix is the question wording — the instructions that isolate the three axes and the photojournalistic framing of `overall` — not a rescaling or a fudge factor. Iterate the rubric until the photographer agrees with it, then freeze it as the embedded default and record its hash, along with which checkpoint the freeze was validated against. This is wording and judgement work, not engineering; the deliverable is a decision and the evidence behind it.

**Blocked by:** 07

**Status:** ready-for-human

- [x] A real shoot has been scored end to end with the installed Clef checkpoint, producing a CSV with a status on every row
- [ ] Scores and star ratings have been reviewed against the photographer's own judgement frame by frame for a sample of the shoot
- [x] Saturation has been checked on the star histogram and the per-score spread; any collapse between axes is attributed to specific rubric wording
- [x] The rubric wording has been iterated and the resulting hash recorded for each iteration
- [ ] A frozen embedded default rubric is committed, with the checkpoint alias it was validated against
- [ ] The disagreement rate between `stars` and `overall` is recorded as the accepted baseline for future rubric changes
- [x] Any leftover open question (per-genre rubrics, alternate checkpoint) is written down rather than left implicit

## Evidence (2026-11-01)

Gathered on `feat/nef-scoring-v1` against the oMLX on this machine; the detail
lives in `.scratch/nef-scoring-v1/evidence/evidence.md`, with the run's CSV and a
per-frame review sheet beside it.

- **Scored end to end:** `~/Downloads/import-251221` (11 NEFs, 10 JPEG sidecars
  skipped), `clef-flash-oQ4e`, embedded rubric `69139540`, eleven rows all
  `ok`, 2m44s, zero writes into the shoot folder, previews upright for both
  portrait frames.
- **Saturation and collapse, measured:** `overall` mean 5.67 sd 0.67, `technical`
  5.31 sd 0.41, `composition` 5.64 sd 0.49, `impact` 5.80 sd 0.86; stars 10×3★
  and 1×4★ with none below 3 or at 5; `overall`~`composition` r=0.94,
  `overall`~`impact` r=0.97, `composition`~`impact` r=0.94, `overall`~`technical`
  r=0.74; `overall` confidence mean 0.23.
- **Wording iteration recorded:** variant B (`5773b2c1`, per-axis legends plus
  "use the whole range") separated `technical` cleanly (`technical`~`overall`
  r 0.74→0.21, `technical`~`impact` 0.62→0.01) but left `composition`~`impact`
  at 0.93, the band just as narrow, the stars just as saturated, and every
  `overall` about a third of a point higher. Wording moves an axis apart and
  shifts the level; it did not widen the distribution on this shoot.
- **`stars`/`overall` disagreement: 0 of 11** under the recorded rule (two or
  more stars away from `round(overall / 2)`), with the caveat that the rule
  cannot fire while every frame is 3★ — so 0% is a consequence of saturation,
  not evidence of agreement, and is *not* offered as the accepted baseline.
- **Open questions written down:** whether a family snapshot set is the right
  test for a photojournalistic rubric; whether `composition` and `impact` are
  separable at all on this material; whether the star vocabulary, not the
  rubric, is what keeps everything at 3★; `clef-oQ4e` (27B) untested; `.dng`
  accepted but not decodable in v1; one frozen rubric versus a per-genre family.

Still open, and deliberately the photographer's: the frame-by-frame judgement
against their own rating (`review-sheet.md` has the blank column), the freeze of
a default rubric with the checkpoint it was validated against, and accepting the
disagreement baseline once the stars actually spread.
