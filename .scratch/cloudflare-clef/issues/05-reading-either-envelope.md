# 05: Reading accepts either answer envelope

Status: resolved (2026-11-02)
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

## Comments

**Implemented 2026-11-02, fixture-driven: ticket 09 is still `ready-for-human`, so
`/cloudflare-clef` proceeds without a recorded live body.** `reading::answer_document`
landed with the rule as stated, and `read` plus `usage` both read through it. Four
notes:

1. **The tests live at the public seam, not in a `#[cfg(test)]` module.** The ticket
   says "`crates/imagecull-core/src/reading.rs` unit tests", but every other reading
   test is an integration test on `reading::read` in
   `crates/imagecull-core/tests/reading.rs`, and the behaviour under test is entirely
   observable there. Three tests were added beside the existing ones:
   `a_wrapped_answer_document_reads_the_same_as_a_bare_one`,
   `a_wrapper_whose_result_has_no_answers_fails_like_no_answers_at_all` and
   `the_wrappers_own_usage_is_not_preferred_over_the_answers_own`.
2. **"Top-level `answers` wins" means the key's presence, not its shape.** A body
   whose top-level `answers` is an array is not rescued by a well-formed `result`
   below it: it takes the existing `no `answers` object` failure, so the existing
   wrong-shape test keeps its meaning and a drifted document is never read by luck.
3. **`usage` calls `answer_document` itself** rather than taking a pre-unwrapped
   document, so the two call sites in `read` cannot disagree about which object the
   answer came from, and a wrapper's own `usage` is structurally unreachable.
4. **`Reading::document` is untouched.** The raw body still reaches `--json-scores`
   verbatim, wrapper and all; the unwrapping is a borrow, not a rewrite.
5. **`answer_document` is `pub(crate)`, and `keeper::read` reads through it.** Code
   review found the gap both axes independently: `keeper` read
   `response.document.get("answers")` on the raw body, so a wrapped Workers AI
   answer would rate a frame and fail every comparison as
   `the comparative answer has no `keeper` answer` — the normal path for a capped
   server, per the spec's Decision 6. `keeper::read`'s own doc already claimed it
   read "at the strictness [`reading`] applies to a rating", which was true until
   this ticket and is true again. Its failure message is unchanged.
