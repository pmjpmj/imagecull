# 01: Config — a `[score]` section, and three tombstones for the sections it replaces

Status: resolved
Type: task

**What to build:** `ConfigFile` gains `score: ScoreSection`; `[preview]`,
`[outputs]` and `[run]` become tombstones that refuse by name; `Resolved` and
`Effective` gain the `score_` prefix on the pair that is a rating's.

## The shape

```toml
[score]
max_edge = 1568
jpeg_quality = 88
json_scores = "scores.json"
rubric = "holiday"
shoot_context = "night street, pushed two stops"
dedupe = true
```

- Every key optional, `#[serde(default, deny_unknown_fields)]`.
- `rubric` deserialises through `rubric_source`, the same reader `--rubric` uses.
- `resolve()` anchors `json_scores` and a file rubric to the config's directory,
  as before; `outputs: Outputs` still carries only `json_scores`.

## The tombstones

One `LegacySection` per retired section, each holding the fields that used to
live there so a set key is parsed rather than reported as an unknown one, and a
key that was never there (a typo, `csv_rewrite_every`) still fails as unknown.

| old key | message |
|---|---|
| `preview.max_edge` | `preview.max_edge has moved to score.max_edge: [score] holds what a rating needs, the way [group] holds what a cull needs — move the key` |
| `preview.jpeg_quality` | same shape, `…score.jpeg_quality…` |
| `outputs.json_scores` | same shape, `…score.json_scores…` |
| `run.rubric` | same shape, `…score.rubric…` |
| `run.shoot_context` | same shape, `…score.shoot_context…` |
| `run.dedupe` | same shape, `…score.dedupe…` |
| `outputs.csv` | unchanged: `outputs.csv is gone: … write each rating into an XMP sidecar …` |
| `outputs.absolute` | unchanged: `outputs.absolute is gone with the report …` |

The tombstone check runs before every other validation, so a file with a retired
section is told to move a key rather than told its `[preview] jpeg_quality` is
out of range.

## Renames

- `Resolved.max_edge` → `score_max_edge`, `Resolved.jpeg_quality` →
  `score_jpeg_quality` (beside `group_max_edge` / `group_jpeg_quality`).
- `Effective.params` → `score_params` (beside `group_params`).
- `Resolved` fields keep their order: score keys first, then group keys.

## Wiring

No command's behaviour changes, only the names it reads through:

- `cli::ScoreArgs::overrides` and `cli::DoctorArgs::overrides` fill the renamed
  fields; `groups` still fills only the `group_` pair and `recursive`.
- `score.rs` and `doctor.rs` pass `effective.score_params`.
- `groups.rs`'s comment says a rating's `[score]` pair never reaches a cull.

## Validation

| key | rule | message |
|---|---|---|
| `score.max_edge` | `>= 1` | `score.max_edge must be at least 1 px, or nothing would be sent` |
| `score.jpeg_quality` | `1..=100` | `score.jpeg_quality must be between 1 and 100, not {x}` |

## Tests

`crates/imagecull-core/tests/config.rs`:

- `EVERY_KEY` moves the six keys under `[score]`; every assertion reads the
  renamed fields.
- New `the_retired_sections_name_the_key_that_replaced_them`: one case per moved
  key, asserting both halves of the message.
- `the_report_keys_are_refused_with_an_explanation_rather_than_a_typo_message`
  keeps its `csv`/`absolute` cases.
- `an_unknown_key_is_fatal_and_names_itself` and the range cases read `[score]`.
- `the_comparison_has_its_own_preview_pair` asserts `score_params` and
  `group_params` never merge.

`crates/imagecull-cli/src/cli.rs`: the `[preview]`-named expectations become
`[score]`, and the `effective.params` / `effective.score_params` split is proven
for `score`, `doctor` and `groups` alike.

## Answer

`ConfigFile` is `server`, `scan`, `score`, `group` plus three tombstones;
`ScoreSection` carries all six keys through `rubric_source` for `rubric`.
`Resolved` gained `score_max_edge` / `score_jpeg_quality` and `Effective`
`score_params`; the CLI's only change is the field names its `overrides()` fill
and the pair `score.rs` / `doctor.rs` read. Ranges and messages moved to
`score.*`.

`refuse_the_retired_sections` runs first in `validate`, so a half-migrated file
is told to move a key rather than that the number it is moving is out of range;
`csv` and `absolute` keep their own refusals. Tombstones hold `toml::Value` so
any value at a moved key is refused as a move rather than as a type error, while
`deny_unknown_fields` keeps a key that was never in the section
(`[run] csv_rewrite_every`) failing as an unknown field.

New tests: `the_retired_sections_name_the_key_that_replaced_them` (one case per
moved key) and `a_retired_key_is_refused_before_its_value_is_judged`. `EVERY_KEY`
and every `[preview]`/`[run]`/`[outputs]` fixture moved to `[score]`. `cargo
test --workspace --locked` green (35 config tests), `cargo clippy --workspace
--all-targets --locked -- -D warnings` clean, and the built binary prints
`preview.max_edge has moved to score.max_edge …` for an old shoot config.
