# 07: `doctor`/`models` refuse a non-oMLX path; the run's preflight is skipped

Status: ready-for-agent
Type: task
Blocked by: 04
Spec: `.scratch/cloudflare-clef/spec.md`

**What to build:** the two oMLX-probing commands know when they have been pointed
somewhere else and say so, and `score`/`groups` stop running oMLX's diagnosis
against a server that has none of oMLX's endpoints.

## One predicate

`Effective` gains a method rather than a new constant:

```rust
/// Whether this run addresses oMLX's own decision path, the one whose probes
/// `doctor` and `models` read. A template with `{model}` in it is some other
/// server's addressing, and neither command can say anything about it.
pub fn asks_omlx(&self) -> bool { self.path == systemone::PATH }
```

It is the only place the question is asked; `cli.rs`, `doctor.rs` and `models.rs`
read it, and `systemone::Adapter::preflight` reads the same comparison (the adapter
holds the template, so it compares the template it was built with).

## `doctor` and `models`

Before building a client, when `!effective.asks_omlx()`:

```
error: doctor reads oMLX's own endpoints (/health, /v1/models, /v1/models/status),
       and this run posts to https://api.cloudflare.com/client/v4/accounts/abc/ai/run/{model}
       instead — there is nothing for it to check. imagecull score and imagecull groups
       work against any SystemOne server; only doctor and models are oMLX-only.
```

Requirements: the message names the endpoint *and* the template as configured (not
the interpolated path, which needs a model the command was not given), exits with
`ExitCode::Fatal`, and makes **no request**. Same shape for `models`, naming the
listing it would have read. `--dry-run` sizing is not involved: `doctor <FILE>` is
refused as a whole, because its file check renders "the preview a rating would send"
and that rating's server is not one it understands — the photographer can use
`score --dry-run` for the same bytes.

## The preflight

`Adapter::preflight` returns `Ok(())` when the adapter's path template is not
`systemone::PATH`, with a doc comment saying why: Workers AI has no `/health` and no
model listing suitable for a one-shot check, and spending a decision request per run
to prove the token works is worse than the 401 rule in ticket 06, which costs one.

`Adapter::preflight` keeps `doctor::diagnose` for the oMLX path, strictness and all.

## Tests

- `crates/imagecull-core/src/config.rs` (or `tests/config.rs`): `asks_omlx()` is true
  for the default and for an explicit `/v1/systemone`, false for `/{model}` and
  `/v1/decisions`.
- `crates/imagecull-cli/tests/` (a new `doctor_command.rs` if none exists): `doctor`
  with a Workers AI config exits fatal, prints the message above, and the mock
  asserts zero requests; likewise `models`.
- `crates/imagecull-core/tests/systemone_http.rs`: a `score`/`decide` run against a
  Workers AI config issues **no** `/health`, `/v1/models` or `/v1/models/status`
  request before its first decision — one mock asserting exactly the decision POST.
