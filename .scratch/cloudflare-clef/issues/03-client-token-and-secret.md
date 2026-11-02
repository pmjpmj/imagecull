# 03: The client carries a token and a path template; `Secret` redacts

Status: resolved (2026-11-02) — the token is a `Secret`, resolved through an
injected lookup, and the client attaches it to every request
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

## Comments

**Implemented 2026-11-02.** `secret::Secret`, `Effective::resolve`, the token
field on both `Resolved` and `Effective`, and the header on both the GETs and the
POST landed as written. Five notes:

1. **`resolve_token` hangs off `Effective`, and is not called `resolve`.** The
   ticket's sketch shows it with no receiver; it reads `token_env` and `token`, so
   it needs the settled config to read them from. The name grew the suffix because
   `ConfigFile::resolve` and `Loaded::resolve` already mean "anchor paths into
   `Resolved`", which is a different job — two `resolve`s one screen apart with
   different meanings read as a bug. `cli.rs`'s `CommonArgs::client` is the only
   caller and passes `|name| std::env::var(name).ok()`.
2. **One constructor, not a builder.** `Client::new(endpoint, timeouts, token)`
   takes the `Option<Secret>` directly: six call sites, one of which carries a
   token, and a `with_token` builder would have meant two constructors to keep in
   step for no readable gain.
3. **The variable's value is trimmed, and a whitespace-only one counts as
   empty.** A config refuses a blank `[server] token` by trimming, so a
   `token_env` whose variable is `"   "` is refused the same way, and one holding
   `"  abc  "` sends `Bearer abc` rather than `Bearer   abc  ` and 401s per frame.
   The literal is left as written: a value in a file is one its author can read.
4. **`Effective::token` is a `Secret` too**, as the ticket asked, so the field can
   no longer be printed by a diagnostic even before it reaches the client. A test
   proves `{:#?}` of the whole effective config does not contain it.
5. **The stub records the `authorization` header** (`tests/support/stub.rs`),
   which is what lets the absent-header assertion be explicit rather than implied
   by the request still working. `a_token_rides_on_the_get_probes_too` covers the
   rule that the GETs carry it as well.
6. **The boundary has its own test** (`crates/imagecull-cli/tests/token_env.rs`).
   Every core test injects a lookup, so all of them would stay green if `cli.rs`
   stopped reading `std::env` — the one-line regression that makes every
   `token_env` config fatal. Two cases through the real binary: the exported value
   is what the request carries, and the variable's absence is a fatal exit with no
   request at all.
