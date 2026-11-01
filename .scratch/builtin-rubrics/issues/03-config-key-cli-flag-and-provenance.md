# 03: The config key, the flag, and the name in provenance

**What to build:** `--rubric <NAME|FILE>` on `score` (parsed by clap with
`RubricSource::parse`, so a blank value is refused before a frame is read) and
`[run] rubric = "general" | "press" | "holiday" | "mine.toml"` in the config
file, both resolving into the one `Resolved.rubric`/`Effective.rubric` field, so
a flag beats a file with no exception for this key. A relative path is anchored
to the config file's directory; a built-in name is not anchored.

The run summary's provenance line and the `--version` line print the name
beside the hash — `rubric general/69139540` — when a name selected the rubric,
and the hash alone when a path did.

**Status:** resolved
**Blocked by:** 02

- [x] `ScoreArgs::rubric: Option<RubricSource>` with a `value_parser`, help text
      naming the three built-ins
- [x] `[run] rubric` deserialised through the same parser; blank is fatal
- [x] `score` calls `Rubric::select`; `scoring::Config`/`Run`/`Summary` carry
      `Option<Builtin>`
- [x] `rubric_name` renders `name/hash` or `hash` for the start line, the
      summary block, `--quiet`'s outcome line and `--version`
- [x] `imagecull.toml.example` documents the key

## Answer

`crates/imagecull-cli/src/cli.rs`, `score.rs`, `progress.rs`;
`crates/imagecull-core/src/config.rs`, `scoring.rs`, `progress.rs`.
`tests/config.rs` covers a name, a path, anchoring and the blank file value;
`cli.rs` covers `--rubric holiday`, `--rubric General` being a path and a blank
refusal; `progress.rs` covers both provenance spellings.
