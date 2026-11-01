# 0010 — The `[group]` section holds the grouping thresholds

Status: accepted · 2026-11-27 · Feature `groups-config` ·
amends [ADR 0003](0003-toml-config-file-one-source-at-a-time.md)'s "five
sections" · supersedes [ADR 0009](0009-the-settings-gate-tolerates-exposure-drift.md)'s
"`--group-exposure-stops` is a command-line flag and not a config key" ·
amends `SPEC.md` §11.1

## Context

ADR 0003 gave the config file five sections and named three subcommands as its
readers: `score`, `doctor`, `models`. `groups` did not exist yet in that shape.
It arrived with `burst-grouping-v1`, took `--recursive`, `--group-gap`,
`--group-similarity` and `--group-exposure-stops`, and read the file only through
`CommonArgs` — endpoint, model, timeout.

Two things followed, and both were wrong in the same direction: a key the file
documents behaving as if it were absent.

`[scan] recursive` is documented as "also scan subdirectories", with no command
named. `score` honours it. `groups` built its `ScanOptions` straight from the
flag (`self.recursive.unwrap_or(false)`), so a shoot config that turned recursion
on was read, parsed, validated — and then not consulted. Worse, the flag could
only add: `--recursive false` had nothing to turn off, because the file's `true`
never reached the scan.

The thresholds were the honest half of the same problem: no section to put them
in, so they were flags only, and ADR 0009 recorded that as deliberate ("the
config file has no grouping section, and adding one is a separate decision").
The decision is due. The values are exactly the kind that stay true across runs:
a photographer who brackets deliberately, or who pans, retypes the same tolerance
on every pass over the same folder, and the one number most worth freezing — the
similarity gate that decides whether a pan splits — is the easiest to mistype.

## Decision

**A sixth section, `[group]`, filling `group::Thresholds`:** `gap` (seconds,
default 2.0), `similarity` (a cosine 0–1, default 0.90) and `exposure_stops`
(default 2.0). Same section shape as the others: every key optional, unknown keys
fatal, a value outside the range its flag enforces fatal with the key named.

**`groups` becomes a reader of the file, on the keys that describe its run:**
`[server]` (the endpoint and the alias a cull asks — an alias already reached it,
so this is the half that was working), `[preview] jpeg_quality`, `[scan]
recursive`, `[run] shoot_context`, and all of `[group]`. It scores nothing and
renders its comparisons at its own measured budget edge, so `[run] rubric`,
`[run] dedupe`, `[outputs] json_scores` and `[preview] max_edge` remain keys it
does not consult.

**One resolution path, not two.** `GroupsArgs::overrides()` reports the flags as
`Resolved` (including `recursive`, so the file's `true` can be beaten by
`--recursive false`), and `GroupsArgs::options()` takes the settled `Effective`.
No key is reachable from a file without going through the same
flag-over-file-over-default merge `score` uses, and the threshold defaults stay
the core constants (`group::DEFAULT_*`), not a second copy in the CLI.

**The numbers are invocation's opposite.** `--limit`, `--jobs` and `--out`
describe this run — a smoke test, a degree, a scratch file — and stay flags. A
degree changes nothing about what a group is, and a limit deliberately stops
before the rest of the folder.

**No new provenance.** The thresholds already enter `grouping_hash`, so a
verdict computed under a file's numbers is not readable under another's. Nothing
about where the numbers came from needs to be written down for the hash to work.

## Considered options

- **Keys under `[run]`.** `[run]` fills what is left of `scoring::Config`, and a
  cull has no scoring config. Hanging a group's tolerances off the scoring
  section would say the two are one thing, which is precisely what ADR 0008
  separated.
- **Thresholds under `[scan]`.** The scan decides which *files* a run covers, not
  what counts as one moment. `ScanOptions` has no place for a tolerance, and
  inventing one to avoid a section is the same "misfiled key" the strict loader
  exists to prevent.
- **Leaving `groups` as it is and documenting the holes.** The missing
  `[scan] recursive` is not a missing feature but a silently ignored key, and the
  file's whole claim is that a typo or a stale entry costs nothing. A documented
  "this section applies to `score` only" would still leave `--recursive false`
  unable to cancel a file.
- **Making `--jobs` and `--out` configurable too.** The degree is a fact about
  the machine, not the shoot, and a file that pinned it would slow a bigger
  machine down without saying so. `--out` names where this run's scratch CSV
  goes; the run's lasting output is the sidecar, which has nothing to configure.
- **A separate `groups.toml`.** Two files, two discovery orders, two answers to
  "which values am I running". ADR 0003 rejected layering for the same reason.

## Consequences

- `SPEC.md` §11.1 now lists six sections and says which command reads which key.
  `imagecull.toml.example` gains the section and stops implying that every key
  reaches every command.
- ADR 0009's bullet is superseded: the exposure tolerance is a key now. Its
  first two consequences — brackets grouping, and the glossary's Group entry
  being the corrected rule — stand unchanged.
- A shoot config can change how a whole folder groups, which makes a stale
  `[group]` entry a silent change of verdicts rather than of scores. The grouping
  hash is what keeps that from being silent in the sidecars: a different
  `exposure_stops` produces a different `groupingHash`, so a verdict computed
  under another number is refused as incomparable rather than believed.
- `[preview] max_edge` still does not reach a cull, and now the docs say so
  instead of leaving a photographer to discover it. If a real shoot shows the
  comparison budget needs the same knob, that is a separate decision about the
  budget, not a missing config key.
- `Resolved` and `Effective` lose `Eq` (they now carry `f64`). `PartialEq` is
  what the tests and the merge need; nothing compared an effective config for
  total equality.
