# 0011 — The cull starts when a group is final

Status: accepted · 2026-11-28 · Feature `parallel-cull` ·
amended by [ADR 0020](0020-the-cleanup-drops-doctor-and-the-measures-csv.md),
which drops the preflight and the measures CSV ·
supersedes `SPEC.md` §7.1's "the client sends one at a time" for the group cull ·
amends `SPEC.md` §7.3 and §14's serial-workload claim

## Context

`imagecull groups` became a cull in ADR 0008: after the model-free pass finds the
groups, each group of two or more is compared and its non-keepers are marked. The
two passes ran strictly one after the other, because grouping was a pure function
of *all* the readings: `assign` sorted every frame's facts by capture time and
swept them, so a plan existed only after the last preview had been decoded.

That is one framing of the cost, and it is the wrong one for a cull. A group's
comparison is a round trip of seconds; a shoot of forty groups therefore spends
most of its wall clock waiting for answers, and `--jobs` — which overlaps the
local preview reads — does nothing about it. The cull also re-extracts each
frame's preview to build the comparison, so every group has local CPU (decode,
resize, JPEG encode) that could be overlapped with another group's server wait.

The grouping algorithm has one property the batch shape hid: **a group is final
as soon as the next frame in capture order is beyond its gap**. The gates after
the time gate cannot make a later frame join an earlier group, and a new group
closes the previous one. So the sweep is already a stream; only the metadata pass
was missing to feed it in capture order.

## Decision

**Grouping streams, and the cull starts the moment a group is final.**

A cheap **metadata pre-pass** opens every file for its header — capture time,
body serial, exposure, orientation — before any preview is decoded. That is what
places frames in capture order without their signatures, and it is the one new
cost: one extra file open per frame, against a preview extraction that is the
expensive half. The preview pass still reads in the scan's order, so frame lines
keep arriving in the order a photographer knows; the readings are *consumed* in
capture order, and the sweep closes a group using the next frame's header before
that frame's preview has been decoded.

**`cull::run_streaming` judges groups on `--cull-jobs` workers (default 4), and
waits for every one before the report and the exit.** Each worker takes one
finished group, so its sidecar writes never meet another group's. The first group
of two or more runs the one preflight before any worker is given a group; a
preflight that fails does not abort the grouping pass, because the measures CSV
is still worth writing — the cull stops, the plan finishes, the caller writes the
CSV, and the run exits fatal. A folder that turns out to have only singletons
never asks the server at all, and its retirements wait until the folder has shown
that no group of two exists.

**The cull may have several comparative requests in flight at once.** This
supersedes §7.1's "never a concurrent burst" for `groups`. oMLX serialises
requests server-side, so the degree is not a claim about server parallelism; it
is a client-side cap that overlaps each group's local preview rendering with
another group's wait, and that a photographer can lower (`--cull-jobs 1`) when the
server is shared. `score`'s per-frame ratings stay one at a time: a rating run
has no independent units to overlap.

**Grouping semantics do not change.** `assign` remains the reference for a folder
as a whole, and both it and the streaming pass drive the same `Grouper`; the
streaming pass must produce the same plan, CSV, groups and hash at any `--jobs`
and `--cull-jobs`. The report is ordered by group number even though workers
finish out of order, and the run does not print it until the last worker is done.

## Considered options

- **Start culling only after the whole folder is grouped.** The safe, small
  change: parallelise groups once `assign` returns. It keeps one pass order, but
  it leaves the cull waiting for the last preview of a long read, which is the
  wait this ADR exists to remove.
- **Keep one request at a time and overlap only local preview rendering.** A
  defensible reading of §7.1: N workers prepare previews, a gate lets one
  `compare` through. It preserves the old wire promise, but it pays a lock for
  every group and gives up the case that actually matters — a server that is
  slower than local CPU, which is every local Clef deployment.
- **Stream grouping without a metadata pre-pass.** Impossible without either
  assuming filenames follow capture time (false across two bodies, and the
  algorithm is documented as name-blind) or buffering every preview until the
  whole folder is read (the status quo, with more memory).
- **One thread per group.** Unbounded request bursts and unbounded preview
  memory. `--cull-jobs` is a cap, not a target.
- **Reuse `--jobs` as the cull degree.** `--jobs` is frames read at once; a
  sixteen-core read degree would also mean sixteen groups' previews in memory,
  and the two degrees describe different resources. They are separate flags.
- **Write the report before the workers finish.** The report says what the cull
  decided; printing it before the last verdict would make a partial delete set
  read as complete, and the user asked for the wait explicitly.
- **Drop the CSV when the preflight fails.** The old order (group, CSV, cull)
  makes the measures available whatever the server does. Streaming cannot write
  the CSV before the groups are found, but it can still finish the plan after a
  failed preflight and write it before exiting fatal.

## Consequences

- **A group can be culled while later previews are still being decoded.** The
  common shoot — filenames and capture times in the same order — starts its first
  comparison almost immediately after the first group's frames are read.
- **Frame lines and cull lines interleave.** Frame events stay in the scan's
  order, but a verdict or a `--print-sidecars` block can appear before a frame
  line whose scan index was still blocked. The final report and the CSV are
  ordered exactly as before, and the summary counts what was actually decided.
- **`--cull-jobs` is a new invocation flag.** It conflicts with `--no-cull`
  (a pass that does not cull has no cull degree) and is not read from a config
  file: it describes this machine and this server, not the shoot.
- **Every frame is opened twice.** The header pass and the preview pass each open
  the file. The cost is bounded by the header parse; if a real shoot shows it
  dominating, the extraction API can grow a variant that returns the chosen
  preview descriptor from the header pass, not a second full pipeline.
- **A failed preflight still pays for the whole read.** That is the old cost too
  (the plan came first), and it is what keeps the CSV honest.
- **Concurrency is a wire change.** §7.1 no longer promises one request at a
  time; the cull's degree is bounded by `--cull-jobs`, and a server that cannot
  take the load is answered with `--cull-jobs 1` rather than a code change.
- **The new seam is small.** `group::Sink` receives a `Found` group; `cull`
  exposes one public `judge` for a group and a `run_streaming` that owns the pool.
  The CLI composes them; neither module learns about the other's terminal.

**ADR 0017 gave the degree a config key.** The consequence above — "`--cull-jobs`
… is not read from a config file: it describes this machine and this server, not
the shoot" — no longer holds: `[group] cull_jobs` sets it for a shoot and the flag
still beats the file. The decision this ADR records — a fan-out whose degree is
bounded by the caller, one preflight before any worker is given a group, a cap
rather than a target — stands unchanged, and the degree stays out of the grouping
hash exactly as the "same plan at any degree" rule here requires.
