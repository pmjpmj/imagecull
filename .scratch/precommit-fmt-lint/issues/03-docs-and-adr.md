# 03: Say how to turn the hook on, and record why this mechanism

Status: ready-for-agent
Type: task
Blocked by: 01

**What to build:** the setup line where a contributor (human or agent) will
actually look for it, and the ADR that records the choice of mechanism.

## `AGENTS.md` — a `## Local checks` section

Append after the existing sections, keeping the file's voice (a pointer, not a
tutorial):

```markdown
## Local checks

Formatting and lints run on commit, through `.githooks/pre-commit`: `rustfmt`
fixes and re-stages the staged Rust files, then `cargo clippy --workspace
--all-targets --locked -- -D warnings` decides whether the commit happens.

Enable it once per clone:

    git config core.hooksPath .githooks

A relative path resolves against the worktree root, so linked worktrees inherit
it. `git commit --no-verify` bypasses it and `git rebase` does not run it; CI
runs the same script with `--check-only` plus `cargo test --workspace`. See
`docs/adr/0013-pre-commit-runs-rustfmt-and-clippy.md`.
```

No `README.md` change: the repo has none, and `AGENTS.md` is the file every
contributor here reads first.

## `docs/adr/0013-pre-commit-runs-rustfmt-and-clippy.md`

Match the shape of `docs/adr/0009-*.md` and `0012-*.md`: a `Status:` line
(`accepted · <date> · Feature precommit-fmt-lint`), then Context, Decision,
Consequences, Alternatives. What it has to hold:

- **Context.** Every ticket's acceptance line already says `cargo clippy
  --workspace --all-targets` clean, and nothing ran it. No CI existed. The
  workspace is clean today, so this is enforcement, not cleanup.
- **Decision.** `.githooks/pre-commit` + `core.hooksPath`; formatting is fixed
  and re-staged, clippy blocks; workspace-wide, `--all-targets`, `-D warnings`,
  `--locked`; the hook no-ops when nothing Rust-ish is staged; the same script is
  the CI gate.
- **Why auto-fix for fmt and block for clippy.** `rustfmt` is deterministic and
  semantics-preserving, so fixing it costs the committer nothing and saves a
  round trip. A clippy warning is a judgement — `-D warnings` in a hook is only
  tolerable because the fix is usually one line and the message is right there.
- **Consequences.** The one-time `git config` per clone, and the part-staged file
  abort. `--no-verify` and `git rebase` remain holes, covered by CI. A cold clone
  pays a full dependency build on its first commit.
- **Alternatives rejected.** `cargo-husky` (installs a fixed hook body during
  `cargo test` — a test run mutating `.git/`); `lefthook` / `pre-commit`
  (a tool or runtime the repo does not otherwise have, for forty lines of `sh`);
  plain `.git/hooks/pre-commit` (never versioned, invisible to the next clone and
  to any agent reading the repo); an `xtask` crate (a third crate compiled before
  you can commit); check-only for both tools (every unformatted save costs a
  re-run); warn-only (a gate that never closes); a `rust-toolchain.toml` channel
  pin (see the spec's out-of-scope note — `rust-version = "1.95"` is an MSRV and
  the toolchain here is already 1.98).

`GLOSSARY.md` is untouched: a hook is tooling, not domain language, and the
glossary's job is the vocabulary a photographer and the model share.

## Tests

- [ ] The four `git config` lines in `AGENTS.md` are copy-pasteable and correct.
- [ ] The ADR filename and `Status:` line match the numbered series, and the
      ticket's ticket-list entry in `.scratch/precommit-fmt-lint/spec.md` points
      at it.
- [ ] A fresh clone following only `AGENTS.md` gets a working hook: edit a `.rs`
      file badly, `git commit`, see it reformatted.

## Answer

