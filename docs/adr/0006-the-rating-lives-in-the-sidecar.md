# 0006 — The rating lives in the sidecar, not in a report

Status: accepted · 2026-11-19 · Feature `xmp-sidecar` ·
retires the report and ledger of [ADR 0002](0002-dedupe-keys-the-state-but-not-the-filename)'s
resume machinery · builds SPEC §12, which was specced as v2

## Context

A run's deliverable was `scores.csv` in the working directory, plus a
`scores.csv.ledger.jsonl` beside it. Neither is where a cull happens. The
photographer finished a 400-frame run with a spreadsheet, a second monitor and 400
numbers to retype into Lightroom one frame at a time — or culled blind in
Lightroom while the ratings sat in a file Lightroom would never read. The one
number the model produced, the whole point of the run, was stored in the one
format the cull could not see.

Three things made that worse than merely awkward:

- **The tool had no opinion about ratings already in the catalogue.** A frame the
  photographer had rated 5 in Lightroom was scored again from scratch, and an
  export would have overwritten that 5 with the model's guess.
- **The ledger existed to serve the CSV.** `--resume`, `--fresh`, the rubric
  mismatch refusal, the mid-record repair and 900 lines of tests were all
  machinery for a file that was a rendering of the ledger.
- **A number travelled without its provenance.** Someone reading `scores.csv` in
  year two could not tell which rubric made a 4 without finding a terminal log or
  the ledger header.

The v1 decision to prove the numbers on a real shoot before writing into the
photo folder had been paid for: one shoot, 400 frames, the legend reviewed.

## Decision

**The run writes `xmp:Rating` into an XMP sidecar beside each frame** —
`DSC_0001.NEF` → `DSC_0001.xmp`, never `DSC_0001.NEF.xmp`, which belongs to
darktable. The sidecar also carries the rating's provenance in a private
namespace: confidence, model alias, rubric hash, when, and which binary. There is
no report file. `--json-scores` remains as the debugging hatch.

**The run leaves a frame alone when its sidecar already carries a rating of 1–5.**
That one rule does two jobs: it protects a human's rating from being overwritten,
and it makes a re-run cheap — a run interrupted at frame 380 of 400 re-asks 20
frames. `--force` exists for the case the rule cannot know about: rate them again.

**Nothing else in the file is rewritten.** The merge is a targeted text edit, not
a parse and re-serialise: foreign `crs:` settings, keywords, GPS and another
application's history survive byte for byte. No XML dependency is added, because
byte-preservation is the requirement and a parser cannot give it.

**The ledger, `--resume`, `--fresh` and the CSV go.** The sidecar is the state,
and the skip rule reads it. Dedupe keeps its key and loses its cross-process
memory: it is now an in-process cache that dies with the run.

**The terminal keeps its shape.** A skipped frame says `5★ kept` where a rating
would be; the headline counts only what this run rated; everything else reads as
it did. `--print-sidecars` is the review step: it scores, prints what each
sidecar would become, and writes nothing.

**The dangerous cases are named rather than guessed at.** `xmp_unreadable` (no
description to write into), `xmp_foreign` (a darktable sidecar, unless
`--allow-foreign`), `xmp_write_failed` (the write failed, which stops the run
because every remaining frame would fail the same way and each attempt has already
been paid for).

## Considered options

- **Keep `--out` as a second, optional report.** Two artifacts that can disagree,
  and a photographer who runs the tool twice gets two answers ordered only by
  which file they open first. The report's remaining value — a spreadsheet view —
  is a script over the sidecars, not a second write path.
- **Make the sidecar opt-in (`--xmp`), as SPEC §12 proposed.** Then the default
  remains a file the cull cannot read, and the flag is the difference between the
  tool being useful and being decorative.
- **Make the sidecar itself the resume state, beyond the skip rule.** After this
  change the sidecar carries everything a resume would need, so `--resume` could
  read ratings back instead of re-asking. That is a different idea from "the
  rating lives in the sidecar" — it makes the filesystem authoritative over money
  already spent — and it deserves its own decision. The skip rule already
  delivers most of the benefit for nothing.
- **Keep the ledger for the rubric-mismatch refusal.** With no ledger there is
  nothing to mismatch: a re-run under a new rubric skips everything, which is why
  `--force` exists and why the sidecar records the rubric hash that made each
  rating. The refusal becomes an explanation in the summary instead of a fatal
  error.
- **Parse and re-serialise the sidecar with an XML library.** Correct XML,
  reordered attributes, re-quoted values, and a diff in another application's file
  on every write. The requirement is the opposite: change only what we own.

## Consequences

- **The upgrade path is a deletion.** `scores.csv` is dead weight, and a stale
  `[outputs] csv` in a config file is a fatal error naming the change rather than
  a silent ignore. Nothing is imported.
- **A deleted sidecar costs one request.** Losing the file loses the rating; there
  is no second record to be rebuilt from.
- **A run killed mid-shoot is cheap to finish**, because the frames it finished
  have ratings already, but it is not free to *re-judge* a folder: `--force` pays
  for every frame again.
- **The skip rule cannot tell our rating from a human's.** That is the point — it
  protects both — and it means a re-score under a different rubric is a no-op
  unless `--force` is typed. The recorded rubric hash is how a skip can be
  explained.
- **Dedupe no longer applies to outcomes a run did not produce.** ADR 0002's
  promise about resumed outcomes dies with the resume; its key is unchanged.
- **The run writes into the shoot folder.** The raws are still opened read-only
  and never modified, but the tool's blast radius is now the folder it scores,
  which is why the preview exists and why `.bak` files are automatic for sidecars
  that were already there.
- **`SPEC.md` §9.2, §12 and §15 change shape**, and `GLOSSARY.md` loses `Report`,
  `Ledger` and `Resume`.
