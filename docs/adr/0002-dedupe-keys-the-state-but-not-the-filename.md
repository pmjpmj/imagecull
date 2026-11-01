# 0002 — The dedupe key covers the state, but not the filename

Status: accepted · 2026-11-08 · Ticket 05 (ledger, resume, burst dedupe) ·
amends ADR 0001

## Context

ADR 0001 closes with a consequence: "The hash does not cover the request state",
so "the dedupe and resume keys of ticket 05 must include the state, not just
`(preview_sha256, rubric_hash, model)`" — and the state, per the wire contract,
is `{task, filename, shoot_context}`.

Taken literally, that makes burst dedupe unreachable. Two frames of one burst
are two files, so they always have different names; a key that covers the
filename therefore never matches, and spec §9.3's "an identical preview reuses
the earlier outcome" — the reason a 400-frame burst-heavy shoot gets faster —
could never fire. The resume key is a separate matter: it is
`(file, size, mtime_ns)` by definition (spec §9.3), because there the file *is*
the subject being identified.

The question ADR 0001 was really asking is: **is the filename part of what makes
this question the same question?** The rubric hash already answers the same kind
of question about the question schema, and it excludes the file for the same
reason.

## Decision

The **dedupe key** is SHA-256 over, in order and length-prefixed:

1. the hash of the bytes that would actually be sent (the oriented, downscaled,
   re-encoded JPEG — not the NEF);
2. the task framing (`photo_cull`);
3. the shoot context, or an explicit marker for "none";
4. the full rubric hash (schema plus model alias);
5. the model alias.

The **filename is deliberately not in it.** It is the photographer's label for
the frame, not something the picture shows: a model looking at two identical
previews is being asked the same question, whatever the files are called. Every
other part of the request state — the bytes, the framing, the context, and the
rubric identity — is in the key, so an outcome is only reused when the question
was the same.

The key is written into every ledger record (`dedupe_key`), so dedupe also
applies to outcomes a resumed run restores rather than only within one process.
A record whose status is not `ok` is never reused: a timeout is not an answer,
so a burst partner gets its own attempt.

## Consequences

- Dedupe fires for bursts, which is the point of it.
- **Cost:** a model that keyed on the filename would have answered differently,
  and this key cannot see that. The cost is made inspectable rather than
  hidden: every reused row carries `duplicate_of` naming the frame whose request
  paid, so a photographer who suspects the names mattered can re-run the same
  folder with `--no-dedupe` and diff the two CSVs.
- The state is in the key, so the same frame judged under a different
  `--shoot-context` is a different question and is not reused. Within one run
  the context is fixed; across a resume, the ledger's records were written under
  the context the resume started with.
- The resume key stays `(file, size, mtime_ns)` and the two must not be
  confused: resume asks "have I done this file?", dedupe asks "have I answered
  this question?".
- ADR 0001's final paragraph is superseded to that extent: it is the *state the
  model reads* that a reuse key must cover, and the filename is the one part of
  that state which is a label rather than content.
