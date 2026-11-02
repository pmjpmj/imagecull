# 0003 — The config file is TOML, and exactly one of them is read

Status: accepted · 2026-11-15 · Feature `config-file`

## Context

Everything that stays true across runs — endpoint, model alias, rubric path,
shoot context, preview size and quality, output paths — could only be said as a
flag. A photographer scoring forty shoots re-types the same eight flags, and the
one value that must not drift between two runs of a shoot (the rubric) is the
easiest one to mistype. Spec §11 has no config file; this records the file it
now has.

Adding one forces six questions at once: the format, what the file may set,
which file is read, what beats what, what a typo does, and whether the file's
identity is part of a run's provenance.

## Decision

**TOML, using the `toml` crate already pinned for the rubric.** No new
dependency, no new lockfile entry, and one parser for both files, so a
malformed config and a malformed rubric report the same way and point at the
same line-and-column machinery.

**Keys grouped into five sections that mirror the core types they fill:** `[server]` (`endpoint`, `model`, `timeout`) → the server arguments,
`[preview]` (`max_edge`, `jpeg_quality`) → `Params`, `[scan]` (`recursive`) →
`ScanOptions`, `[outputs]` (`csv`, `json_scores`, `absolute`) → `Outputs`,
`[run]` (`rubric`, `shoot_context`, `dedupe`, `csv_rewrite_every`) → the rest of
`scoring::Config`. A section that fills nothing is a typo, so it is rejected.

**The file carries what persists across runs; flags carry what describes this
invocation.** Not settable from a file: the positional shoot folder, and
`--dry-run`, `--resume`, `--fresh`, `--limit`, `--quiet`, `--verbose`. A config
that can silently turn a real run into a dry run, or resume a ledger from
another day, is not a convenience.

**Exactly one file is resolved, in this order:** `--config <FILE>`, else
`./imagecull.toml`, else `$XDG_CONFIG_HOME/imagecull/config.toml`, else
`~/.config/imagecull/config.toml`, else the built-in defaults. The first that
exists wins whole. Precedence over its contents is
**flag > file > built-in default**.

**The file is read by all three subcommands**, not only `score`: a `doctor`
preflight that certifies a different pipeline than the run it is certifying is
worse than no preflight. `models` uses `[server]` and ignores the rest.

**Strict.** Unknown keys and unknown sections are fatal (exit 2), values are
validated to the same ranges clap enforces on the flags, an unreadable
`--config` target is fatal, and a malformed file is fatal with the parser's own
line and column. Nothing is clamped and nothing is warned-and-ignored.

**Paths in the file are relative to the file's directory**, so a shoot config
that travels with its folder keeps working wherever the folder is mounted.

**Resolution lives in the core**, not in the argument parser: every
file-overridable clap field becomes `Option<T>` and a core function merges the
file's values with the flags into the effective config. The terminal still
resolves nothing (`cli.rs`'s own rule).

**Provenance is display-only for now.** When a file was loaded, the summary's
provenance line prints it beside the rubric hash and the model alias. Its
content hash is *not* added to the ledger header.

## Considered options

- **YAML.** More popular in general (K8s, CI), but no YAML crate is in the tree;
  `serde_yaml` is archived and its successors are pre-1.0, which conflicts with
  the workspace rule that every dependency is pinned exactly so `cargo build`
  cannot drift. YAML's implicit typing (`no` → `false`, `1.0` → float) is also a
  new failure class in a tool whose whole claim is that two runs are comparable.
- **Layering a user config under a shoot config.** Rejected: it makes the
  effective config a merge of two files, so "which values am I running" stops
  having one answer, and a key can arrive from a file the photographer forgot
  about.
- **`IMAGECULL_*` environment overrides.** The same objection, in a form with no
  file to point at. Env vars also need clap's `env` feature, which is off.
- **clap's `value_source` trick** (keep `default_value`, ask `ArgMatches`
  whether a flag was given). It preserves `[default: …]` in `--help`, but it
  moves the merge into the presentation layer, threads `ArgMatches` through
  three `run()` signatures, and leaves the core unable to say what an effective
  config is without a process.
- **An inline `[rubric]` section.** Rejected: a rubric's identity is its file
  and its hash, and an inline copy invites a second, drifting wording of the
  text ticket 08 is about to freeze.

## Consequences

- `--help` no longer prints `[default: 1568]` for the file-overridable flags,
  because the default now depends on a file. The numbers move into the help
  prose, and `cli.rs`'s "defaults are the spec's defaults" test becomes a test
  of the resolver rather than of the parser.
- A config file is now a way to change the effective config without touching a
  flag, which makes an existing hole easier to walk into: resume keys on file
  identity, so `--resume` after changing `max_edge` or `shoot_context` reuses
  rows scored from different bytes. This is recorded, not fixed here — the fix
  is a config (or payload) hash in the ledger header, which belongs with the
  flags too and is its own ticket.

## Amendments

**Ticket 09 removed `csv_rewrite_every` from `[run]`.** The CSV is now appended
one row per judged frame rather than rewritten on an interval, so the key had
nothing left to control. The strict loader is the whole deprecation: a file that
still sets it fails as an unknown key, naming it. The section map above is
otherwise unchanged.

**ADR 0010 added a sixth section, `[group]`** (`gap`, `similarity`,
`exposure_stops` → `group::Thresholds`), and `groups` joined the file's readers
for `[server]`, `[preview] jpeg_quality`, `[scan] recursive`, `[run]
shoot_context` and `[group]`. The decision this ADR records — TOML, one file,
flag > file > default, strict, core-side resolution — is unchanged; only the
section list and the reader list grew.

**ADR 0010 added a sixth section, and ADR 0016 cut the file back to four.** The
five-section map above and ADR 0010's sixth became `[server]`, `[scan]`,
`[score]` and `[group]`: the rating's keys from `[preview]`, `[outputs]` and
`[run]` now share one section, and a file that still writes a retired section is
refused by name. The decision this ADR records — TOML, exactly one file read,
flag > file > default, strict, core-side resolution, per-key `Resolved` — is
unchanged; only the section list and the reader list shrank.

**ADR 0019 shrank `[server]` from eight keys to six.** `path` became part of
`endpoint` and `selector` became `model`: the address a request is posted to is
one value, `{model}` and all, and the body's `model` is the same name the address
interpolates. `token_env`, `token` and `truncate` keep the no-flag clause below.
The resolution discipline is untouched: `endpoint` and `model` are still fields
on `Resolved`, merged in the one place, their defaults core constants — and
`--endpoint` now takes exactly what the key takes.

**ADR 0018 grew `[server]` from three keys to eight, and gave `[group]` a
seventh.** `path`, `selector`, `token_env`, `token` and `truncate` describe the
HTTP wire a run posts on, and `[group] max_images` the comparison's batch cap, so
the section a file writes about its server is now the request's shape rather than
only its address. One clause of this ADR is amended in the narrowest way: "flag >
file > built-in default, per key" still holds, but those six keys *have* no flag,
so for them the rule is file > default. The resolution discipline is otherwise
untouched — every one of them is a field on `Resolved`, merged in the one place,
with its default a core constant.
