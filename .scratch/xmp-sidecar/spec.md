# Spec — the rating lives in the sidecar

**Status:** landed (2026-11-19) — five tickets, all resolved, plus review fixes
**Date:** 2026-11-19
**Source:** `SPEC.md` §9, §12, §15; `GLOSSARY.md`; the grilling session of 2026-11-19. Builds the
sidecars `SPEC.md` §12 specced as v2, and retires the ledger §9.2 specced by deleting it
(ADR 0006 new, ADR 0002 amended).

---

## Problem Statement

A run's deliverable today is `scores.csv` in the working directory, plus a
`scores.csv.ledger.jsonl` beside it. Neither is what the cull is done in. The
photographer finishes a 400-frame run with a spreadsheet, a second monitor and
400 numbers that have to be retyped into Lightroom one frame at a time — or culled
blind in Lightroom while the ratings sit in a file Lightroom will never read. The
one number the model produced is the only thing anyone needs from the run, and it
is stored in the one format the cull cannot see.

Three properties of the current design make this worse rather than merely awkward:

- **The tool has no opinion about ratings already in the catalogue.** A frame the
  photographer already rated 5 in Lightroom is scored again from scratch and, on
  export, would have that 5 overwritten by the model's guess.
- **The ledger exists to serve the CSV.** It is there so an interrupted run can
  rebuild a file that is a rendering of it. Its `--resume` and `--fresh` flags,
  its rubric-mismatch refusal, its mid-record repair and its 900 lines of tests
  are all machinery for the spreadsheet.
- **The report is not attributable per frame.** Reading `scores.csv` in year two
  and asking "which rubric made this 4?" means finding the run's terminal output
  or the ledger header. The number travels without its provenance.

## Solution

The rating goes where the cull reads it: `xmp:Rating` in an XMP sidecar beside
each frame, written as the run goes, next to the RAW the photographer will import.
The sidecar also carries the provenance of that rating in a namespace nobody else
writes, so a number never travels without saying which rubric and which model
made it.

The run leaves alone any frame whose sidecar already carries a rating of 1–5. That
is one rule doing two jobs: it protects a human's rating, and it makes a re-run
cheap — a run interrupted at frame 380 of 400 re-asks 20 frames, with no ledger,
no state file and no `--resume` flag. `--force` exists for the one case the rule
cannot know about: "rate them again anyway".

The report and the ledger both go. In their place: sidecars on disk, the existing
frame lines and summary on the terminal, and `--json-scores` for debugging. The
terminal output stays what it was, except where honesty requires a word: a skipped
frame says `kept`, and the headline counts only what this run rated.

## User Stories

1. As a photographer, I want the model's rating written to the XMP sidecar beside
   each RAW, so that Lightroom Classic shows the stars at import without me
   retyping anything.
2. As a photographer, I want the sidecar named `DSC_0001.xmp` (not
   `DSC_0001.NEF.xmp`), so that Lightroom and Bridge find it and darktable does
   not treat it as its own history file.
3. As a photographer, I want `xmp:Rating` to be an integer 1–5, so that every
   application that reads XMP understands it, not only Lightroom.
4. As a photographer, I want the tool to never write `xmp:Rating=-1`, so that a
   low rating is never silently turned into the destructive reject I did not ask
   for.
5. As a photographer, I want the tool to never write `xmp:Label` or `xmpDM:pick`,
   so that the colour label and pick flag stay mine. *(One exception, decided
   later: a group's non-keepers get `xmp:Label="Red"` — see
   [ADR 0007](../../docs/adr/0007-the-red-label-marks-the-non-keepers.md). The
   rule holds for every other case: a label this tool did not write is never
   overwritten, no label is ever removed, and `xmpDM:pick` is never written.)*
6. As a photographer, I want the rating's provenance — rubric hash, model alias,
   confidence, when it was scored, which version scored it — in a private
   namespace inside the same sidecar, so that a suspicious 4 can be traced
   without a log file.
7. As a photographer with an established Lightroom catalogue, I want a frame whose
   sidecar already has a rating of 1–5 to be left untouched, so that a run over a
   half-culled shoot does not overwrite my own stars.
8. As a photographer, I want such a frame to cost no request and no preview
   extraction, so that a re-run over 400 already-rated frames finishes in seconds
   and costs nothing.
9. As a photographer, I want `--force` to rate frames again even though their
   sidecars already carry a rating, so that I can re-judge an already-rated shoot
   under a different rubric.
10. As a photographer, I want a forced run to still merge rather than replace, so
    that my develop settings, keywords and GPS survive it.
11. As a photographer, I want a `.bak` copy of a sidecar that already existed
    before the run overwrites it, so that a wrong legend is one file-copy away
    from being undone.
12. As a photographer, I want no `.bak` clutter beside frames the run only created
    sidecars for, so that a first pass over a fresh shoot leaves the folder no
    messier than it found it.
13. As a photographer, I want `.bak` files never to overwrite an older `.bak`, so
    that two forced runs cannot destroy the only copy of the original.
14. As a darktable user, I want the run to refuse to write a sidecar that carries
    a `darktable:` namespace, so that darktable does not later rewrite my rating
    from its own history.
15. As a darktable user, I want `--allow-foreign` to override that refusal for one
    run, so that the guard is a guard and not a wall.
16. As a photographer, I want a sidecar that carries foreign XML to keep every
    byte of it — develop settings, keywords, GPS, orientation — so that the file
    the other application wrote is the file it reads back.
17. As a photographer, I want the sidecar written atomically, so that a killed run
    or a full disk leaves either the old sidecar or the new one, never half of
    one.
18. As a photographer, I want a frame the model never answered to leave its
    sidecar completely alone, so that "no answer" never appears on disk as a
    rating of zero, a blank, or a new empty file.
19. As a photographer, I want a frame the run could not write to be reported as
    `xmp_write_failed` and stop the run, so that a read-only mount or a full disk
    costs seconds rather than 400 paid requests whose numbers went nowhere.
20. As a photographer, I want a sidecar I cannot safely extend to be reported as
    `xmp_unreadable` and left alone, so that a malformed file from another tool is
    a skipped frame and not a corrupted document.
21. As a photographer, I want the frame line to say `4★ kept` for a skipped frame,
    so that the stream of frames shows which ones the run's money did not touch.
22. As a photographer, I want the headline to read `scored 20 of 400 … · 380
    skipped`, so that a run that made 20 requests never claims to have scored 400.
23. As a photographer, I want a run in which every frame was skipped to say so in
    one warning and still exit successfully, so that a no-op invocation is
    distinguishable from a broken one.
24. As a photographer, I want `--print-sidecars` to show me, per frame, what the
    run would write and what is already there, so that I can check the legend
    against 20 frames before letting it loose on 400.
25. As a photographer, I want `--print-sidecars` to write nothing at all, so that
    reviewing costs only the requests I chose to spend.
26. As a photographer, I want `--print-sidecars` to show skipped frames as
    untouched, so that the preview accounts for the whole folder rather than only
    its unrated part.
27. As a photographer, I want the CLI's existing output — frame lines, the status
    clause, the stars histogram, the rating's mean and median, every warning — to
    keep its current wording, so that my scripts and my reading of the terminal do
    not move.
28. As a photographer, I want an identical preview within one run to reuse the
    earlier frame's answer, so that a 20-frame burst still costs one request.
29. As a photographer, I want `--no-dedupe` to keep working, so that I can prove
    the filename never mattered.
30. As a photographer, I want `--dry-run` to keep its current meaning — previews
    written, no requests — so that the cheap pipeline self-test stays cheap.
31. As a photographer, I want `--json-scores` to keep writing the raw answer
    documents, so that a surprising score can be re-derived without paying again.
32. As a photographer, I want my next run not to report my own sidecars as "400
    skipped (not RAW)", so that the left-out line still means what it says.
33. As a photographer, I want a config file that says `csv = "scores.csv"` to fail
    with a message naming what changed, so that I do not spend a run believing a
    file is being written.
34. As a photographer, I want to delete nothing but my old `scores.csv`, so that
    upgrading is not a migration.
35. As a photographer whose sidecar got deleted, I want `--force` to re-rate that
    frame, so that losing one file costs one request rather than a re-run I cannot
    ask for.
36. As a photographer, I want the run to never modify a RAW file, so that a cull
    can never threaten the negative.
37. As a maintainer, I want the ledger, its flags and its machinery deleted rather
    than left dormant, so that nothing in the tree exists to serve a report that
    is no longer written.
38. As a maintainer, I want the sidecar document update to be one pure function
    over text, so that byte-preservation is provable without a shoot folder.
39. As a maintainer, I want the sidecar writer to be an injected port, so that the
    abort-on-failure path is testable without chmod-ing a directory.
40. As a maintainer, I want the glossary's `Report`, `Ledger` and `Resume` terms
    retired and `Sidecar` and `Skip` added, so that the vocabulary matches the
    tool.

## Implementation Decisions

### Modules and interfaces

- **`report` becomes `sidecar`.** It keeps the per-frame `Row` (what the run
  accumulates: the frame, its answers, its usage, its failure, its duplicate
  provenance, the rubric hash and the model alias) and loses everything that
  renders a CSV — `COLUMNS`, the cells function, the RFC 4180 writer, the
  `Writer::open`/`append` pair. `Outputs` shrinks to the single field it still
  needs: `json_scores: Option<PathBuf>`.
- **`ledger` is deleted**, with everything that exists for it: `Record`,
  `Provenance`, `FileIdentity`, `ResumeKey`, `Header`, `LedgerMode`,
  `Ledger::recorded`/`reusable`/`truncated_bytes`/`discarded_frames`, the
  mid-record and "plain score starts the ledger over" warnings, `Run::resumed`
  and `Summary::resumed`, and `tests/ledger.rs`. `Row` is built from a `Frame` and
  the run's provenance directly instead of from a ledger record.
- **A `sidecar` module owns the document** and exposes two things: a pure document
  update (`existing document or none` + `this frame's properties` → the text to
  write, or a classified refusal) and a `Writer` port:

  ```text
  trait Writer {
      fn write(&self, target: &Path, document: &str, existing: Option<&str>) -> Result<Written>;
  }
  ```

  `Written` says whether a sidecar was created or replaced (which is what decides
  a `.bak`), never the rating — the run already knows it. The production
  implementation is a filesystem writer (temp file in the same directory, `rename`
  over the target, permission bits preserved when the target existed); tests
  inject a broken writer to prove the abort path. `scoring::run` takes the writer
  as a second explicit port beside `Decider` — ports are behaviour and belong in
  the argument list, not in `Config`.
- **Ingest** stops counting `.xmp` and `.bak` as `skipped_extension`: they are the
  run's own output, not files the photographer left out. No new counter.

### The sidecar document

- **Filename:** the frame's base name with the extension replaced —
  `DSC_0001.NEF` → `DSC_0001.xmp`, case-insensitively for `nef`, `nrw`, `dng`.
  Never `DSC_0001.NEF.xmp`: that name belongs to darktable.
- **Written beside the frame**, inside the shoot folder. The run therefore writes
  into the folder it scores; it still never modifies a RAW.
- **Properties**, and only these:

  | property | value |
  |---|---|
  | `xmp:Rating` | the model's `stars`, integer 1–5 |
  | `imagecull:confStars` | the model's own confidence number, unrounded |
  | `imagecull:model` | the alias the run used |
  | `imagecull:rubricHash` | the rubric hash's first 8 hex characters — the same spelling the terminal prints |
  | `imagecull:scoredAt` | RFC 3339, UTC, `Z` |
  | `imagecull:version` | the binary's version |

  Namespace `https://ns.imagecull.app/1.0/`, declared on the `rdf:Description` we
  insert into. There is no `imagecull:stars`: the rating is `xmp:Rating`, full
  stop, and two properties holding one number is a disagreement waiting to happen.
- **Merge, never replace.** The document update is a targeted text edit, not a
  parse-and-reserialise: locate the first `rdf:Description` opening tag, remove any
  `imagecull:` attributes from it, replace an existing `xmp:Rating` value there if
  present (or add the attribute if not), add the namespace declarations the tag
  does not already carry, and leave every other character of the document
  untouched — quoting style, attribute order, comments, CDATA, foreign namespaces.
  No XML dependency is added; `roxmltree` is rejected along with reserialisation,
  because byte-preservation is the requirement and a parser cannot give it.
- **A document with no `rdf:Description`**, or one whose opening tag cannot be
  located unambiguously, is a per-frame refusal (`xmp_unreadable`), never a guess.
- **A new file** gets a minimal `x:xmpmeta`/`rdf:RDF`/`rdf:Description` document
  carrying only our properties.
- Attribute values are escaped for XML (`&`, `<`, `>`, `"`). Nothing else is
  escaped and nothing is reformatted.

### What a run does, per frame

1. **Skip check.** Read the frame's existing sidecar. If `xmp:Rating` there is an
   integer 1–5 and `--force` was not given: no preview extraction, no request, no
   write. The frame is reported `4★ kept` and counted as skipped.
2. **Foreign guard.** If the sidecar carries a `darktable:` namespace and
   `--allow-foreign` was not given: `xmp_foreign`, no write, no request. `--force`
   does not bypass this — the two flags answer different questions ("rate again"
   and "write into another application's file"), and one must not imply the other.
3. **Prepare, dedupe, request, read** — unchanged from today, except that the
   dedupe memory is an in-process map from dedupe key to the earlier frame's
   answers. Dedupe no longer reaches across processes; that is the accepted cost
   of deleting the ledger, and the second half of ADR 0002's promise dies with it
   (ADR 0002 gets an amendment line; its key is unchanged).
4. **Write the sidecar** through the `Writer` port, atomically.
5. **Emit the frame line** — after the write, so the line never claims a rating the
   disk has not got. A frame that fails to write emits its line carrying the
   `xmp_write_failed` status and stops the run.

A frame whose scoring failed writes nothing at all: no sidecar, no `.bak`, no
empty file, no touch.

### Statuses and exit behaviour

- `xmp_write_failed` — the write itself failed. A failed frame in every sense: it
  joins the status clause, so the exit code already says something is wrong, and
  it **aborts the run** at that frame (the same reasoning as an unwritable
  output directory today: it will fail identically for every remaining frame, and
  each attempt has already been paid for).
- `xmp_unreadable` — the existing sidecar cannot be safely extended. A failed
  frame; the run continues.
- `xmp_foreign` — a darktable sidecar without `--allow-foreign`. A failed frame;
  the run continues.
- `skipped_existing_rating` is **not** a failure and does not appear in the status
  clause: refusing to overwrite a human's rating is the tool working. It is
  counted as `skipped` on the summary's optional line, and it does not affect the
  exit code.
- Backups: a `.bak` is written only when the target sidecar already existed, and
  never over an existing `.bak`. A newly created sidecar produces none.

### CLI surface

- **Added:** `--force` (rate frames whose sidecars already carry a rating),
  `--allow-foreign` (write into a sidecar carrying another application's
  namespace), `--print-sidecars` (score everything, print what would be written,
  write nothing — no sidecar, no `.bak`).
- **Removed:** `--out`, `--absolute`, `--resume`, `--fresh`. `--absolute` has no
  consumer once the CSV's `file` column is gone; `--json-scores` keys use the
  in-folder path spelling.
- **Unchanged:** `--dry-run` and `--previews-dir` (previews written, no requests,
  no sidecars), `--no-dedupe`, `--quiet`, `--verbose`, `--json-scores`, `--limit`,
  `--recursive`, `--rubric`, `--shoot-context`, `--max-edge`, `--jpeg-quality`,
  and the shared server/config flags.
- **Config file:** `[outputs]` keeps `json_scores` only. `csv` and `absolute` are
  removed from the schema, and since unknown keys are already fatal, an old
  `imagecull.toml` that sets them fails with a message naming what moved and
  where the ratings go now.
- **Rendering:**
  - frame line, skipped frame: `4/412  DSC_0421.NEF  4★ kept  eta 6m2s` — the
    existing line's field order and spacing, with `kept` where a latency would be
  - headline (`--quiet`'s single line, the one a script reads): `scored 20 of 400
    in 0m41s · ok 20 · 380 skipped`, where `scored` counts only ratings this run
    produced. The default summary block keeps its `scored:` and `status:` lines
    and adds `skipped: N` to its optional second line.
  - optional second line, only when non-zero: `skipped: 380`
  - every frame skipped: exits 0, with one warning: every frame already had a
    rating, pass `--force` to rate them again.
  - `--print-sidecars`, one block per frame, in ingest order:

    ```text
    DSC_0421.NEF  would create DSC_0421.xmp
      xmp:Rating              4      (was: none)
      imagecull:confStars     0.63

    DSC_0422.NEF  would update DSC_0422.xmp
      xmp:Rating              2      (was: 3)

    DSC_0423.NEF  would leave DSC_0423.xmp alone (rating 5, pass --force to rate again)
    ```

### Documentation the change owes

- `GLOSSARY.md`: `Report`, `Ledger` and `Resume` deleted; **Sidecar** (the `.xmp`
  beside a frame, carrying its rating and the provenance of that rating) and
  **Skip** (a frame whose sidecar already carries a rating, which the run leaves
  alone) added; `Rating` amended to say it is written to `xmp:Rating` in the
  sidecar; `Dedupe` amended to say it is within one run.
- **ADR 0006** — "the rating lives in the sidecar": deleting v1's deliverable, the
  ledger with it, and writing into the shoot folder, with the alternatives
  (`--out` retained, sidecar opt-in, sidecar-as-state) and the accepted costs.
- **ADR 0002** — amended: the dedupe key is unchanged, but its memory is now
  in-process, so it no longer applies to outcomes restored by a resume.
- `SPEC.md`: §9 becomes the sidecars section, §9.1 and §9.2 deleted, §12 folded in
  and deleted, §11's flag list and §15's determinism/caching text rewritten.
- `imagecull.toml.example`: `[outputs]` trimmed to `json_scores`.

## Testing Decisions

**What makes a good test here:** assertions on what a photographer or another
application can observe — the files left in the shoot folder, the events a
`Reporter` receives, the statuses a summary counts, the text the terminal would
print. No assertions on private helpers, no assertions on the order in which the
writer happened to be called.

**Seams (agreed):**

1. **`scoring::run` over a tempdir shoot folder** — the one behavioural seam.
   Prior art: `tests/report.rs`, `tests/failure_taxonomy.rs`, `tests/scoring.rs`,
   using `tests/support::{shoot, decider::Scripted, scenes}`.
2. **The pure document update** in the sidecar module — documents in, documents
   out, no filesystem. Bytes that are not ours are the assertion.
3. **The `Writer` port** with an injected failing implementation — the only way to
   reach the abort path portably. The scripted writers live in `tests/support`.
4. **`ingest::scan`** for `.xmp`/`.bak` counting, in `tests/ingest.rs` where the
   other counting tests live.
5. **CLI rendering functions** for the frame line, headline, second line, warning
   and print block, in the in-crate unit tests that already pin exact output.

**What must be pinned:**

- A clean run over a shoot folder leaves one `.xmp` per accepted frame, no `.bak`,
  and no other new file; a failed frame leaves no sidecar at all.
- The document merge preserves foreign bytes: given a sidecar with `crs:`, a
  keyword bag, differing quote styles and an unusual attribute order, the written
  document equals the original with only our properties added/changed (assert by
  comparing the whole text, not by parsing).
- A second run over the same folder makes **no requests** and writes **no files**,
  and reports `skipped` equal to the frame count.
- `--force` re-rates, creates exactly one `.bak` per existing sidecar, and never
  overwrites an existing `.bak`.
- A sidecar carrying `darktable:` is refused without `--allow-foreign` and written
  with it.
- A sidecar with no `rdf:Description` is refused as `xmp_unreadable`; the file is
  byte-identical afterwards.
- A frame whose write fails carries `xmp_write_failed` and the run stops there; the
  frames after it make no requests.
- `--print-sidecars` writes nothing to the shoot folder while costing the requests
  it prints for.
- Dedupe fires within one run (two identical previews, one request) and does not
  fire across runs.
- The skip check runs before preview extraction: a scripted decider sees no
  request, and a frame with an unreadable preview but an existing rating is
  skipped rather than failed.
- Summary and frame-line rendering: `4★ kept`, `scored 20 of 400 … · 380 skipped`,
  `skipped: 380` present only when non-zero, and every existing line unchanged when
  nothing is skipped.

**Deleted:** `tests/ledger.rs`; the ledger and CSV assertions inside
`tests/report.rs`, `tests/summary.rs`, `tests/config.rs` and
`tests/failure_taxonomy.rs` are rewritten rather than deleted.

## Out of Scope

- Reading sidecars as resume state beyond the skip rule (no reuse of a previous
  run's confidence, latency or raw document).
- `xmp:Label` (with the one exception of [ADR
  0007](../../docs/adr/0007-the-red-label-marks-the-non-keepers.md): the `Red` on
  a group's non-keepers, written only where the label is ours to write and never
  removed), `xmpDM:pick`, rejection (`-1`), collections, keywords, GPS or any
  other catalogue field — including the spec's old "confidence in our namespace is
  preserved by other apps" claim, which stays untested here.
- XMP embedded in the RAW. The RAW is opened read-only and never modified.
- Any XML dependency, and any full document reserialisation.
- Cross-run or cross-process dedupe.
- Importing an existing `scores.csv` into sidecars; a stale CSV is simply deleted
  by the photographer.
- `imagecull groups`, `groups.csv`, the measures, and `doctor` (beyond compiling).
- The Tauri GUI.

## Further Notes

- **The upgrade path is a deletion.** No migration, no importer: `score` no longer
  reads or writes `scores.csv`, and the old file is dead weight. A stale
  `imagecull.toml` key fails loudly rather than silently, which is the only
  migration the change owes.
- **The dangerous failure is a wrong legend labelling 400 frames in place.** That
  is why `--print-sidecars` exists and why it is cheap to run first; the review
  protocol is 20 frames by eye, then the shoot. This replaces SPEC §12.5's
  `--xmp --dry-run` protocol, which could never have worked: a diff of a rating
  requires a rating, and `--dry-run` makes no requests by design.
- **Why the ledger is not replaced.** The sidecar *is* the state, and the skip rule
  reads it. The consequence is honest and cheap: a deleted sidecar costs one
  request, and a run killed at frame 380 re-asks 20 frames.
- **Cost of the skip rule.** The run cannot tell a rating it wrote from one a human
  wrote. That is the whole point of the rule, and it is also why `--force` and the
  `imagecull:rubricHash` property both matter: the hash is how a skip can be
  explained, and `--force` is how a re-score under a new rubric gets to happen at
  all.
- **Ordering note for the implementer:** the write precedes the frame line so the
  terminal never reports a rating that is not yet on disk. This is the one ordering
  constraint in the per-frame loop that is deliberate rather than incidental.
