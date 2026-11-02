# 0017 — The cull's degree has a config key

Status: accepted · 2026-11-02 · Feature `cull-jobs-config` ·
supersedes [ADR 0011](0011-the-cull-starts-when-a-group-is-final.md)'s
"`--cull-jobs` … is not read from a config file" · amends `SPEC.md` §7.1 and §11.1

## Context

ADR 0011 introduced `--cull-jobs` and drew a line around it: the degree
"describes this machine and this server, not the shoot", so it was a flag and
only a flag. Every other number a cull sends — the three thresholds and the
comparison's preview pair — got a key in `[group]`, because a shoot that needs
one of them needs it on every run.

The degree is the same kind of number, and the line was drawn looking at the
wrong noun. A shoot does not travel with one machine. The same folder is culled
at a desk next to a dedicated oMLX box and again on a laptop that shares a server
with someone else's rating run; the photographer who lowers the degree there is
not describing *this invocation* but *this shoot's home*. A flag that has to be
retyped on every run is exactly what the config file exists to stop, and the
runtime cost of forgetting it is the failure ADR 0011 named — a burst of
comparative requests at a server that cannot take them.

The counter-argument — that a config file describes a shoot and a degree
describes a machine — is real but small, and it is already answered for the
endpoint. `[server] endpoint` is as machine-specific as `cull_jobs` and has been
a key since ADR 0003; the file is a per-shoot (or per-user) statement about the
environment a run happens in, not a claim that every key is a fact about the
pixels.

## Decision

**`[group] cull_jobs`, an integer of at least 1.** It is the most comparative
requests in flight together, the same number `--cull-jobs` takes, and it resolves
through the one merge everything else uses: `GroupSection::cull_jobs` →
`Resolved::group_cull_jobs` → `Effective::cull_jobs`, settling on
`cull::DEFAULT_JOBS` (4) when neither a file nor a flag sets it. The default is
the core constant, not a second copy in the config loader.

**The flag still beats the file.** A shoot that normally runs at 1 can still be
pushed to 4 for an afternoon, and a default of 4 can be dropped to 1 for one run
against a busy server. Precedence is unchanged: flag, then file, then default.

**`0` is fatal, named by its key.** `group.cull_jobs must be at least 1, or no
group would be culled` — the same refusal `--cull-jobs 0` gets, because a degree
of zero would ask for a run that does nothing.

**The degree stays out of the grouping hash.** It is scheduling, not algorithm:
the same folder must produce the same plan, CSV, groups and hash at any degree
(ADR 0011's own test), so a verdict made at 1 and one made at 4 stay comparable.

**`--jobs` stays a flag.** The read degree is this machine's cores and nothing
about the shoot; it is the number the line was drawn for, and it is the only one
left on it.

## Considered options

- **Leave it flag-only.** Defensible under ADR 0011's wording, and rejected for
  the reason above: the retype tax is paid on every run by the user who needs it
  most, and forgetting it is the failure the option exists to prevent.
- **Put it under `[server]`.** The degree is a statement about the server's
  patience, so the section is not absurd. Rejected because the section holds what
  every command shares and `cull_jobs` means nothing to `score` or `doctor`; ADR
  0016's split is by which command reads the key, and `groups` reads this one.
- **Call it `[group] jobs`.** Shorter, and a trap: `--jobs` is the read degree,
  and one word would name two resources. The key matches the flag it overrides,
  so `cull_jobs = 1` and `--cull-jobs 1` read as the same setting.
- **Make it a float or a multiplier of the core count.** A degree is a small
  whole number of groups; a fraction would only mean rounding, and a
  core-derived default would make a laptop and a workstation disagree about the
  same file.

## Consequences

- **A shoot can ship its own degree**, and `--cull-jobs` overrides it for one
  run. Nothing in an existing file changes: the key is optional and unknown keys
  were already fatal, so a typo'd `cull_job` is refused rather than ignored.
- **`imagecull.toml.example`, the README table and sample, `SPEC.md` §7.1 and
  §11.1 name the key**, and §11.1's "what a file cannot set" list keeps
  `--jobs` while losing `--cull-jobs`.
- **ADR 0011's consequence is superseded in one clause**, not in its decision:
  the fan-out, the preflight ordering and "a cap, not a target" stand.
- **The loader's line moves by one key.** `[group]` holds six settings, five
  of which change the request and one of which changes only how many requests
  overlap; the section's comment says which is which.
