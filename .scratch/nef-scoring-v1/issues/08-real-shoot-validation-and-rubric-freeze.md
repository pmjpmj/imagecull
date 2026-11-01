# 08: Real-shoot validation and rubric freeze

**What to build:** Evidence that the numbers are worth trusting, and a rubric frozen on the back of it. Run a real shoot end to end against the checkpoint the photographer actually has installed, then compare the scores and star ratings against the photographer's own judgement rather than against a threshold. Where the four scores collapse into each other or saturate into a narrow band, the fix is the question wording — the instructions that isolate the three axes and the photojournalistic framing of `overall` — not a rescaling or a fudge factor. Iterate the rubric until the photographer agrees with it, then freeze it as the embedded default and record its hash, along with which checkpoint the freeze was validated against. This is wording and judgement work, not engineering; the deliverable is a decision and the evidence behind it.

**Blocked by:** 07

**Status:** ready-for-human

- [ ] A real shoot has been scored end to end with the installed Clef checkpoint, producing a CSV with a status on every row
- [ ] Scores and star ratings have been reviewed against the photographer's own judgement frame by frame for a sample of the shoot
- [ ] Saturation has been checked on the star histogram and the per-score spread; any collapse between axes is attributed to specific rubric wording
- [ ] The rubric wording has been iterated and the resulting hash recorded for each iteration
- [ ] A frozen embedded default rubric is committed, with the checkpoint alias it was validated against
- [ ] The disagreement rate between `stars` and `overall` is recorded as the accepted baseline for future rubric changes
- [ ] Any leftover open question (per-genre rubrics, alternate checkpoint) is written down rather than left implicit
