# Spec — config file for imagecull

Status: ready-for-agent
Date: 2026-11-15
Source: `SPEC.md` §11.1, `docs/adr/0003-toml-config-file-one-source-at-a-time.md`,
`GLOSSARY.md`. These tickets implement ADR 0003.

---

## Problem Statement

Everything that stays true across runs — endpoint, model alias, rubric path,
shoot context, preview size and quality, output paths — can only be said as a
flag. A photographer scoring forty shoots re-types the same eight flags, and the
one value that must not drift between two runs of a shoot (the rubric) is the
easiest one to mistype. `--help` has no answer for "I always shoot with the same
rubric and the same preview size".

## Solution

A TOML config file whose values resolve into the same effective config the flags
already produce. Keys grouped in five sections mirroring the core types they
fill; exactly one file resolved, never layered; flag beats file beats default;
unknown keys and out-of-range values are fatal; relative paths resolve against
the file's directory; all three subcommands read it; the summary says which file
was used.

Nothing about the pipeline becomes reachable only from a file: every key names a
core value that a flag also names.

## User Stories

1. As a photographer with a rubric I tuned myself, I want it named once in
   `./imagecull.toml` next to the shoot, so that I cannot mistype its path on the
   run that matters.
2. As a photographer whose endpoint and model never change, I want them in
   `~/.config/imagecull/config.toml`, so a run inside a shoot folder needs no
   flags at all.
3. As a photographer scripting a run, I want `--max-edge 1024` to beat my file
   without editing it, so a one-off stays one-off.
4. As a photographer who suspects a stale config file, I want `--no-config`, so
   that "how does it behave as shipped" is one flag away.
5. As a photographer running `doctor` before a big run, I want `doctor` to read
   the same file `score` will, so the preflight certifies the pipeline I am
   about to use.
6. As a photographer who mistyped `max_edeg`, I want the run to stop and name the
   key, so I do not score 300 frames at the wrong resolution.
7. As a photographer reading a summary two weeks later, I want it to tell me
   which config file produced it, so the numbers are traceable.

## Decisions taken (ADR 0003)

- TOML via the already-pinned `toml` crate. No new dependency.
- Sections `[server]`, `[preview]`, `[scan]`, `[outputs]`, `[run]`; snake_case
  keys; `#[serde(deny_unknown_fields)]` on every struct.
- Not settable from a file: the shoot folder, `--dry-run`, `--resume`,
  `--fresh`, `--limit`, `--quiet`, `--verbose`.
- Discovery, first hit wins whole: `--config`, `./imagecull.toml`,
  `$XDG_CONFIG_HOME/imagecull/config.toml`,
  `~/.config/imagecull/config.toml`, built-in defaults. No layering, no
  environment variables.
- `--no-config` conflicts with `--config`.
- Values validated to the same ranges clap enforces (`max_edge >= 1`,
  `1 <= jpeg_quality <= 100`, `csv_rewrite_every >= 1`); out of range is fatal,
  never clamped.
- Paths in the file resolve against the file's directory.
- Resolution is core-side: file-overridable clap fields become `Option<T>` and a
  core function merges them with the file's values.
- Provenance is display-only: the summary's provenance line gains the config
  path. The ledger header is unchanged (see ADR 0003, Consequences).

## Out of scope

- A config content hash in the ledger header, and refusing `--resume` on a
  mismatch. Deferred deliberately; it applies to flags as much as to files.
- An inline `[rubric]` section. A rubric's identity is its file and its hash.
- YAML, JSON or `.rc` in any form. Comment-preserving config rewriting.
- Per-genre or per-checkpoint profiles beyond the two discovery locations.

## Acceptance

`cargo test --workspace` green, plus a manual run of the sequence in
`.scratch/config-file/issues/02-config-file-cli-wiring.md`, and a
`imagecull doctor` that reads a config file and reports the endpoint from it.

## Tickets

- `01-config-file-parse-and-resolve.md` — core: `ConfigFile`, discovery,
  merging, validation.
- `02-config-file-cli-wiring.md` — `cli.rs` `Option<T>` fields, `--config` /
  `--no-config`, all three subcommands, summary provenance line.
