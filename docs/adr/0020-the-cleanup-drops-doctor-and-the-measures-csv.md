# 0020 — The cleanup drops `doctor` and the measures CSV

Status: accepted · 2026-12-02 · Feature `cleanup` ·
supersedes [ADR 0018](0018-the-server-section-describes-the-wire-not-the-vendor.md)'s
"`doctor` and `models` stay oMLX-only" and its preflight,
[ADR 0019](0019-the-endpoint-is-the-whole-address.md)'s `doctor` half,
[ADR 0016](0016-the-score-section-holds-what-a-rating-needs.md)'s "`doctor`
reads `[score]`", and the measures-CSV half of
[ADR 0008](0008-the-group-command-culls.md),
[ADR 0010](0010-the-group-section-holds-the-grouping-thresholds.md) and
[ADR 0011](0011-the-cull-starts-when-a-group-is-final.md) ·
amends `SPEC.md` §5.1, §7, §11.1 and §14 and the README

## Context

Two features had outlived the shape of the tool around them.

**`doctor` was a second front end for a diagnosis nothing else needed.** It read
oMLX's `/health`, `/v1/models` and `/v1/models/status`, classified the result
into nine verdicts, and printed one sentence. A run also wanted the same
information — refusing a broken server before a 400-frame shoot — but wanted it
*leniently*: only what the server had proved unfit, never what it had merely left
unsaid. So `doctor` grew two readings of one diagnosis (`is_ready` and
`can_score`), a file-check mode that rendered the preview pipeline on one file,
and a CLI refusal path for a config that addressed some other server. The
strictness existed for explaining an install; the run used the leniency. The
explanation was the only consumer of the half that mattered, and `models`
already answers the question a photographer actually asks before a shoot: what
does this server serve, and is it a decision model?

**The measures CSV was a deliverable without a reader.** `groups` wrote one row
per frame — group, measures, status, error — to `groups.csv`, with `--out` to
name another path, `COLUMNS`, `render_csv`, a CSV escaper, and CLI tests that
re-derived the grouping from the file. The report on stdout already names every
group, its keeper, its reds and its summary; the per-frame measures are on the
progress lines while the pass runs. No consumer existed for the file, and the
"one row per frame" shape kept a second, stale copy of the plan alive.

## Decision

**Remove `doctor`, its file check, and the run's preflight.** The `doctor`
subcommand and `imagecull-core/src/doctor.rs` are deleted. `Decider` loses
`preflight`; `cull::Pool` loses the hold-back that waited for it, `Streamed` loses
`fatal`, and `group`'s streamed sink dispatches every group as it arrives. A
broken server now shows up as the first frame's classified failure, and a refused
credential (401/403) still stops the run at one request rather than four hundred.
`models` keeps the oMLX-only refusal and the `asks_omlx` predicate behind it.

**Remove the measures CSV.** `groups` no longer writes a file: `--out`,
`DEFAULT_GROUPS_CSV`, `group::render_csv`, `COLUMNS`, `csv.rs` and the CSV
assertions in the tests are deleted. The command keeps its per-group report and
its summary, and the grouping hash stays in the summary because it is the identity
a later run is compared against.

## Alternatives considered

- **Keep `doctor` and have the run call it.** Rejected: the run only ever needed
  `can_score`, and keeping both readings of one diagnosis keeps the strict half
  alive for a front end that is not being built. `models` is the useful half.
- **Keep the preflight by inlining its probes into `Adapter`.** Rejected as the
  same feature under a different name: it is still three GETs and a verdict table
  whose only consumer is a run that can classify the first request instead.
- **Print the CSV to stdout instead of writing a file.** Rejected: the report is
  what a photographer reads, and a machine-readable stream was never consumed.
  Keeping the renderer for a redirect nobody uses is the cost without the reader.
- **Keep `--out` as an empty flag for compatibility.** Rejected: a flag that does
  nothing is worse than a fatal unknown-flag error, and this is a pre-1.0 tool.

## Consequences

- `imagecull doctor` is an unknown subcommand. The README's setup section now
  stops at `models`, and `score --dry-run` is the preview self-test.
- A run against a server that cannot answer fails its first frame with the
  taxonomy's message instead of a pre-run sentence. The message still names the
  endpoint, the status and the variable to check.
- `cull::Streamed` is `{ plan, culled }`; the CLI writes no report for a cull
  that never started because there is no longer a cull-wide start to fail.
- The grouping-hash identity (ADR 0017) is unaffected: the plan, the labels and
  the hash are the same at any degree; only the CSV that was compared alongside
  them is gone.
- `imagecull-core` loses `doctor` and `csv` from its module list; `SPEC.md`'s
  crate layout loses `csv.rs`.
