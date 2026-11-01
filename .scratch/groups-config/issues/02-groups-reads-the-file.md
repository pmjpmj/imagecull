# 02: CLI — `groups` resolves through the config file

Status: resolved
Type: task

**What to build:** `GroupsArgs` reports its own overridable flags as `Resolved`
and builds `group::Options` from the settled `Effective`, so the file's
`[scan] recursive` and `[group]` numbers reach the pass.

## The wiring

```rust
// before: two sources, and the file's recursion never seen
pub fn overrides(&self) -> Resolved { self.common.overrides() }
pub fn options(&self) -> group::Options {
    ScanOptions { recursive: self.recursive.unwrap_or(false), .. }
}

// after: one source of truth
pub fn overrides(&self) -> Resolved {
    Resolved {
        recursive: self.recursive,
        group_gap: self.group_gap,
        group_similarity: self.group_similarity,
        group_exposure_stops: self.group_exposure_stops,
        ..self.common.overrides()
    }
}
pub fn options(&self, effective: &Effective) -> group::Options
```

`groups::run` already resolves the effective config before it does anything else,
so `options(&effective)` needs no reordering: it was built after the `effective`
binding and called with `&effective`.

`--limit`, `--jobs` and `--out` stay the flags they are: they describe the
invocation.

## Tests

`crates/imagecull-cli/src/cli.rs` (unit):

- `groups_needs_a_directory_and_defaults_to_the_core_thresholds` and
  `groups_takes_its_own_thresholds_and_scan_flags` go through a
  `groups_options(&args)` helper that settles an empty file first.
- `groups_takes_its_grouping_keys_from_the_config_file` — a `ConfigFile` with
  `[server] model`, `[scan] recursive` and `[group]`, merged under the flags,
  reaches `group::Options` and `Effective.model`.
- `a_groups_flag_beats_the_file_it_overrides` — `--group-gap` and
  `--recursive false` win; a key no flag touched keeps the file's number.

`crates/imagecull-cli/tests/groups_command.rs`:
`a_shoot_configs_scan_section_reaches_the_groups_scan` writes a shoot config with
`recursive = true` and a frame in `sub/`, runs the real binary with that folder
as the working directory, and asserts the CSV lists the frame; the same run with
`--no-config` does not, which is what proves the file did it.

## Answer

`GroupsArgs::overrides()` carries `recursive` and the three thresholds;
`GroupsArgs::options(&Effective)` reads `effective.recursive` and
`effective.thresholds` and keeps `limit`/`jobs` from the flags. `groups::run` now
calls `args.options(&effective)`. `cargo test -p imagecull-cli` green, including
the new integration test.
