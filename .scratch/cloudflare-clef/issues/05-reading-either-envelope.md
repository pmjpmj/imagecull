# 05: Reading accepts either answer envelope

Status: ready-for-agent
Type: task
Blocked by: 04, 09
Spec: `.scratch/cloudflare-clef/spec.md`

**What to build:** `reading::read` finds the answer document whether the server
returns it at the top level (oMLX, and Cloudflare per `schema-output.json`) or
wrapped in `{"result": …}` (other Workers AI models, if ticket 09 says Clef does
that too).

## The work

One function at the top of `extract` (or just before it, so `usage` shares it too —
`usage` reads `document.usage`, which moves with the answers):

```rust
/// The document the answers live in: the body itself, or the body's `result`
/// object when a Workers AI-style envelope wraps it.
fn answer_document(document: &Value) -> &Value
```

The rule, stated in a doc comment: **top-level `answers` wins; otherwise a `result`
object that has `answers` is read instead; otherwise the document is what it is and
the existing "no `answers` object" failure fires.** Never both, never a search — one
substitution, so a document that is genuinely drifted still fails loudly.

The raw document stored in `Reading::document` (and written by `--json-scores`) stays
the body as it arrived. The unwrapping is a reading, not a rewrite: a surprising
score must still show the bytes the server sent.

## Tests

`crates/imagecull-core/src/reading.rs` unit tests:

- Top-level answers: unchanged (existing tests).
- Wrapped: `{"result": {"model": ..., "answers": {...}, "usage": {...}}, "success": true}`
  reads to the same `Answers` and the same `Usage`, and `document` still equals the
  wrapper.
- Neither: the current `no `answers` object` failure, message unchanged.
- Wrapped but the `result` has no `answers`: same failure, message unchanged.
- `usage` is read from the same place as the answers: a wrapped document's
  `input_tokens` reaches `Usage::input_tokens`; a wrapper's own `usage` (there is
  none today) is not preferred.

## Done

If ticket 09 shows Clef is unwrapped, this ticket still lands — the rule is one
branch and it is what protects the next Workers AI model — but its "wrapped" tests
are fixture-driven rather than recorded from a live call, and the doc comment says so.
