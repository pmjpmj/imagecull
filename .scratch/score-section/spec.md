# Spec — the `[score]` section holds what a rating needs

Status: done
Date: 2026-11-02
Source: `SPEC.md` §11.1, `docs/adr/0003-toml-config-file-one-source-at-a-time.md`,
`docs/adr/0010-the-group-section-holds-the-grouping-thresholds.md`,
`docs/adr/0014-the-comparison-has-its-own-preview-config.md`, `GLOSSARY.md`,
`imagecull.toml.example`.

---

## Problem Statement

The config file's six sections do not hold the same kind of thing. `[group]`
owns one command: every key in it is what `groups` reads, and reading the section
tells you what a cull does. The rating's keys are spread over three — `[preview]`
for the payload shape, `[outputs]` for the raw answers, `[run]` for the rubric,
the shoot's framing and dedupe — and none of the three names a command. `[run]`
is the worst of them: it reads as "settings for a run", but `groups` is a run and
reads exactly one key from it, while `score` reads all of them.

The cost is concrete. A photographer writing a shoot config has to know which of
three sections holds a rating's edge, and the example file has to say, beside
each key, which command reads it. `[group]` needs no such note.

ADR 0014 rejected the symmetric shape — one preview pair per command subsection —
on compatibility grounds: a file that set `[preview] max_edge` would stop
applying. That cost is real but bounded, and it is now one a strict loader can
pay out loud, key by key, instead of silently.

## Solution

Four sections, each a place rather than a pile:

```toml
[server]                # every command
endpoint, model, timeout
[scan]                  # every command that scans
recursive
[score]                 # what a rating needs
max_edge, jpeg_quality, json_scores, rubric, shoot_context, dedupe
[group]                 # what a cull needs
gap, similarity, exposure_stops, max_edge, jpeg_quality
```

`[preview]`, `[outputs]` and `[run]` are gone. A file that still sets one is a
fatal error naming the key's new home, in the style the retired `[outputs] csv`
already uses; `[outputs] csv` and `[outputs] absolute` keep their own refusals
because those two keys did not move, they died with the report.

> Supersedes ADR 0014's rejection of "moving both pairs under a per-command
> subsection", and amends ADR 0003's five-section map and ADR 0010's "keys under
> `[run]`" bullet. Recorded as ADR 0016.

## User Stories

1. As a photographer writing a shoot config, I want every key a rating needs in
   one section, so I do not have to learn three.
2. As a photographer with a `[group]` section I already understand, I want the
   rating to have the same shape, so the file reads as two commands rather than
   six unrelated piles.
3. As a photographer whose old config sets `[preview] max_edge`, I want the run
   to stop and name `[score] max_edge`, so I move one line instead of silently
   rating a whole shoot at 1568 px.
4. As a photographer running `groups` on a shoot with a `shoot_context`, I want
   the cull to frame the comparison the same way the rating did, so one section
   holds that framing for both passes.
5. As a photographer running `doctor <FILE>`, I want it to render the preview
   `score` would send, so the preflight certifies the pipeline that will run.

## Decisions taken (ADR 0016)

- `[score]` holds `max_edge`, `jpeg_quality` (→ `Params`), `json_scores`
  (→ `Outputs`), `rubric`, `shoot_context`, `dedupe` (→ the rest of
  `scoring::Config`).
- `groups` reads `[score] shoot_context` — the one key it borrows. A cull and a
  rating of the same folder deserve the same framing, and a second copy of the
  key under `[group]` would let the two disagree. Every other `[score]` key is
  not consulted by `groups`, exactly as `[group]` is not consulted by `score`.
- `doctor` reads `[score] max_edge` / `jpeg_quality`: its file check renders the
  preview a rating would send.
- The three retired sections are tombstones in `ConfigFile`. Any key set under
  one is fatal and names the move; `outputs.csv` and `outputs.absolute` keep the
  existing "gone with the report" refusals.
- Internal symmetry: `Resolved.score_max_edge` / `score_jpeg_quality` and
  `Effective.score_params` beside `group_max_edge` / `group_params`.
- Ranges and messages move with the keys: `score.max_edge must be at least 1 px…`,
  `score.jpeg_quality must be between 1 and 100, not {x}`.

## Out of scope

- Aliasing the retired sections. A second spelling for every key is the shape
  ADR 0014 was protecting, and it is what this decision gives up.
- Moving `endpoint`, `model` or `timeout` under `[score]`: all four commands ask
  a server, so they stay where every one of them looks.
- Moving `recursive`: both scanning commands honour it.
- Any change to which flags exist, or to what `groups` does with
  `shoot_context`.

## Acceptance

`cargo test --workspace` green, `cargo clippy --workspace --all-targets --locked
-- -D warnings` clean, `imagecull.toml.example` parses under the strict loader,
and a file containing `[preview] max_edge = 1024` fails with a message naming
`score.max_edge`.

## Tickets

- `01-config-score-section.md` — core: the `[score]` section, the three
  tombstones, validation, `Resolved`/`Effective` renames, CLI and test updates.
- `02-docs-example-spec-adr.md` — the example file, `SPEC.md` §11.1 and §6.4, the
  README, `GLOSSARY.md`, ADR 0016, and the amendments to ADRs 0003, 0010, 0014.
