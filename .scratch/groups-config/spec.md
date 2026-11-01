# Spec — the `groups` command reads the config file

Status: ready-for-agent
Date: 2026-11-27
Source: `SPEC.md` §11.1, `docs/adr/0003-toml-config-file-one-source-at-a-time.md`,
`docs/adr/0009-the-settings-gate-tolerates-exposure-drift.md`, `GLOSSARY.md`,
`imagecull.toml.example`.

---

## Problem Statement

The config file documents `[scan] recursive` without naming a command, and
`score` is the only command that honours it. `groups` builds its scan and its
thresholds straight from the flags, so a shoot config can say
`recursive = true`, be parsed and validated, and change nothing about a cull —
and `--recursive false` has nothing to cancel.

The grouping thresholds have no section at all. ADR 0009 recorded that as a
deliberate omission and named the reason: adding one is a decision, not a
by-product. The decision is due, because the tolerances are exactly the values a
photographer retypes on every pass over the same folder — the gap of a deliberate
bracket, the similarity that decides whether a pan splits — and they are part of
the grouping hash, so a mistyped one quietly changes what is comparable.

## Solution

A sixth section, `[group]` (`gap`, `similarity`, `exposure_stops`), filling
`group::Thresholds`, and `groups` wired through the same
flag-over-file-over-default resolution `score` uses. Minimum surface:

- `model` — through `[server] model`, which already reached the cull; keep it
  proven by a test instead of by inspection.
- `recursive` — through `[scan] recursive`, with `--recursive true|false` able to
  beat it in either direction.
- `gap`, `similarity`, `exposure_stops` — through `[group]`, validated to the
  same ranges the flags enforce.

`--limit`, `--jobs` and `--out` stay flags: they describe the invocation.

## User Stories

1. As a photographer with a bracketed shoot, I want `exposure_stops` in the shoot
   config, so every pass over that folder groups it the same way without flags.
2. As a photographer whose shoot folder has `2024-03-11/` beside it, I want
   `recursive = true` to work in `groups` as it does in `score`, so the cull sees
   the frames the rating run saw.
3. As a photographer running one quick pass, I want `--recursive false` to beat a
   file that turned recursion on, so a one-off stays one-off.
4. As a photographer who mistyped `similiarity`, I want the run to stop and name
   the key, so I do not cull 400 frames at the wrong tolerance.
5. As a photographer reading the example file, I want to know which key each
   command reads, so the file cannot silently stop applying to the command I am
   running.

## Decisions taken (ADR 0010)

- `[group]`: `gap` (seconds, > 0), `similarity` (cosine 0–1), `exposure_stops`
  (≥ 0). All optional; `group::DEFAULT_*` are the defaults.
- `#[serde(deny_unknown_fields)]`, same as every other section.
- `Resolved` gains the three keys; `Effective` gains `thresholds: Thresholds`.
- `groups` consults `[server]` (endpoint, model), `[preview] jpeg_quality`,
  `[scan] recursive`, `[run] shoot_context`, `[group]`. It does not consult
  `[run] rubric`, `[run] dedupe`, `[outputs] json_scores` or `[preview]
  max_edge`, and the docs say so.
- `--limit`, `--jobs`, `--out`, `--force`, `--allow-foreign`,
  `--print-sidecars`, `--no-cull`, `--quiet`, `--verbose` stay flags.
- `Resolved` and `Effective` drop `Eq`; they now carry `f64`.

## Out of scope

- `[preview] max_edge` reaching a cull. The comparison budget is a measured
  configuration (`keeper::Budget`), and changing it is a decision about the
  budget, not a config key.
- A `groups` key for `--out` or `--jobs`.
- A grouping profile per genre or per body.

## Acceptance

`cargo test --workspace` green, `cargo clippy --workspace --all-targets` clean,
and a shoot folder whose `imagecull.toml` sets `[scan] recursive = true` makes
`groups --no-cull` list a subdirectory's frame while `--no-config` does not.

## Tickets

- `01-config-group-section.md` — core: section, validation, `Resolved`,
  `Effective`.
- `02-groups-reads-the-file.md` — `cli.rs` overrides and options, the command
  wiring, tests.
- `03-docs-example-spec-adr.md` — example file, `SPEC.md` §11.1, ADR 0010, the
  ADR 0009 amendment, `GLOSSARY.md`.
