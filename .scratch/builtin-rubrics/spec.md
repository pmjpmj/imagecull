# Spec — built-in rubrics: a general default, and a name for each

Status: resolved
Date: 2026-11-22
Source: `SPEC.md` §8 and §11.1, `docs/adr/0004-built-in-rubric-family.md`,
`GLOSSARY.md`, `.scratch/nef-scoring-v1/issues/08-real-shoot-validation-and-rubric-freeze.md`.
These tickets implement ADR 0004.

---

## Problem Statement

The rubric compiled into the binary is the photojournalistic cull, so a run that
does not name a rubric assumes its folder is press work — the very assumption a
mixed folder of portraits, landscapes and snapshots does not support. The
second rubric that exists (`holiday.toml`) is reachable only by typing a path to
a file inside the source tree, so in practice nobody reaches it.

A photographer has no way to say "judge this holiday" except a path they must
keep correct, and no way at all to say "judge this the way you judge by default,
because I have not decided what it is yet".

## Solution

Three rubrics are compiled into the binary and selected by name — `general`
(the default), `press`, `holiday` — with each file staying in the tree as the
copy the binary carries. `--rubric` and the config file's `[run] rubric` hold
either one of those three names or a path, so "which rubric" remains one setting
with one answer, and `flag > file > default` needs no exception.

The new `general` rubric assumes no genre: it reads the frame for what it is
trying to be, and holds craft to a fixed standard regardless. Four per-axis
legends, because the press rubric's shared legend is what ticket 08 measured as
keeping the four scores collapsed.

The name is provenance and never part of the rubric hash, so the same words are
comparable however a run reached them.

## User Stories

1. As a photographer scoring a folder whose genre nobody declared, I want the
   tool to make no assumption about it, so that a mixed import is judged on its
   own terms.
2. As a photographer with a holiday, I want `--rubric holiday`, so that I am not
   typing a path into the source tree on every run.
3. As a photographer with my own tuned rubric, I want `[run] rubric =
   "mine.toml"` to keep working exactly as before.
4. As a photographer comparing two reports, I want two names for identical words
   to produce the same rubric hash, so that a name can never make two comparable
   runs look incomparable.
5. As a photographer who mistyped a name, I want the error to say what the three
   names are, so the failure is one line rather than a trip to the docs.

## Out of scope

Per-genre rubrics beyond these three, a `rubrics` subcommand that lists them, and
freezing the press wording (ticket 08's own remaining work).
