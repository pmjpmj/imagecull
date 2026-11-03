# 0016 — The `[score]` section holds what a rating needs

Status: accepted · 2026-11-02 · Feature `score-section` ·
amended by [ADR 0020](0020-the-cleanup-drops-doctor-and-the-measures-csv.md),
which removes the `doctor` reader ·
supersedes [ADR 0014](0014-the-comparison-has-its-own-preview-config.md)'s
considered-but-rejected "moving both pairs under a per-command subsection" ·
amends [ADR 0003](0003-toml-config-file-one-source-at-a-time.md)'s section map
and [ADR 0010](0010-the-group-section-holds-the-grouping-thresholds.md)'s "keys
under `[run]`" bullet · amends `SPEC.md` §6.4 and §11.1

## Context

ADR 0010 gave `groups` a section of its own, and ADR 0014 gave a comparison a
preview pair inside it. Both argued from the same principle: a section owns what
one command needs, so a photographer reading `[group]` learns what a cull does
without consulting a second table.

The rating never got the same treatment. Its six keys sit in three sections that
name no command. `[preview]` holds the payload shape, `[outputs]` the raw
answers, `[run]` the rubric, the shoot's framing and dedupe. `[run]` is the
misleading one: it reads as "settings for a run", but `groups` is a run and reads
exactly one key from it, while `score` reads every key. A file therefore teaches
two different things under one heading, and `imagecull.toml.example` has to
annotate each key with the command that honours it — an annotation `[group]`'s
keys never need.

ADR 0014 saw this shape and declined it, for one reason: symmetry would break
every config in the wild that sets `[preview] max_edge`, or force alias support
"for a shape with no other user". That is a real cost, and it was the right call
while the section map was still settling. It is the wrong call now, because the
loader is strict enough to pay the cost out loud: a retired key can be refused by
name and pointed at its new home, one line per file, at the moment the file is
read. The alternative is a file that stays permanently asymmetric to spare a
migration that a single error message can perform.

## Decision

**Four sections, split by who reads them.** `[server]` (`endpoint`, `model`,
`timeout`) and `[scan]` (`recursive`) are every command's. `[score]`
(`max_edge`, `jpeg_quality`, `json_scores`, `rubric`, `shoot_context`,
`dedupe`) is what a rating needs. `[group]` (`gap`, `similarity`,
`exposure_stops`, `max_edge`, `jpeg_quality`) is what a cull needs. The two
command sections are peers; the two shared ones are what neither command can
own alone.

**The retired sections are tombstones.** `[preview]`, `[outputs]` and `[run]`
still parse, and any key that a file sets under one is fatal with a message that
names the old key and the new one: `preview.max_edge has moved to
score.max_edge`. The two report keys that did not move — `outputs.csv` and
`outputs.absolute` — keep the refusals they already had, because they died with
the CSV report rather than relocating. A key that was never in a retired section
still fails as an unknown field, so a typo is never mistaken for a migration.

**`groups` borrows one key.** A cull reads `[score] shoot_context`, and nothing
else from that section. The framing of a shoot is a property of the shoot, not of
the pass: a dim, high-ISO burst deserves the same description when it is
compared as when it is rated, and a copy of the key under `[group]` would let the
two passes disagree about what the folder is. This is the same borrowing
`[server]` already does for all four commands, made explicit in one direction.

**`doctor` reads `[score]`.** Its file check renders the preview a rating would
send, so it reads the rating's pair; a preflight that certified a different
payload would be certifying a pipeline nobody is about to run (ADR 0003's own
reason for reading the file at all).

**The merge is untouched.** `Resolved` still holds only what a source set, and
`score`/`group` prefixes now name which command's pair a field belongs to
(`score_max_edge` / `group_max_edge`, `Effective::score_params` /
`group_params`). Flag > file > default, per key, unchanged; the only key whose
meaning moves is none of them.

## Considered options

- **Keep three sections and document the split better.** Cheapest, and what the
  example does today. It leaves `[run]` claiming to be about "a run" while
  holding only rating keys plus one key a cull borrows, and leaves the reader
  with three tables to learn for one command.
- **Alias the retired sections instead of tombstoning them.** Nothing breaks,
  but every key keeps two spellings forever, and the strict loader — the thing
  that makes this file trustworthy — starts accepting two shapes for one value.
  ADR 0014's objection was exactly this cost; it is answered by refusing loudly,
  not by carrying both.
- **A `shoot_context` under both `[score]` and `[group]`.** Full symmetry, at the
  price of a value a photographer must keep identical by hand. The passes would
  be free to disagree, so the symmetry would buy confusion.
- **Moving `endpoint`, `model`, `timeout` and `recursive` into `[score]` too.**
  Symmetric to the point of falsehood: `groups`, `doctor` and `models` ask a
  server, and both scanning commands honour recursion. A shared key in a
  command section is the misfiling ADR 0010 argued against.
- **Nested tables (`[score.preview]`, `[group.preview]`).** Rejected in ADR 0014
  in favour of flat keys in `[group]`, and for the same reason here: a section
  with six flat keys is one table to explain.

## Consequences

- A config file that sets `[preview]`, `[outputs]` or `[run]` now fails, naming
  the key's new home. This is the breaking change ADR 0014 was avoiding, now
  paid once, in public, and with the remedy in the message.
- Four sections instead of six, and two of them are commands. The example file's
  per-key "which command reads this" annotations shrink to one line per section.
- `groups` reads exactly one key outside `[group]`/`[server]`/`[scan]`, and both
  the example and §11.1 say so.
- `SPEC.md` §11.1's "keys grouped into six sections, each owning what one command
  needs" becomes four, two of which are shared; ADR 0003's "a section mirrors the
  core type it fills" keeps its amendment from ADR 0014 and gains another.
- ADR 0014's preview split stands unchanged in substance: a rating's 1568 px and
  a comparison's 512 px are still two numbers with two keys, now
  `[score] max_edge` and `[group] max_edge`.
