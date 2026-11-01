# 03: Docs — the example, the spec and the ADR say what a file reaches

Status: resolved
Type: task

**What to build:** every place that described the config surface before
`[group]` existed.

## Changes

- `imagecull.toml.example` gains `[group]` with the three keys and their
  defaults, and the comment says `groups` reads them while `score` ignores them.
  `[scan]` names both commands; `[preview] max_edge` says `score` and `doctor`
  only, with the reason (a cull renders at its own budget edge); `[preview]
  jpeg_quality` says both; `[outputs] json_scores` says it is `score`'s record;
  `[run] dedupe` says `score` only; `[server] model` names all four commands. The
  "cannot set" list gains `--jobs` and `--out`. `the_committed_example_parses`
  keeps it honest.
- `SPEC.md` §11.1: six sections, a "Which command reads which key" paragraph, and
  `groups` added to the provenance sentence. "What a file cannot set" now lists
  `--jobs` and `--out` and records that `--recursive` takes an explicit
  `true`/`false`.
- New `docs/adr/0010-the-group-section-holds-the-grouping-thresholds.md`:
  context (the ignored `recursive`, the missing section and ADR 0009's
  deferral), decision, the options considered (`[run]`, `[scan]`, documenting the
  holes, `--jobs`/`--out`, a second file), and consequences (six sections,
  grouping hash already covering the numbers, `max_edge` still not reaching a
  cull, `Eq` dropped).
- `docs/adr/0009-…`: the "flags only" bullet points at ADR 0010, and an
  Amendments section records that only that bullet changed.
- `GLOSSARY.md`'s Effective config entry lists grouping thresholds.

## Answer

Applied as listed. `cargo test -p imagecull-core --test config` proves the
committed example still parses; `grep` for "five sections" and "the config file
has no grouping section" finds no remaining live claim.
