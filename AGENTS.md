## Agent skills

### Issue tracker

Issues and specs live as markdown files under `.scratch/<feature-slug>/`. See `docs/agents/issue-tracker.md`.

### Triage labels

Default vocabulary — the five canonical roles used verbatim. See `docs/agents/triage-labels.md`.

### Domain docs

Single-context: `GLOSSARY.md` + `docs/adr/` at the repo root. See `docs/agents/domain.md`.

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
