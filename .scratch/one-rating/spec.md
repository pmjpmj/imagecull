# Feature: one rating

**Status:** landed · **Slice:** the rubric's six questions become one

## Problem

A rubric asked six questions: four 0–10 sub-scores (`overall`, `technical`,
`composition`, `impact`), a five-way `stars` choice and a three-way `verdict`.
The sub-scores were meant to keep different failure modes diagnosable, but the
one real shoot showed them moving together and the run's only actionable output
being the star column. Meanwhile the cull — the thing the tool is for — has a
five-word vocabulary a photographer already uses: delete, not great, okay to
keep, good, great.

## Decision

One question, answered 1–5. The rubric asks `stars` as a `choice` on keys
`"1"`–`"5"`, with the anchors fixed by the loader and the gloss written per
built-in. `overall`, `technical`, `composition`, `impact` and `verdict` are
removed; confidence is still recorded per answer. The three built-ins remain,
each a wording of the same one question.

The reasoning, alternatives and consequences are recorded in
[ADR 0005](../../docs/adr/0005-one-rating-replaces-six-questions.md).

## What changed

- `crates/imagecull-core/rubrics/{general,press,holiday}.toml`: one question
  each, five levels, the words themselves.
- `rubric.rs`: one required id, one required shape, no question-type branching;
  the schema tag in the hash becomes `imagecull-rubric-v2`.
- `reading.rs`: `Answers { stars: u8, confidence: f64 }`.
- `report.rs`: ten columns instead of seventeen.
- `progress.rs` / CLI: the frame line shows the rating; the summary shows its
  mean and median plus the histogram. The `stars`/`overall` disagreement rule is
  retired with the second number.
- Spec, glossary and the burst-grouping slice's ranking clause follow.

## Acceptance

- All three built-ins parse, offer `1`–`5` in order, and hash differently.
- A rubric file that asks anything else fails at load with the one id it wants.
- One NEF run writes `file,stars,conf_stars,…`; a failed row leaves the value
  cells empty.
- Two runs over one folder agree on every column except `latency_ms`.
