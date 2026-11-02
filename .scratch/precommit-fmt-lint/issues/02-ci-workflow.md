# 02: CI runs the same gate, and the tests

Status: ready-for-agent
Type: task
Blocked by: 01

**What to build:** `.github/workflows/ci.yml`, the first CI in the repo. It runs
the hook in `--check-only` mode plus `cargo test --workspace`, on push to `main`
and on every pull request.

Why it exists: the hook is bypassable (`git commit --no-verify`), git does not
run it during `git rebase` (verified), and a fresh clone has to run one `git
config` line before the hook exists at all. None of those are failures the local
gate can report.

## The workflow

```yaml
name: ci

on:
  push:
    branches: [main]
  pull_request:

concurrency:
  group: ci-${{ github.ref }}
  cancel-in-progress: true

jobs:
  gate:
    # Apple Silicon: the target platform (SPEC §2), and `raw-files` is a C
    # dependency whose arm64 build is the one that ships.
    runs-on: macos-14
    steps:
      - uses: actions/checkout@v4
      - run: uname -m        # confirms the arm64 image; drop once verified
      - uses: dtolnay/rust-toolchain@stable
        with:
          components: rustfmt, clippy
      - uses: Swatinem/rust-cache@v2
      # One definition of the gate: the same script the hook runs, minus the
      # staged-file walk.
      - run: ./.githooks/pre-commit --check-only
      - run: cargo test --workspace --locked
```

## Notes on the shape

- **The script is the definition, not a copy of the commands.** If the gate
  changes (a new lint flag, a `clippy.toml`), one edit moves both the hook and
  CI. The alternative — two literal `cargo` lines here — drifts the first time
  someone tunes the hook.
- **Pinned actions.** The repo pins every dependency exactly because a scorer
  must be reproducible; actions are the same argument. Pin each to a full commit
  SHA at implementation time and keep the `# v4`/`# v2` comment beside it.
- **`--locked` on the test run too**, so CI fails on a stale lockfile rather than
  resolving one.
- **`macos-14` rather than `ubuntu-latest`.** Linux would be cheaper, but the
  target is macOS on Apple Silicon and the RAW path is the thing under test. If
  the runner label turns out to be x86_64 on this plan, keep it and add `uname
  -m` to the log — the tests do not depend on architecture — but write the
  finding down in the ADR.
- **No coverage, no `cargo audit`, no bench comparison.** Each is a different
  gate with its own false-positive rate; none is asked for by any ticket.
- **Tests need no network** (`.scratch/nef-scoring-v1/spec.md`: canned bodies
  through a localhost mock), so the job needs no secrets and no service.

## Tests

- [ ] Push to a branch with a clippy warning: the `gate` job fails at the
      `pre-commit --check-only` step, and the log shows the warning.
- [ ] Push a branch with `cargo fmt`-dirty code: fails at the same step, log
      lists the files.
- [ ] Push a clean branch: green, with `cargo test --workspace` output visible.
- [ ] Confirm the cache restores on the second run (`rust-cache` step reports a
      hit) so a cold build is a once-per-lockfile cost.
- [ ] `main` is green after the merge.

## Answer

