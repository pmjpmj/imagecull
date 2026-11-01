# 01: Config file — parse, discover, resolve

**What to build:** `crates/imagecull-core/src/config.rs`. The file as a wire
type (`ConfigFile`, `#[serde(deny_unknown_fields)]` on it and on every section),
the discovery order, validation, and the merge that turns "what the flags said"
plus "what the file said" into the effective `Config`, `Params`, `ScanOptions`
and `Outputs`. No terminal, no exit codes, no clap in this crate — the same rule
`cli.rs` already states, applied in the other direction.

Shape, matching `rubric.rs`'s existing loader:

```rust
pub struct ConfigFile {   // every section optional, every key optional
    server: Server,       // endpoint, model, timeout
    preview: Preview,     // max_edge, jpeg_quality
    scan: Scan,           // recursive
    outputs: Outputs,     // csv, json_scores, absolute
    run: Run,             // rubric, shoot_context, dedupe, csv_rewrite_every
}

impl ConfigFile {
    pub fn from_toml(text: &str) -> Result<ConfigFile>;      // strict, validated
    pub fn discover(explicit: Option<&Path>, env: &Env) -> Result<Option<Loaded>>;
    pub fn resolve(&self, base: Option<&Path>) -> Result<Resolved>;     // relative paths
}
```

`Env` is a three-field struct (`cwd`, `xdg_config_home`, `home`) that the front
end fills from the process environment. Discovery takes it as an argument rather
than reading `std::env` itself for one reason: `cargo test` runs tests in
threads, so a test that sets `XDG_CONFIG_HOME` to prove the order would race
every other test in the binary. With `Env` injected, the whole discovery order
is a pure function over paths and the tests stay parallel.

`Resolved` holds only the keys the file actually set, as `Option<T>`, so "the
file did not say" stays distinguishable from "the file said the default value" —
otherwise a file's `max_edge = 1568` and no `--max-edge` flag would have to be
told apart some other way.

**Tests first.** Unit tests over `from_toml` strings, in the style of
`crates/imagecull-core/tests/rubric.rs`:

- an empty file, and a file with only `[server]`, are both valid;
- every key at its documented value round-trips into `Resolved`;
- unknown key in a known section is fatal and names the key;
- unknown section is fatal and names the section;
- `jpeg_quality = 0` and `jpeg_quality = 101` are fatal;
- `max_edge = 0` is fatal; `csv_rewrite_every = 0` is fatal;
- `timeout` of 0 is fatal (a run that cannot wait cannot run);
- a file that is not TOML at all is fatal, with the parser's line and column in
  the message;
- `rubric = "mine.toml"` in a file at `/a/b/imagecull.toml` resolves to
  `/a/b/mine.toml`; the same for `csv` and `json_scores`; an absolute path is
  left alone;
- `discover` returns `--config`'s path when given and it exists; is fatal when
  given and missing (never falls back); prefers `./imagecull.toml` over the XDG
  path; honours `XDG_CONFIG_HOME` when set and falls back to `<home>/.config`
  when unset; returns `None` when nothing exists — each case built from a
  hand-made `Env`, with `tempfile` roots so nothing touches the real home;
- `include_str!("../../../imagecull.toml.example")` parses clean, so the
  committed example cannot rot into an invalid file;
- a merge test that a flag value beats a file value, and a file value beats the
  built-in default, for at least `max_edge`, `model` and `csv`.

**Error wording** follows `Rubric::load`: `the config file <path> is not usable:
<toml error>`, `cannot read the config file <path>: <io error>`, and for a
missing explicit target, `the config file <path> does not exist`.

**Blocked by:** —

**Status:** resolved

- [x] `ConfigFile` deserialises with `deny_unknown_fields` at every level
- [x] Out-of-range values are fatal, never clamped, with the range in the message
- [x] Discovery order implemented exactly, first hit wins whole, missing `--config` fatal
- [x] Discovery takes an injected `Env`, never reads `std::env` itself
- [x] `imagecull.toml.example` is parsed by a test, so it cannot rot
- [x] Relative paths in the file resolve against the file's directory
- [x] `Resolved` keeps "unset" distinct from "set to the default value"
- [x] Merge precedence unit-tested: flag > file > built-in default
- [x] `cargo test -p imagecull-core` green; no clap or `std::process` in `config.rs`

## Comments

Implemented, tests-first. `crates/imagecull-core/src/config.rs` plus
`crates/imagecull-core/tests/config.rs` (23 tests).

- **`ConfigFile`** is the wire type: five private section structs
  (`ServerSection`, `PreviewSection`, `ScanSection`, `OutputsSection`,
  `RunSection`), each `#[serde(default, deny_unknown_fields)]`, so every key and
  every section is optional and a typo is fatal. A toml unknown-field error
  already carries line, column and the list of fields that would have been
  accepted: `unknown field `max_edeg`, expected `max_edge` or `jpeg_quality``.
- **`Resolved`** is one source's values, all `Option`. `override_with` is the
  whole precedence rule (flags win) and `settle` applies the built-in defaults,
  returning `Effective`. The three-step shape is what keeps "unset" apart from
  "set to the default value".
- **`Env`** is injected, not read: `Env::of_process()` is the only place that
  touches `std::env`, and it drops a relative `XDG_CONFIG_HOME` as the XDG spec
  requires. Discovery is then a pure function of paths, so the six discovery
  tests run in parallel without racing on the process environment.
- **Validation** mirrors the flags' ranges (`max_edge >= 1`,
  `1 <= jpeg_quality <= 100`, `csv_rewrite_every >= 1`, `timeout >= 1`) and adds
  the two empty-string checks; nothing is clamped and every message names the
  section-qualified key (`preview.jpeg_quality must be between 1 and 100, not
  101`).
- **Paths** anchor to the config file's own directory, so a shoot config is
  relocatable with its folder.
- `Params`, `ScanOptions` and `Outputs` gained `PartialEq, Eq` so the effective
  config can be compared in tests. `scoring::Config` gained `source:
  Option<PathBuf>` for provenance, and `scoring::Run` and `progress::Summary`
  carry it — the same mechanism `endpoint` already uses.
- `include_str!("../../../imagecull.toml.example")` is parsed by a test, so the
  committed example cannot rot.

`cargo test -p imagecull-core` green (23 new tests), `cargo clippy
--workspace --all-targets` clean.
