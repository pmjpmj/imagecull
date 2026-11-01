# 04: The CLI's flags and the words it prints

**Type:** task
**Status:** resolved
**Blocked by:** 03
**Spec:** [../spec.md](../spec.md) §"CLI surface", §"Testing Decisions" seam 5

## What to build

The command line catches up with the core: three flags added, four removed, and
four pieces of text that now have something new to say. Nothing else about the
terminal's output moves.

- Added: `--force`, `--allow-foreign`, `--print-sidecars`.
- Removed: `--out`, `--absolute`, `--resume`, `--fresh`, and the config keys that
  fed them (already gone from the schema in ticket 03 — this is the flag side).
- `--print-sidecars` scores, prints one block per frame and writes nothing at all
  (no sidecar, no `.bak`), including the `would leave … alone` line for frames the
  skip rule covers.
- Frame line for a skipped frame: `DSC_0421.NEF   4/412   4★ kept   0.0s`.
- Headline: `scored 20 of 400 in 0m41s · ok 20 · 380 skipped`, where `scored`
  counts only ratings this run produced.
- Optional second line, only when non-zero: `skipped: 380`.
- Every frame skipped: exit 0, one warning saying every frame already had a
  rating and `--force` would rate them again.
- Every existing line and warning keeps its current wording when nothing is
  skipped.

## Acceptance

- `--print-sidecars` over a tempdir writes no file into the shoot folder, while
  the scripted decider records exactly the requests the printed blocks describe.
- A run with skips prints the three new lines above and no other change.
- A run with no skips is byte-identical to today's output apart from the headline
  clause the spec names.
- `--out`, `--absolute`, `--resume` and `--fresh` are rejected as unknown
  arguments, and `--help` no longer mentions them.
- `--dry-run` still writes previews and makes no requests and no sidecars.
- The in-crate rendering tests pin the exact strings; the flag tests pin that each
  new flag reaches the core `Config`.

## Notes

- Prior art for exact-output tests: the unit tests at the bottom of
  `crates/imagecull-cli/src/progress.rs`.
- The `Terminal` reporter renders `Event`s; `--print-sidecars` is a run that
  suppresses writes rather than a new pipeline, so it should not fork `run`.
- Keep the summary's `status:` and `stars:` clauses untouched.

## Comments

### Evidence (2026-11-19)

`--force`, `--allow-foreign` and `--print-sidecars` on `score`; the latter
conflicts with `--quiet`/`--verbose`, because it says its own thing about every
frame. `--out`, `--absolute`, `--resume` and `--fresh` are gone, refused as
unknown arguments and absent from `--help`.

`Verbosity::Print` is the preview mode: the started line is suppressed, an
answered frame prints nothing there (its block comes from the printing writer),
a kept frame prints `DSC_0014.NEF  would leave DSC_0014.xmp alone (rating 5, pass
--force to rate them again)`, a failure prints in the same shape, and the run
still ends with the summary block.

`crates/imagecull-cli/src/printer.rs` is the printing `Writer`: it renders
`frame  would create/update sidecar` and one line per property with the previous
value in parentheses, and writes nothing at all — no sidecar, no `.bak`, no temp
file. The writer port now takes the frame path and derives the sidecar, since a
preview has to name both.

A skipped frame's normal frame line is `14/400  DSC_0014.NEF  5★ kept  eta 7m37s`;
the quiet one-liner already carried `N skipped` in the headline.

A run in which every frame was kept now emits one warning telling the
photographer to pass `--force`, and still exits 0.

Tests: frame line for a kept frame, print-mode block for a kept and a failed
frame, print mode's silence on an answered frame, print mode still summarising,
the printer's block shape (new and update, with `was:` values), the three flags
parsing, `--print-sidecars` conflicting with quiet/verbose, the four removed flags
being refused, and the all-skipped warning at the run seam.
