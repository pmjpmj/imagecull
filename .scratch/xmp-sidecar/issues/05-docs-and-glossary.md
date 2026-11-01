# 05: The vocabulary, the ADRs and the spec

**Type:** task
**Status:** ready-for-agent
**Blocked by:** 03
**Spec:** [../spec.md](../spec.md) §"Documentation the change owes"

## What to build

The written record of what tickets 01–04 did to the tool. No code.

- `GLOSSARY.md`: delete `Report`, `Ledger`, `Resume`; add **Sidecar** and
  **Skip**; amend `Rating` (written to `xmp:Rating` in the sidecar, no longer "in
  v2") and `Dedupe` (within one run). Keep the glossary free of implementation
  detail — no flags, no paths, no property tables.
- **ADR 0006** — "the rating lives in the sidecar". Hard to reverse, surprising,
  a real trade-off: deleting v1's deliverable, writing into the shoot folder,
  deleting the ledger with it. Include the alternatives (`--out` retained, sidecar
  opt-in, sidecar-as-state) and the accepted costs (a deleted sidecar costs one
  request; the skip rule cannot tell our rating from a human's).
- **ADR 0002** amended, not replaced: the dedupe key is unchanged, its memory is
  now in-process, and the promise that dedupe applies to outcomes restored by a
  resume dies with the resume. Add a status line saying so.
- `SPEC.md`: §9 becomes the sidecars section; §9.1 and §9.2 deleted; §12 folded
  into §9 and deleted; §11's flag list, §15's determinism/caching text and §3's or
  §4's mentions of the CSV rewritten. Sweep for every remaining `scores.csv`,
  `ledger`, `--out`, `--absolute`, `--resume`, `--fresh` mention.
- `imagecull.toml.example`: `[outputs]` trimmed to `json_scores`, with a comment
  saying where ratings go now.

## Acceptance

- `grep -rin "ledger\|scores.csv\|--resume\|--fresh\|--absolute\|--out " SPEC.md
  GLOSSARY.md imagecull.toml.example docs/adr/` finds nothing that is not
  deliberate history (an ADR may name a superseded thing; a spec may not).
- Every glossary term still in the glossary is still true of the code as merged.
- ADR 0006 follows the format of the existing ADRs and states its alternatives and
  costs.
- ADR 0002 carries an amendment line naming what changed and why.
- No document promises a feature that is out of scope in the spec.

## Notes

- ADR format: `docs/adr/0001-*` … `0005-*`; pick `0006` and match their shape.
- `AGENTS.md` describes the single-context layout: `GLOSSARY.md` + `docs/adr/` at
  the repo root.
- The spec is the source of truth for what landed; if the code and the spec
  disagree by the time this ticket runs, the code wins and the spec gets fixed —
  say so in the ticket's comment.
