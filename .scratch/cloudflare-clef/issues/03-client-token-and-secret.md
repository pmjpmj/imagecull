# 03: The client carries a token and a path template; `Secret` redacts

Status: ready-for-agent
Type: task
Blocked by: 01
Spec: `.scratch/cloudflare-clef/spec.md`

**What to build:** `omlx::Client` sends `Authorization: Bearer …` when the run has a
token, and a `Secret` newtype makes leaking one impossible by accident.

## `Secret`

New module `crates/imagecull-core/src/secret.rs` (or a type in `config.rs` if it
stays a dozen lines):

```rust
pub struct Secret(String);

impl Secret {
    pub fn new(value: impl Into<String>) -> Secret;
    pub fn as_str(&self) -> &str;   // the only way out
}

impl fmt::Debug for Secret  { /* "Secret(<redacted>)" */ }
impl fmt::Display for Secret { /* same */ }
impl PartialEq for Secret    { /* by content, so tests can compare */ }
impl Eq for Secret {}
```

No `Serialize`, no `Deref`, no `From<Secret> for String`: every use is an explicit
`as_str()`, so a review can grep for the two call sites that could leak it.

## Resolution

`Resolved`/`Effective` keep `token_env: Option<String>` and, once this ticket lands,
`token: Option<Secret>`. Resolution is a pure function of an injected lookup, called
by the front end:

```rust
/// The bearer token this config asks for, if any: the environment variable it
/// names when that variable is set and non-empty, else the literal fallback.
pub fn resolve(lookup: impl Fn(&str) -> Option<String>) -> Result<Option<Secret>>
```

- Neither key set → `Ok(None)`: no header, today's behaviour.
- `token_env` set, variable set and non-empty → that value wins, even when `token`
  is also set.
- `token_env` set, variable unset or empty, `token` set → the literal.
- `token_env` set, variable unset or empty, no `token` → fatal:
  `CLOUDFLARE_API_TOKEN is not set and [server] token is empty — export the
  variable or write the key`. The message names the variable, never a value.

The front end passes `|name| std::env::var(name).ok()`; tests pass a closure over a
`HashMap`, because a test that sets a process variable races every other test in the
binary (the same reason `config::Env` exists).

## Client

`omlx::Client::new(endpoint, timeouts)` becomes `Client::new(endpoint, timeouts,
token: Option<Secret>)` — or a `Client::with_token(...)` builder if that keeps the
call sites in `cli.rs` and the tests readable. Whichever: the token is a field, not a
parameter threaded through `post_json`/`get`.

Both `get` and `post_json` attach `Authorization: Bearer {token}` when the field is
set. Attaching it to the GETs too is deliberate: an oMLX guarded with a key answers
`/v1/models` with 401 otherwise, and one code path is worth more than one header
saved on a run that has exactly one GET.

## Tests

- `secret.rs` unit tests: `format!("{:?}")` and `format!("{}")` never contain the
  value; `{:#?}` of a struct holding one does not either; `PartialEq` compares
  contents.
- `crates/imagecull-core/tests/systemone_http.rs`: a mock asserting the
  `authorization` header is absent when no token is configured (today's tests, plus
  one explicit assertion) and `Bearer xyz` when one is.
- A test that a `Client` with a token, printed with `{:#?}`, does not contain the
  token — the invariant, proven at the type rather than by reading the code.

## Done

Workspace tests green, clippy clean; no call site can print a token.
