# 02: `[group] max_images` fills `keeper::Budget::max`

Status: ready-for-agent
Type: task
Spec: `.scratch/cloudflare-clef/spec.md`

**What to build:** the comparison batch cap becomes a config key, settled into
`keeper::Budget::max`, and `groups` builds its budget from the effective config
instead of `Budget::default()`.

## The work

- `GroupSection` gains `max_images: Option<usize>`; `Resolved.group_max_images`;
  `Effective` gains `max_images: usize` settled to `keeper::MAX`.
- `crates/imagecull-cli/src/groups.rs:134` (`let budget = keeper::Budget::default();`)
  becomes `let budget = keeper::Budget { max: effective.max_images, ..Budget::default() };`
  — or an `Effective::budget()` helper, whichever reads better; the floor stays
  `keeper::FLOOR`.
- Validation in `ConfigFile::validate`, beside the other group ranges:
  `server`-free, key-named, `2..=20`:

  ```
  group.max_images must be between 2 and 20 — one image is not a comparison,
  and 20 is the batch a comparison's token budget was measured at
  ```

- Not in the grouping hash: `max_images` is scheduling, like `cull_jobs`
  (`group::hash` and the ADR 0011 story stay untouched). Say so in the example
  config comment, one line.

## Tests

- `crates/imagecull-core/tests/config.rs`: `max_images = 4` reaches
  `Effective::max_images`; `1`, `0` and `21` are fatal with the message above.
- `crates/imagecull-core/src/keeper.rs`: `chunk` already covers 20; add one case for
  `max = 4` over 12 contenders (three batches of four, no batch of one).
- `crates/imagecull-core/src/cull.rs` (the in-crate tests at ~983 already build a
  budget by hand): one cull test with `Budget { max: 4, .. }` over a twelve-frame
  group asserting one decided keeper, `rounds == 3`, `undecided == 0`.
- `crates/imagecull-cli/tests/groups_command.rs`: a shoot config with
  `[group] max_images = 2` produces one request per pair rather than one per group
  (assert on the mock's request count).

## Done

`cargo test --workspace` green; a file without the key still batches 20.
