# 01: Workspace scaffold, error taxonomy, `models` and `doctor`

**What to build:** A buildable Rust workspace with the scoring core in its own library crate and a thin CLI binary, so every later ticket has somewhere to land. On top of it, the two commands that answer "is my server even right?" — `imagecull models` lists the aliases the local oMLX is serving and flags which are decision models, and `imagecull doctor` reports reachability, server version, whether the configured alias exists, and whether it is being served as a decision model rather than a plain VLM. `--version` prints the binary version. A photographer whose oMLX is missing, too old, or serving a Clef checkpoint as a plain VLM gets one actionable sentence instead of discovering it as 400 identical HTTP 400s.

**Blocked by:** None (can start immediately)

**Status:** resolved

- [x] `cargo build` produces a workspace with a core library crate and a CLI binary; every dependency is pinned exactly and `Cargo.lock` is committed
- [x] The core crate defines the error taxonomy as types and never writes to stdout, never checks whether it is a TTY, and never calls process exit
- [x] `imagecull --version` prints the version
- [x] `imagecull models` lists the server's aliases and marks which are decision models
- [x] `imagecull doctor` distinguishes: server unreachable, server version older than the decision-model release, alias absent, alias served as a non-decision model — each with an actionable message and a fatal exit classification
- [x] `--endpoint` and `--model` are accepted by both commands
- [x] `raw-files` and the image crate's orientation API are the two upstream dependencies flagged in the spec as critical path; confirm at pin time that both compile on Apple Silicon and note the versions chosen

## Comments

Implemented. What landed, and how each box is met:

- **Workspace** — `Cargo.toml` (resolver 3, edition 2024) with `crates/imagecull-core` (lib) and `crates/imagecull-cli` (bin `imagecull`). Every direct dependency is pinned with `=` (`clap 4.6.7`, `serde 1.0.229`, `serde_json 1.0.154`, `ureq 3.5.0`, `raw-files 0.6.0`, `image 0.25.10`); `Cargo.lock` is tracked and `cargo build --locked` succeeds.
- **Error taxonomy** — `core::error`: `ErrorKind` is the nine failure statuses of spec §10 with `as_str()` (the CSV `status` vocabulary), `from_status_code()` and `recovery()` mapping the "action" column; `Error` carries `Option<ErrorKind>` (None = run-level fatal), optional `http_status`, and `ExitCode` holds 0/1/2/3. The crate denies `clippy::print_stdout`, `print_stderr`, `dbg_macro` and `clippy::exit`, so the no-stdout/no-exit rule is enforced, not just documented.
- **`--version`** — `imagecull 0.1.0` via clap's `version`. The rubric hash part of spec §11 lands with the rubric.
- **`imagecull models`** — `core::omlx::Client::catalog()` joins `GET /v1/models` (aliases) with `GET /v1/models/status` (`model_type`/`engine_type`), marking `decision` models with `*` and the configured alias. On a server without the status endpoint it prints `unknown` plus a line saying types are not reported and which oMLX version to upgrade to, rather than guessing.
- **`imagecull doctor`** — `core::doctor::diagnose()` returns a `Verdict` with one actionable sentence and a fatal exit classification: `Unreachable`, `TooOld` (version below `0.7.1.dev1`, or no `/v1/models/status` at all), `NoModelTypes` (claims a new version but reports no types), `AliasMissing` (lists what is served), `PlainModel` (reports the type it *is* served as), plus `NotReady` (503 while models load), `Unauthorized` (401/403) and `NotOmlx`. Version comparison (`core::version`) handles `0.7.1.dev1`-style pre-releases, and unknown version strings fall through to capability checks rather than guessing.
- **`--endpoint` / `--model`** — accepted by both commands, together with `--timeout` (read timeout, default 120 s as in spec §7.3).
- **Upstream pins confirmed on Apple Silicon** — `raw-files 0.6.0` and `image 0.25.10` both build and run on `aarch64-apple-darwin` (rustc 1.98.0). `crates/imagecull-core/tests/upstream_api.rs` keeps that honest: it applies `DynamicImage::apply_orientation` for all eight EXIF tags (asserting 5–8 transpose the axes — the portrait-frame regression) and calls `raw_files::RawFile::open`.

Tests: 36 green (`cargo test`), clippy clean. `crates/imagecull-core/tests/doctor_http.rs` drives the real `ureq` client against a hand-rolled stub server for every verdict above, including a 401 on `/v1/models/status`; the `imagecull-core` lib never touches stdout, so those paths are covered without capturing terminal output.

Verified by hand against stub servers: ready (`ok: oMLX 0.7.1.dev1 ... ready to score`, exit 0), old server (exit 2, names `0.7.1.dev1`), plain VLM alias (exit 2), absent alias (exit 2, lists served aliases), bad `--endpoint` (exit 2).

### Resolution

Merged to `feat/nef-scoring-v1`. The branch is green: `cargo test --locked` (204 tests), `cargo fmt --check` and `cargo clippy --all-targets --locked -- -D warnings`.

### Found later, on a real server (2026-11-01)

Pointing this at the oMLX on this machine showed the optional type endpoint
being treated as required: the server guards `GET /v1/models/status` with its API
key while `/v1/models` and `/v1/systemone` stay open, so `models` printed an
error, `doctor` called the server fatal, and a run's preflight refused to start —
against a server that then scored eleven frames. Fixed on
`feat/nef-scoring-v1`: the catalog refuses only when `/v1/models` fails, `doctor`
gained `Verdict::TypesUnavailable` (ready with a caveat, exit 0) and checks the
alias before the type report, `models` names the status code instead of blaming
the version, and preflight (`Report::can_score`) refuses only what the server has
proven unfit. `Unauthorized` now means what it says: the alias list itself is
behind the key.
