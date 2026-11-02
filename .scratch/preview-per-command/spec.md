# Spec — a separate preview config for a rating and a comparison

Status: done
Date: 2026-11-02
Source: `SPEC.md` §6.4 and §11.1,
`docs/adr/0014-the-comparison-has-its-own-preview-config.md`,
`docs/adr/0010-the-group-section-holds-the-grouping-thresholds.md`,
`imagecull.toml.example`.

---

## Problem Statement

`score` sends one preview per request and `groups` sends up to twenty, so the two
want different amounts of picture — 1568 px is a good rating size and a bad
comparison size. The tool had one and a half knobs: `[preview] max_edge` reached
only `score` and `doctor`, while `[preview] jpeg_quality` reached both. A
photographer could therefore configure the comparison's compression but not its
size, which is the number that moves tokens, detail and whether a group of twenty
fits the model's 16,384-image-token context at all.

`keeper::Budget` held the comparison edge as a measured constant with a doc saying
a run does not take it from the command line. When a group is too large for the
context the server answers `413`, the group is reported undecided, nothing is
marked — and the `413` message could name no remedy, because the remedy was the
number that could not be set.

## Solution

Two preview pairs, one per kind of request, each configurable from a flag and
from the config file:

| What | Flag (`score` / `groups`) | Key | Default |
|---|---|---|---|
| A rating's preview | `--max-edge`, `--jpeg-quality` | `[preview] max_edge`, `jpeg_quality` | 1568 px, 88 |
| One image of a comparison | `--max-edge`, `--jpeg-quality` | `[group] max_edge`, `jpeg_quality` | 512 px, 88 |

- `Resolved` gains `group_max_edge` and `group_jpeg_quality`; `Effective` gains
  `group_params: Params`. Both pairs resolve flag-over-file-over-default through
  the same merge, and neither reaches the other command.
- `keeper::Budget` keeps only what is measured about the model's answer (`max`
  frames per request, the confidence `floor`); the payload shape moves to
  `cull::Config::preview`, defaulting to `keeper::default_preview()`.
- Validation matches the flags: an edge of at least 1 px, a quality in 1–100,
  fatal with the key named.
- A comparison's `413` now names the flag that would make it fit, and still
  refuses rather than quietly re-rendering smaller.

## Acceptance

- `imagecull groups shoots/ --max-edge 256` renders each comparison preview at
  256 px on its long edge, and leaves a rating's preview at 1568.
- `[group] max_edge` / `jpeg_quality` in a shoot config reach the cull; the same
  keys under `[preview]` do not.
- `[group] max_edge = 0` and `[group] jpeg_quality = 101` are fatal and name the
  key.
- `cargo test --workspace` green, `cargo clippy --workspace --all-targets` clean.
