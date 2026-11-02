# Spec — rustfmt and clippy run on commit

Status: ready-for-agent
Date: 2026-12-04
Source: `.scratch/groups-config/spec.md` (its acceptance line already asks for
`cargo clippy --workspace --all-targets` clean), `Cargo.toml`
(`[workspace.package] rust-version`, exact-pinned deps), `AGENTS.md`.

---

## Problem Statement

Every ticket in this repo ends with the same acceptance sentence: `cargo test
--workspace` green, `cargo clippy --workspace --all-targets` clean. Nothing runs
it. The gate is a line a human reads and an agent re-reads, and git history shows
what that is worth — a branch can land with a warning, or with formatting that
only the next person's editor settles, and the record does not say so.

There is no CI either: nothing in the repo runs a check automatically, so the
only enforcement of the acceptance line is whoever remembered to type it.

The workspace is already clean on both counts today (`cargo fmt --all --check`
exits 0; `cargo clippy --workspace --all-targets` is silent in 1.7s
incremental). This is therefore wiring, not cleanup: the cost of the gate is
paid once at setup, and per commit it is the seconds the build cache already
saves.

## Solution

A committed git hook and the config line that enables it, plus CI as the gate
the hook cannot be trusted to be.

- `.githooks/pre-commit` — POSIX `sh`, committed, mode `100755`. Formats and
  re-stages staged Rust files, then runs clippy over the workspace and blocks on
  any warning.
- `git config core.hooksPath .githooks` — the one-time per-clone setup, recorded
  in `AGENTS.md`. A relative path is resolved against the root of the working
  tree, so it follows every worktree without extra setup (verified).
- `.github/workflows/ci.yml` — `cargo fmt --all --check`, `cargo clippy
  --workspace --all-targets -D warnings`, `cargo test --workspace` on push and
  pull request. The hook is the fast local loop; CI is the gate.

## Decisions taken (ADR 0013)

- **`.githooks` and `core.hooksPath`, not a hook manager.** lefthook, `pre-commit`
  and `cargo-husky` each add a tool or a dev-dependency to a repo that has
  neither, and `cargo-husky` installs its own fixed hook body. The script is
  forty lines of `sh`.
- **Formatting is fixed, clippy blocks.** `rustfmt` is deterministic and
  semantics-preserving, so reformatting and re-staging a staged file costs the
  committer nothing and removes a whole class of "run fmt then commit again"
  round trip. A clippy warning is a judgement call the hook cannot make, so it
  stops the commit and says what it saw.
- **Workspace-wide, `--all-targets`, `-D warnings`.** The acceptance line already
  says this, and `--all-targets` is what reaches the `#[cfg(test)]` code where the
  next warning will actually be.
- **`--locked` on the clippy invocation.** A hook that rewrites `Cargo.lock` while
  you are committing is a hook that fails the commit it just dirtied.
- **Nothing Rust-ish staged ⇒ exit 0 immediately.** A docs or `.scratch` commit
  pays nothing.
- **No tests in the hook.** `cargo test --workspace` in CI only: the suite reads
  RAW fixtures and a pre-commit that runs a minute is a pre-commit that gets
  bypassed.

## The four hazards, and the answers

1. **Partially staged file.** `rustfmt` rewrites the whole working-tree file and
   `git add` would then sweep the unstaged hunks into the commit. The hook
   detects any staged file that also has unstaged changes, refuses to touch *that
   file*, and aborts with the two ways out (`git stash --keep-index` or commit the
   hunk). It never silently commits work you did not stage.
2. **Formatting needed outside the staged set.** Fixing the staged files can leave
   `cargo fmt --all --check` still failing, because the offender is an unstaged
   file. The hook re-runs the workspace check after fixing and, if it still
   fails, aborts naming the files instead of formatting someone's work in
   progress behind their back.
3. **`rustfmt` recursing into child modules.** `rustfmt src/main.rs` also formats
   `mod` children by default (verified), which would edit files the committer did
   not stage. The fix runs with `--config skip_children=true`, one file at a time.
4. **Missing component.** `rustfmt` and `clippy` are rustup components, not part
   of `cargo`. A preflight prints `rustup component add rustfmt` / `add clippy`
   instead of failing with `error: no such command: fmt`.

## What the hook is not

`git commit --no-verify` skips it, `git rebase` does not run it (verified — git
only runs `pre-commit` for `commit` and `commit --amend`), and nothing forces a
clone to run the `git config` line. That is why CI exists and why CI, not the
hook, is described as the gate. The hook's job is to make the gate cheap.

## Out of scope

- `cargo test` in the hook, a `pre-push` hook, or any hook that reads the RAW
  fixtures.
- Formatting-only tools: `taplo` for `Cargo.toml`/`imagecull.toml`, `typos`, a
  markdown formatter. Add them when a commit is actually broken by their absence.
- A `rust-toolchain.toml` pinning the channel. `rust-version = "1.95"` is an MSRV,
  and the installed toolchain here is already 1.98; pinning a channel to make
  rustfmt reproducible would contradict the MSRV policy for a risk that `cargo
  fmt --check` in CI catches anyway.
- An `xtask` crate or a `justfile`. The command is two lines; a build step to run
  them is a third crate to compile before you can commit.
- Nightly `rustfmt` options (`imports_granularity`, `group_imports`). They need a
  pinned nightly, and `rustfmt.toml` does not exist yet.

## Acceptance

- `.githooks/pre-commit` is committed with mode `100755`
  (`git ls-files -s .githooks/pre-commit` starts `100755`).
- In a fresh clone: `git config core.hooksPath .githooks`, then a commit of a
  badly formatted `.rs` file reformats it, re-stages it, and succeeds; a commit
  that adds `#[allow(clippy::all)]`-free triggering code fails with the clippy
  message and the commit does not happen.
- A docs-only commit (`git commit -m 'docs: ...' -- .scratch/...`) prints nothing
  and takes no measurable time.
- A file with both staged and unstaged hunks aborts with the partial-staging
  message and leaves the index exactly as it was.
- `AGENTS.md` names the setup line, and `docs/adr/0013-*.md` records the
  mechanism choice and the auto-fix/block split.
- The workflow runs the same two commands on a fresh runner and goes green on
  `main`.

## Tickets

- `01-pre-commit-hook.md` — the script, its two modes, and the setup line.
- `02-ci-workflow.md` — `.github/workflows/ci.yml`, calling the same script in
  check-only mode.
- `03-docs-and-adr.md` — `AGENTS.md` section, ADR 0013.
