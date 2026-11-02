# 02: Docs — the example, the spec, the README, the glossary and ADR 0016

Status: resolved
Type: task

**What to build:** every place that described `[preview]`, `[outputs]` or `[run]`
as a section a photographer can write.

## Changes

- `imagecull.toml.example`: `[preview]`, `[outputs]` and `[run]` collapse into one
  `[score]` section with all six keys and their defaults. The header comment keeps
  the "what a file cannot set" list, and gains a line saying `[score]` is what
  `score` reads, `[group]` what `groups` reads, `[server]`/`[scan]` what both do.
  `[group]`'s comment says `groups` borrows `[score] shoot_context` for the
  comparison's framing.
- `SPEC.md` §11.1: four sections; `[score]`'s six keys; "which command reads
  which key" rewritten around the two command sections; the preview sentence names
  `[score]` for `score` and `doctor`, `[group]` for `groups`; a sentence records
  that the retired sections are fatal and name the move.
- `SPEC.md` §6.4: "`[preview]` never reaches a cull" becomes `[score]`.
- `README.md`: the config block in "Config file" is the four-section shape; the
  `--max-edge` table row and the "what gets written" note say `[score]`.
- `GLOSSARY.md`: **Effective config** lists score parameters under their section;
  **Preview** says which section owns which pair. Add **Score section** and
  **Group section** entries only if the existing prose does not already carry the
  idea — the sections are file structure, not domain vocabulary, so prefer
  naming them in the existing entries.
- New `docs/adr/0016-the-score-section-holds-what-a-rating-needs.md`: context
  (three sections for one command, ADR 0014's rejection and the cost it was
  trading), decision, options considered (keep three sections; alias the old
  ones; put `shoot_context` in both command sections; move the shared keys too),
  consequences (breaking change, named by the strict loader; four sections;
  `groups` borrows one key; the internal `score_` prefix).
- `docs/adr/0003-…`: an Amendments bullet recording that `[preview]`,
  `[outputs]` and `[run]` became `[score]`, so the five-section map is no longer
  the file's shape while the decision it records stands.
- `docs/adr/0010-…`: an Amendments bullet recording that `[run] shoot_context`
  became `[score] shoot_context`, and that its "keys under `[run]`" bullet and
  its `[preview] jpeg_quality` note are superseded by ADR 0016.
- `docs/adr/0014-…`: an Amendments bullet recording that its considered option
  "moving both pairs under a per-command subsection" was later taken, and its
  compatibility objection is paid by the tombstoned sections (ADR 0016) rather
  than by aliases.

## Tests

`ConfigFile::from_toml(include_str!("../../../imagecull.toml.example"))` must
still pass, and the no-config-file default test must be unchanged — the example
is the one document that is executed.

## Answer

Applied as listed. `imagecull.toml.example` is four sections in the order
`[server]`, `[scan]`, `[score]`, `[group]`, and says in its header that the three
retired sections are refused by name; `[group]`'s comment records that a cull
borrows `[score] shoot_context`. `SPEC.md` §6.4 and §11.1, the README's config
block and paragraph, and `GLOSSARY.md`'s Preview / Effective config / Config file
entries now describe four sections. ADR 0016 is new; ADRs 0003, 0010 and 0014
each gained an Amendments paragraph, 0014's recording that its rejected option
was taken and paid by tombstones rather than aliases.

`the_committed_example_parses` still passes, so the example is a config the
strict loader accepts.
