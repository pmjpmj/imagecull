# 01: The sidecar document, as one pure function

**Type:** task
**Status:** ready-for-agent
**Blocked by:** none
**Spec:** [../spec.md](../spec.md) §"The sidecar document", §"Testing Decisions" seam 2

## What to build

The `sidecar` module in `imagecull-core`, holding only the pure document update:
an existing document (or none) plus one frame's properties in, the text to write
out, or a classified refusal. No filesystem, no `run`, no ports — this ticket is
the document, and the rest of the world is ticket 03's job.

Properties, namespace, filename rule, merge algorithm, escaping and the refusal
cases are all fixed by the spec's "The sidecar document" section; implement it
verbatim. No XML dependency is added: the merge is a targeted text edit that
leaves every character it does not own byte-identical.

The refusal cases are values the run will classify: a document with no locatable
first `rdf:Description` opening tag. Give them an error the taxonomy can carry
(existing `Error`/`ErrorKind` machinery, new kinds are ticket 03's to name in the
taxonomy if it wants them).

## Acceptance

- A new frame yields a minimal `x:xmpmeta`/`rdf:RDF`/`rdf:Description` document
  carrying `xmp:Rating` and the five `imagecull:` properties, nothing else.
- Merging into a document with foreign `crs:`, keywords, a `darktable:`
  namespace, mixed quote styles and unusual attribute order changes only our own
  properties: every other byte is identical (assert on the whole text).
- An existing `xmp:Rating` is replaced in place, and repeated updates are
  idempotent: merging twice yields the same document as once.
- `--force`-style rewrite replaces our `imagecull:` properties and leaves the rest
  untouched.
- `imagecull:rubricHash` is the 8 hex characters the terminal prints;
  `imagecull:confStars` is the model's own number, unrounded; `scoredAt` is
  RFC 3339 UTC with `Z`.
- A document with no locatable `rdf:Description` opening tag is refused, and the
  refusal names why.
- Values are XML-escaped; nothing else is escaped or reformatted.
- No new dependency in `Cargo.toml`.

## Notes

- `imagecull-core/src/report.rs` is the module that becomes `sidecar` (ticket 03
  moves `Row` and deletes the CSV writer; do not do that here). This ticket adds
  `sidecar.rs` with the document half only, and `lib.rs` exports it.
- Domain vocabulary: **Frame**, **Sidecar**, **Rating**, **Rubric hash**. See
  `GLOSSARY.md`.
- Spelling of the short rubric hash: `RubricHash::short()` in `rubric.rs` already
  does this — reuse it rather than slicing hex.
