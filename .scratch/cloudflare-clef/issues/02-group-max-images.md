# 02: `[group] max_images` fills `keeper::Budget::max`

Status: resolved (2026-11-28) — the cap is a config key, settled into
`keeper::Budget::max`; two test placements moved (see Comments)
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

## Comments

**Implemented 2026-11-28.** The key, the settle and the wiring landed as written;
`groups` builds `keeper::Budget { max: effective.max_images, ..Budget::default() }`,
so the floor stays `keeper::FLOOR`. Three notes:

1. **The twelve-frame cull test moved to `tests/groups_marking.rs`.** The in-crate
   fixtures at `cull.rs` point at `/no-such-shoot`, so a twelve-frame cull there
   could not render a preview and would only ever assert `undecided == 1`. The
   test lives where synthetic NEFs already do and asserts a decided keeper, eleven
   reds, `undecided == 0`.
2. **`rounds == 3` was one request short.** `Verdict::rounds` counts comparative
   requests: three batches of four plus the final batch of three winners is four.
   The test pins the batch shapes `[4, 4, 4, 3]`, which is what makes the count
   checkable rather than asserted.
3. **The CLI test is `crates/imagecull-cli/tests/group_cap.rs`, not
   `groups_command.rs`.** That file's doc assigns sockets to `systemone_http.rs` and
   has no fixture of its own; the new test includes the core's `tests/support`
   through `#[path]` and adds `image` as a CLI dev-dependency rather than copying a
   NEF writer and a stub. Two cases: a cap of two costs three requests for four
   frames, and the same folder without the key costs one.

`keeper`'s prose was corrected with the code: its budget docs said `max` was a fact
about the model that a run could not move, which `[group] max_images` now
contradicts (ADR 0014 decided that line, and ADR 0018 / ticket 08 may want to say
so). The module docs and `Budget` now describe `max` as the measured default, the
way `EDGE` already reads.
