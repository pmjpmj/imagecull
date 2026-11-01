# 0004 — Three built-in rubrics, and the general one is the default

Status: accepted · 2026-11-22 · Feature `builtin-rubrics` ·
amends [ADR 0001](0001-rubric-renders-two-criteria-shapes.md)

## Context

Until now exactly one rubric was compiled into the binary: the
photojournalistic cull from ticket 03, which also happened to be what a run
used when the photographer said nothing. Ticket 08's evidence made that
awkward — the shoot that was scored end to end was a family import, and the
rubric asked whether each frame was a press keeper. A second rubric
(`holiday.toml`) existed only as a file that `--rubric <PATH>` could reach, with
no name and no discovery.

So the tool had a default that assumed a genre, and a second judgement that
could only be reached by typing a path. Both are wrong for the common case: a
folder of photographs whose genre nobody declared.

## Decision

**Three rubrics are compiled in and selected by name**: `general`, `press`,
`holiday`, each from its own file under `crates/imagecull-core/rubrics/` via
`include_str!`, so the file in the tree is the copy the binary carries and the
copy a photographer reads.

**`general` is the default.** It assumes no genre: it reads the frame for what
it is trying to be — a portrait, a landscape, an event, a street frame, a
snapshot — without holding it to something it never aimed at. The standard does
not move with the genre (craft is judged absolutely); only what counts as the
subject and the moment is read off the frame. `press` keeps the
photojournalistic wording ticket 08 has not yet frozen; `holiday` keeps the
family-album wording. (Since
[ADR 0005](0005-one-rating-replaces-six-questions.md) each built-in is that one
wording of the same single rating question; the family is unchanged, the schema
it varies is smaller.)

**One setting, two forms.** `--rubric` and `[run] rubric` hold either a bare
built-in name or a path:

- `general`, `press`, `holiday` exactly, in lowercase, are names;
- anything else is a path, anchored to its config file's directory when it came
  from one;
- a name always beats a file that shares it, so `./holiday` is how a file is
  said; selection never consults the filesystem;
- a blank value is fatal and lists the three names; a path that cannot be read
  and looks like a bare word is told what the three names are.

This keeps "which rubric" one answer, so `flag > file > default` needs no
exception: `--rubric press` over a file that says `rubric = "mine.toml"` simply
wins, the way `--model` does.

**The name is provenance, never identity.** The rubric hash stays
`sha256(schema + alias)`, so `--rubric holiday` and a path to an identical copy
of that file produce the same hash and comparable reports, and no future
built-in can be born incomparable to an equal file. The run summary and the
`--version` line print the name beside the hash (`rubric general/69139540`) when
a built-in was selected; a file-selected rubric prints the hash alone, because
it has no name to give. `Rubric` carries `Option<Builtin>` for exactly this.

**Vocabulary.** `GLOSSARY.md` gains *built-in rubric* and *general rubric*, and
retires "default rubric": *default* now names the role — the built-in a run uses
when nothing selects another — never a file. The file formerly called
`default.toml` is `press.toml`, so no file is named for a role it no longer has.

## Considered options

- **A separate `--builtin <NAME>` flag beside `--rubric <PATH>`.** No value is
  interpreted, but it makes two keys for one concept, and then
  `--builtin press` over a file's `[run] rubric = "mine.toml"` needs its own rule
  — either a new fatal conflict or a shadowing exception to
  `flag > file > default`. Both are worse than one setting that holds a name or
  a path.
- **Putting the name in the hash.** It would make two spellings of one rubric
  incomparable, which is the opposite of what the hash is for, and it would make
  a built-in uncomparable to the file it was copied from.
- **Freezing `press` now that it is no longer the default.** Ticket 08's own
  evidence says the frame-by-frame review that would justify a freeze is still
  blank. The rename is not an occasion to close that checkbox.

## Consequences

- Every unqualified `imagecull score` now judges differently than it did before
  this change, and every hash already stored in a CSV or a ledger refers to the
  press wording. That is why `press` still loads: an old report can be
  reproduced by naming it, and no old run is silently resumed under new words.
- `--version` no longer identifies "the" rubric but the default built-in. A
  build with a changed `general.toml` prints a different hash, as before.
- A third rubric is now wording work with no code attached: add a `Builtin`
  variant, its file, and its name to `ALL`. That is deliberate — three is the
  number the evidence justifies, not a family of per-genre rubrics.
- The ledger and the CSV are unchanged: the built-in is not recorded there,
  because the hash already says what was judged and the ledger's resume check
  must keep accepting any path to the same words.
