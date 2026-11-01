# 0001 — The rubric is authored once and rendered twice

Status: accepted · 2026-11-08 · Ticket 03 (rubric and wire adapter) ·
amended by [ADR 0002](0002-dedupe-keys-the-state-but-not-the-filename.md) and
[ADR 0004](0004-built-in-rubric-family.md) · superseded in part by
[ADR 0005](0005-one-rating-replaces-six-questions.md), which removes the `score`
half: a cull now asks one `choice` question, so only the `choice` rendering is
reached in practice. The wire contract still carries both types, and the argument
below is why the authored form stays order-preserving.

## Context

SPEC §8.3 says a question's `criteria` is an **ordered array of
`{key, description}` pairs**, "because TOML tables do not guarantee key order
and the option order matters to the model's legend". SPEC §7.1 sketches the
request as `"questions": { "…§8…" }` and leaves the concrete shape to the
rubric.

The upstream implementation of `POST /v1/systemone` (`jundot/omlx#4315`,
`omlx/api/systemone_models.py` and `omlx/models/clef.py`) resolves it
differently per question type:

- `score` takes `criteria` as an **ordered list of level descriptions**; the
  index is the key, and the answer's `score` is the expected value over those
  indices, with the legend returned alongside.
- `choice` takes `criteria` as an **object mapping option → description** (or
  `null`). A `choice` question sent with a list is rejected by the model
  backend with a 400.

So the authored form the spec asks for is neither of the two wire forms: it is
the order-preserving superset of both.

## Decision

Keep **one authored form** — the ordered `{key, description}` arrays of TOML —
and render it per question type in exactly one place,
`Rubric::questions_json`:

- a `score` question renders as a list of descriptions, in key order;
- a `choice` question renders as a JSON object keyed by option, in declaration
  order (`serde_json`'s `preserve_order` keeps that order on the wire);
- the request's `questions` is a JSON object keyed by question id, in
  declaration order.

The rubric hash (SPEC §8.3) is SHA-256 over **that rendered form** plus the
model alias, not over the file and not over a parallel canonicalisation. A
change to an instruction, a criterion, an option's order, or to how the schema
is expressed to the model therefore moves the hash. Two runs with the same hash
sent the model byte-identical question schemas.

Validation fixes the relationship in both directions: a score question must
offer exactly `"0"`…`"10"` in order (otherwise the index is no longer the key),
and a choice question must offer exactly the keys the output column parses. Since
ADR 0005 the only choice a cull asks is `stars`, whose keys are `1`–`5`.

## Consequences

- A photographer edits one file; the wire shape is the adapter's problem, and the
  same rubric could be re-rendered for another backend without a rewrite of the
  text.
- Changing the rendering *is* a rubric change and invalidates cached scores by
  design. That is the desired direction of failure: a silent reuse of scores
  produced from a different legend would be worse than a re-run.
- The raw TOML is not the hash subject, so reformatting, comments, quoting style
  and table-vs-inline layout do not invalidate a ledger.
- `probabilities`/`legend` in an answer stay display-only; `score`, `choice` and
  `confidence` are the only recorded values (spec §7.2).
- **The hash does not cover the request state.** `filename` and `shoot_context`
  are part of what the model reads, so an outcome is only reusable when the
  state matches too: the dedupe and resume keys of ticket 05 must include the
  state, not just `(preview_sha256, rubric_hash, model)`. The same frame judged
  as "dim, high-ISO street shoot" is a different question from the same frame
  judged bare. — *Superseded in part by
  [ADR 0002](0002-dedupe-keys-the-state-but-not-the-filename.md):* ticket 05's
  dedupe key covers the bytes, the task framing, the shoot context and the
  rubric identity, but not the filename, because two frames of one burst always
  have different names and a key that covered it could never fire.
