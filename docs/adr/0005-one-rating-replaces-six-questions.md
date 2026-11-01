# 0005 — One rating replaces the six questions

Status: accepted · 2026-11-25 · Feature `one-rating` ·
amends [ADR 0001](0001-rubric-renders-two-criteria-shapes.md) ·
amends [ADR 0004](0004-built-in-rubric-family.md)

## Context

The rubric shipped six questions in one forward pass: four 0–10 `score`s
(`overall`, `technical`, `composition`, `impact`) and two `choice`s (`stars`,
`verdict`). The premise was that diagnosis beats one number — a blurry decisive
frame and a sterile sharp one are different decisions, and a blended number
erases the difference (SPEC §8.1, pre-0005).

Two things broke that premise in use. First, the evidence from the one
end-to-end shoot: `overall` had a spread of 0.67 sd and a mean top-option
confidence of 0.23 against a 0.09 chance floor. The axes were correlated in the
way that matters — when `technical` and `impact` disagreed, both readings were
usually wrong in the same direction — so no decision rule could use the
disagreement. Second, the deliverable is a cull: a folder sorted by which frames
to keep. The star column was already that sort. The four scores were a diagnosis
nobody had asked for, carried in three extra CSV columns, two confidence
columns, and a disagreement metric that existed to reconcile two answers to the
same question.

A cull also has a vocabulary. Photographers already say *delete*, *not great*,
*okay*, *good*, *great*, and every catalogue understands a 1–5 rating
(`xmp:Rating`). Six questions asked the model to invent a second language for a
decision the first one already states.

## Decision

**A rubric asks exactly one question.** It is the `choice` question `stars`,
offered on a fixed five-point legend:

| key | meaning |
|---|---|
| `1` | delete |
| `2` | not great |
| `3` | okay to keep |
| `4` | good |
| `5` | great |

The anchor word of each level is fixed by the loader, because a report's reader
sorts by it; the gloss after the dash is the rubric's to write, per built-in and
per genre. The question is judged whole — subject, craft and moment in one call —
and the answer's `confidence` is recorded beside it.

`overall`, `technical`, `composition`, `impact` and `verdict` are gone. A rubric
that asks anything else, or offers any other keys, is rejected at load with a
fatal error, exactly as a malformed six-question rubric was.

**The id stays `stars`.** It is the CSV column, the ledger key and the v2
sidecar's mapping to `xmp:Rating`, and none of those names changed meaning: the
model's own 5-way choice is now the whole judgement rather than one of six.

**The three built-ins stay.** `general`, `press` and `holiday` are now three
wordings of the same one question, with the same keys and their own instructions
and level glosses. Their hashes still differ, so reports stay incomparable across
genera.

**The rubric hash is re-tagged `imagecull-rubric-v2`.** The schema shape
changed; no digest from the six-question era may collide with one from this one.

**The summary reports the rating.** Mean and median of the answer, plus the
1–5 histogram. `DISAGREEMENT_RULE`, `implied_stars` and `agrees` are deleted:
with one answer there is nothing to disagree with, and the metric that compared
`stars` to `overall` compared an answer to a probe of the same judgement.

## Considered options

- **Keep `verdict` as a second question.** A 4 or 5 and `like` are the same
  statement, and the run already reported their disagreement rather than
  reconciling it. Keeping both would keep two ways to say one thing and a metric
  to compare them.
- **Keep the 0–10 scale and collapse it into a rating afterwards.** That is a
  threshold in disguise: a rating derived from a score is exactly the post-hoc
  mapping G4 rules out, and it would be re-deriving the same ten levels the
  model was asked to answer.
- **Use a five-level `score` question instead of a `choice`.** The recorded
  value would be an expected value in `[1, 5]` — `3.42` rather than `3`. It is
  more information about hesitation, but the photographer asked for a decision,
  and confidence already carries the hesitation honestly.
- **Keep the sub-scores for debugging.** The raw answer document is still
  preserved in the ledger and written by `--json-scores`, so a future diagnosis
  is a rubric wording exercise, not a schema that six columns have to survive.

## Consequences

- Every CSV, ledger and rubric hash produced before this change is
  incomparable with one produced after it. An old ledger needs `--fresh`; the
  rubric-hash mismatch already says so.
- The burst-grouping ranking rule's final clause now names the rating among
  survivors where it named `impact` (see `.scratch/burst-grouping-v1/spec.md`).
- The v2 sidecar writes `xmp:Rating` from the answer and no longer derives
  `xmp:Label`/`xmpDM:pick` from a verdict. Pick and reject are the
  photographer's, not the model's.
- SPEC §8, §9.1 and §12.1 are rewritten around the one question; the glossary
  gains *rating* and loses the vocabulary of sub-scores.
- A rubric file a photographer wrote for the six-question schema fails loudly at
  load and lists the one id it now expects. That is the intended failure: a
  reworded rubric must re-score, never silently re-interpret.
