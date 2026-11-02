# 01: `.githooks/pre-commit` — fmt the staged files, block on clippy

Status: ready-for-agent
Type: task

**What to build:** one POSIX `sh` script at `.githooks/pre-commit`, committed
mode `100755`, with the behaviour in the spec. No dev-dependency, no new crate,
no tool for a contributor to install.

## The script

```sh
#!/bin/sh
# imagecull pre-commit: rustfmt on the staged Rust files, clippy on the workspace.
#
# Enable once per clone:  git config core.hooksPath .githooks
# Bypass:                 git commit --no-verify
#
# CI runs this same file with --check-only, so the gate has one definition
# (docs/adr/0013-pre-commit-runs-rustfmt-and-clippy.md).
set -u

say() { printf 'pre-commit: %s\n' "$*" >&2; }

check_only=0
[ "${1:-}" = "--check-only" ] && check_only=1

# rustfmt and clippy are rustup components, not cargo subcommands that ship with
# the toolchain: name the fix instead of failing with "no such command".
cargo fmt --version >/dev/null 2>&1 ||
  { say 'rustfmt is missing: rustup component add rustfmt'; exit 1; }
cargo clippy --version >/dev/null 2>&1 ||
  { say 'clippy is missing: rustup component add clippy'; exit 1; }

if [ "$check_only" -eq 0 ]; then
  staged=$(git diff --cached --name-only --diff-filter=ACMR) || exit 1
  # Nothing Rust-ish staged: a docs commit pays nothing.
  printf '%s\n' "$staged" |
    grep -qE '(\.rs$|(^|/)Cargo\.(toml|lock)$|rustfmt\.toml$)' || exit 0

  unstaged=$(git diff --name-only)
  fixed=''
  blocked=''
  for f in $(printf '%s\n' "$staged" | grep -E '\.rs$'); do
    if printf '%s\n' "$unstaged" | grep -qxF -- "$f"; then
      # Partially staged: rustfmt rewrites the whole working-tree file and
      # `git add` would sweep the unstaged hunks into this commit.
      blocked="$blocked $f"
      continue
    fi
    # --edition tracks [workspace.package] edition; skip_children keeps a
    # `mod` child that was not staged out of the write set.
    rustfmt --edition 2024 --config skip_children=true -- "$f" ||
      { say "rustfmt failed on $f"; exit 1; }
    if ! git diff --quiet -- "$f"; then
      git add -- "$f" || exit 1
      fixed="$fixed $f"
    fi
  done

  if [ -n "$blocked" ]; then
    say 'these files have unstaged changes, so formatting them would stage work'
    say 'you did not choose:'
    for f in $blocked; do say "  $f"; done
    say 'either commit them whole, or: git stash --keep-index'
    exit 1
  fi
  [ -n "$fixed" ] && say "reformatted and re-staged:$fixed"
fi

# Whole-workspace check, both modes. If this fails after the fix above, the
# offender is a file outside the staged set and it is not ours to reformat.
if ! cargo fmt --all --check >/dev/null 2>&1; then
  say 'formatting differs from rustfmt; the files are:'
  # rustfmt 1.98 prints `Diff in <path>:<line>:`; older ones print
  # `Diff in <path> at line <n>:`. Both are matched, and the header lines are
  # uncoloured even when the diff body is not.
  cargo fmt --all --check 2>&1 |
    sed -n -e 's/^Diff in \(.*\):[0-9]*:$/  \1/p' \
           -e 's/^Diff in \(.*\) at line [0-9]*:$/  \1/p' | sort -u >&2
  say 'staged changes are already formatted — run: cargo fmt --all'
  exit 1
fi

if ! cargo clippy --workspace --all-targets --locked -- -D warnings; then
  say 'clippy failed; fix the warnings above, or git commit --no-verify'
  exit 1
fi
```

## Notes on the shape

- **`sh`, not bash, not a Rust helper.** `dash`, `sh` and `zsh` all run it; no
  arrays, no `local`, no process substitution.
- **`--locked`.** A hook that rewrites `Cargo.lock` mid-commit dirties the index
  it is guarding.
- **`--edition 2024` is spelled out** because this calls `rustfmt` directly.
  `cargo fmt` would inherit the edition from `Cargo.toml` but formats the whole
  tree. The workspace check at the end is `cargo fmt`, so an edition drift here
  fails loudly rather than diverging silently.
- **Filenames with spaces** are not handled (`for f in $list`). Every path in
  this repo is ASCII snake_case; a `while IFS= read -r` loop is available if that
  ever stops being true.
- **`--check-only` skips the staged-file walk entirely**, so CI needs no git
  state beyond the checkout.
- **Verified while planning** (rustfmt/clippy 1.98): `rustfmt --edition 2024
  --config skip_children=true -- <file>` accepts the `--` separator and leaves a
  `mod` child untouched; the `sed` above extracts the file list from real `cargo
  fmt --all --check` output. **Not yet verified:** the commit-path scenarios
  below — this box has no write access to git, so a human runs them.

## Install line

```sh
git config core.hooksPath .githooks
```

A relative `core.hooksPath` resolves against the root of the working tree, so
linked worktrees under `.pi/worktrees/` pick the hook up with no extra config
(verified against `git worktree add`).

## Tests

The hook is `sh` driving `git`, so its tests are a scenario checklist run by
hand in a throwaway clone (a `cargo test`-hosted harness for this would cost more
than the script):

- [ ] Commit a badly formatted `.rs` file: commit succeeds, file is reformatted,
      `git show --stat` already contains the formatted version, working tree clean.
- [ ] Commit a file with an unused import: commit aborts, clippy's warning is
      visible, `git log` unchanged, the staged content still staged.
- [ ] Commit a `.scratch/*.md` change: no output, no delay.
- [ ] Stage half a file (`git add -p`), then commit: aborts with the partial
      staging message, `git diff --cached` unchanged.
- [ ] Stage a formatted file while an unstaged file needs formatting: aborts
      naming the unstaged file, and the unstaged file is untouched.
- [ ] `git commit --no-verify` with a clippy warning: succeeds.
- [ ] Remove `rustfmt` from `PATH` (`PATH=$(dirname $(command -v cargo)):/usr/bin
      git commit ...`): prints the `rustup component add rustfmt` line.
- [ ] `./.githooks/pre-commit --check-only` on `main`: exit 0, no writes.

## Answer

