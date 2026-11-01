# 01: Config — a `[group]` section that fills `group::Thresholds`

Status: resolved
Type: task

**What to build:** `ConfigFile` gains `group: GroupSection` with `gap`,
`similarity` and `exposure_stops`; `Resolved` carries the three as `Option<f64>`;
`Effective` carries a settled `Thresholds`.

## The shape

```toml
[group]
gap = 2.0
similarity = 0.90
exposure_stops = 2.0
```

- Every key optional, `#[serde(default, deny_unknown_fields)]`, like every other
  section.
- `resolve()` passes the numbers through untouched; there is no path to anchor.
- `settle()` builds `Thresholds` from `group::DEFAULT_GAP`,
  `group::DEFAULT_SIMILARITY`, `group::DEFAULT_EXPOSURE_STOPS` — the core's own
  constants, not a second copy.
- `override_with()` merges per key, so a flag that said nothing keeps the file's
  number.

## Validation

The same ranges the flags enforce, with the key named:

| key | rule | message |
|---|---|---|
| `group.gap` | `> 0` and finite | `group.gap must be a positive number of seconds, or a group could never hold two frames` |
| `group.similarity` | `0.0..=1.0` | `group.similarity is a cosine between 0 and 1, not {x}` |
| `group.exposure_stops` | `>= 0` and finite | `group.exposure_stops must be zero or more stops, not {x}` |

`nan` and `inf` are TOML floats, and each rule above rejects them; `gap = 2` (an
integer) is accepted as two seconds, because serde casts an integer to `f64` and
a photographer should not have to write `2.0`.

## Tests

`crates/imagecull-core/tests/config.rs`:

- `EVERY_KEY` gains the section; `every_key_reaches_the_resolved_values` asserts
  all three.
- `a_value_the_flags_would_reject_is_fatal` gains nine cases (0, negative, nan,
  inf per key where each applies).
- `the_file_wins_over_the_built_in_default` and
  `an_empty_source_settles_to_the_specs_defaults` assert the whole `Thresholds`.
- `a_groups_flag_wins_over_the_file` and
  `a_whole_number_is_a_threshold_like_any_other` are new.

## Answer

`ConfigFile.group: GroupSection`, `Resolved.group_gap`/`group_similarity`/
`group_exposure_stops`, `Effective.thresholds: Thresholds`. `Resolved`,
`Effective`, `ConfigFile` and `Loaded` drop `Eq` (they now hold `f64`);
`PartialEq` is all the merge and the tests use. Validation lives in
`ConfigFile::validate` beside every other range check. `cargo test
-p imagecull-core --test config` green, 30 tests.
