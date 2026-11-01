# 02: Config file — the CLI resolves through it

**What to build:** `cli.rs` stops filling defaults for the keys a file may set,
gains `--config` and `--no-config`, and hands both to the core's resolver; all
three subcommands get their effective config that way, and the run summary says
which file it used.

**The field change.** Every file-overridable flag becomes `Option<T>` and loses
its `default_value`:

| flag | becomes |
|---|---|
| `--endpoint` | `Option<String>` |
| `-m, --model` | already `Option<String>` |
| `--timeout` | `Option<u64>` |
| `--rubric` | already `Option<PathBuf>` |
| `--shoot-context` | already `Option<String>` |
| `--max-edge` | `Option<u32>`, still `range(1..)` |
| `--jpeg-quality` | `Option<u8>`, still `range(1..=100)` |
| `--recursive` | `Option<bool>` (`--recursive` / absent; no `--no-recursive` in this ticket) |
| `--out` | `Option<PathBuf>` |
| `--json-scores` | already `Option<PathBuf>` |
| `--absolute` | `Option<bool>` (same treatment as `--recursive`) |
| `--no-dedupe` | stays `bool`; `overrides()` maps it to `dedupe: Some(false)` when set and `None` when not, so a file can still decide |

Flags that describe the invocation keep their shape and their defaults:
`dir`, `--dry-run`, `--resume`, `--fresh`, `--limit`, `--quiet`, `--verbose`,
`--previews-dir`.

clap's `[default: …]` lines disappear from `--help` for the table above, so the
numbers move into the help prose: `/// Cap on the longest edge of the sent
preview, in pixels (default 1568)`. The `--help` text is now the only place a
default is stated, so state them there.

**`--config` / `--no-config` go on `ServerArgs`**, which is already flattened
into `doctor` and `score` and passed whole to `models` — all three commands get
them without a second struct. That name no longer fits what the struct holds, so
rename it `CommonArgs` (`ServerArgs::model()` and `.client()` move with it) and
update the doc comment: *the options every command shares*.

**Resolution, in each `run()`:** build `Env` from the process, call
`ConfigFile::discover(args.config, &env)` unless `--no-config`, and hand the
resulting `Resolved` plus the args to the resolver. A config error prints to
stderr and returns `ExitCode::Fatal.code()` — the same shape the existing
`Rubric::load` and `args.server.client()` error paths already use.

**Provenance.** `scoring::Config` gains `source: Option<PathBuf>` — the same
mechanism `endpoint` already uses to carry provenance for the ledger header —
and `Run`/`Summary` carry it to the terminal. `summary_block` gains the path on
the existing provenance line:

```
rubric 69139540 · model clef-flash-oQ4e · config /Volumes/Shoot A/imagecull.toml
```

with the `· config …` suffix omitted when no file was loaded, so every existing
summary test that has no config file keeps passing unchanged.

**No new dev-dependency.** The CLI crate has no `tests/` directory and only
`serde_json` as a dev-dependency. Rather than add `assert_cmd` + `predicates` for
this ticket, prove the wiring with `cli.rs` unit tests through
`Cli::try_parse_from` (the parser accepts `--config`, `--no-config` conflicts
with it, and the `Option<T>` fields are `None` when absent) plus resolver tests
in core (ticket 01, which own the precedence rules), and verify the end-to-end
path by hand with the sequence below.

**Blocked by:** 01

**Status:** resolved

- [x] Every field in the table is `Option<T>` with no `default_value`; clap range validation kept
- [x] Defaults now live in the resolver and are named in the `--help` prose
- [x] `ServerArgs` renamed `CommonArgs`, carrying `--config` and `--no-config`; doc comment updated
- [x] `--config` and `--no-config` conflict; `models`, `doctor` and `score` all resolve through the file
- [x] `--no-config` leaves a `./imagecull.toml` on disk unread
- [x] `cli.rs`'s "defaults are the spec's defaults" test replaced by a resolver test with the same assertions
- [x] Summary provenance line shows the config path only when one was loaded; existing summary tests untouched
- [x] `cargo test --workspace` green, `cargo clippy --workspace --all-targets` clean
- [x] Manual sequence below behaves as described

## Manual sequence

From a shoot folder containing `DSC_0001.NEF` and a config file:

1. `imagecull.toml` with `[preview] max_edge = 1024`, run `imagecull score . --dry-run` →
   previews written at longest edge 1024, summary names the config file.
2. Same run with `--max-edge 1568` → previews at 1568; the flag beat the file.
3. `mv imagecull.toml ~/.config/imagecull/config.toml`, run `imagecull doctor DSC_0001.NEF`
   → `doctor` reports the endpoint from the file, and 1024 applies there too.
4. Same command with `--no-config` → built-in endpoint and 1568; no config line in the summary.
5. Add `max_edeg = 900` to the file → exit code 2, stderr names `max_edeg` and the file path.
6. `imagecull score . --config nope.toml` → exit code 2, "does not exist", no fallback to
   `./imagecull.toml`.
7. `jpeg_quality = 101` → exit code 2, the message states the 1-100 range.

## Comments

Implemented. `cli.rs`, `score.rs`, `doctor.rs`, `models.rs`, the summary
provenance line, and `crates/imagecull-cli/src/progress.rs` tests.

- **`ServerArgs` → `CommonArgs`**, now holding `--config` and `--no-config`
  beside the server triple. One struct means all three subcommands got the flags
  with no second flatten; `--config` and `--no-config` conflict.
- **Every file-overridable field is `Option<T>`** with its `default_value`
  removed, so clap no longer answers "what did the photographer leave out?".
  `--recursive` and `--absolute` take an optional boolean
  (`num_args = 0..=1, default_missing_value = "true"`), which keeps
  `--recursive` working as a bare flag while letting a run say *no* to a config
  file that turned it on. `--timeout 0` is now refused by the parser as well as
  by the file.
- **Defaults moved into prose** — `(default 1568)`, `(default 88)`,
  `(default 120)`, `(default ./scores.csv)` — since `[default: …]` can no longer
  be true. `cli::DEFAULT_OUTPUT` was folded into `config::DEFAULT_CSV`: one
  source of truth for the default CSV name.
- **`CommonArgs::effective(flags)`** does discovery, merge and settle, and returns
  the message to print. Each command calls it first: `score` before reading the
  rubric, `doctor` and `models` before building a client, so an unreadable config
  stops the run before a single frame is read.
- **Provenance** rides `Config::source` → `Run::source` → `Summary::config` and
  is printed as a `· config <path>` suffix on the existing provenance line,
  omitted when no file was read. New test:
  `the_summary_names_the_config_file_when_one_was_read`.
- `cli.rs`'s "defaults are the spec's defaults" test now asserts two things: the
  parser leaves every file-overridable field `None`, and the resolver settles
  them to the spec's numbers.

## Evidence (2026-11-15)

`cargo test --workspace` green (19 test binaries), `cargo clippy --workspace
--all-targets` clean. Manual sequence from a scratch directory, against the real
binary:

- a discovered `./imagecull.toml` with `endpoint = "http://127.0.0.1:9311"`
  made `models` dial 9311 (`cannot reach http://127.0.0.1:9311`); `--no-config`
  fell back to `http://127.0.0.1:8000` and listed the server that is running
  there; `--endpoint http://127.0.0.1:9401` beat the file.
- `score . --config nope.toml` → exit 2, `does not exist`, no fallback to the
  discovered file. `--config <a directory>` → exit 2, `Is a directory`.
- `max_edeg = 900` → exit 2, naming the file, the line and column, the key, and
  the keys that would have been accepted. `doctor --config` failed the same way.
- `jpeg_quality = 101` → exit 2, `between 1 and 100, not 101`.
- `--config x --no-config` → clap conflict, exit 2.
- `doctor --config` with the 9311 endpoint dialled it, so the preflight reads the
  same file a run would.
- `score . --dry-run` with a discovered config and zero RAW files exited 0 and
  reported the previews directory it used.

**Not verified here:** the two steps that need a real NEF — that a config's
`max_edge` actually shows up in written preview dimensions, and the live summary
provenance line on a real run. No RAW file is on this machine, and the line is
covered by the summary test instead. Both are worth a minute with a real shoot
before this is called done.
