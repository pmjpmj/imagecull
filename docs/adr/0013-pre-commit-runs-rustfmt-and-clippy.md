# 0013 — Pre-commit runs rustfmt and clippy

Status: accepted · 2026-11-02 · Feature `precommit-fmt-lint`

## Context

Every ticket's acceptance line already says `cargo clippy --workspace
--all-targets` must be clean, but nothing ran it. The repository had no CI, so
the only enforcement was remembering to run the command. The workspace is clean
today; this change wires enforcement rather than cleaning up existing warnings.

## Decision

Commit `.githooks/pre-commit` and enable it per clone with
`git config core.hooksPath .githooks`. The hook runs rustfmt on staged Rust
files and re-stages any formatting changes, then runs workspace-wide clippy over
all targets with `--locked` and `-D warnings`. It exits immediately when no
Rust-related files are staged. CI runs the same script in `--check-only` mode,
then runs `cargo test --workspace --locked`.

Formatting is fixed automatically because rustfmt is deterministic and
semantics-preserving; fixing it saves a commit round trip. Clippy blocks because
a warning needs a judgment about its fix, and `-D warnings` makes the result
enforceable while keeping the diagnostic in the commit output.

## Consequences

Each clone needs the one-time `git config` command. A partially staged Rust
file is rejected because formatting the working-tree file could stage changes
that were not selected. `git commit --no-verify` and `git rebase` remain outside
the hook, so CI provides the shared gate. A cold clone pays a full dependency
build on its first commit.

## Alternatives

- **`cargo-husky`:** installs a fixed hook body during `cargo test`, making a
  test run mutate `.git/`.
- **`lefthook` or `pre-commit`:** adds a tool or runtime the repository does
  not otherwise need for a short shell script.
- **`.git/hooks/pre-commit`:** is not versioned and is invisible to the next
  clone or an agent reading the repository.
- **An `xtask` crate:** adds another crate to compile before committing.
- **Check-only formatting:** makes every unformatted save require another
  formatting pass and commit attempt.
- **Warn-only clippy:** reports warnings without enforcing the acceptance line.
- **A `rust-toolchain.toml` channel pin:** `rust-version = "1.95"` is the MSRV,
  while the installed toolchain is newer; CI's format check catches formatter
  drift without imposing a fixed channel on contributors.
