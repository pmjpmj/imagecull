# 01: `[server]` gains `path`, `selector`, `token_env`, `token`, `truncate`

Status: ready-for-agent
Type: task
Spec: `.scratch/cloudflare-clef/spec.md`

**What to build:** `ServerSection` grows five optional keys, they reach `Resolved`
and `Effective`, and `ConfigFile::validate` refuses the shapes that cannot be a
request. Nothing outside `config.rs` changes in this ticket.

## The shape

```toml
[server]
endpoint = "https://api.cloudflare.com/client/v4/accounts/abc/ai/run"
path = "/{model}"
model = "@cf/cloudflare/clef-flash"
selector = "clef-flash"
token_env = "CLOUDFLARE_API_TOKEN"
token = "..."
truncate = false
```

- `path`: `Option<String>`, settled to `systemone::PATH` (`/v1/systemone`). Keep the
  default in one place — move `PATH`'s default out of `systemone.rs` only if it
  reads better from `config.rs`; do not write the literal twice.
- `selector`: `Option<String>`, settled to nothing (see `Effective::selector()`
  below), never to a copy of the model.
- `token_env`, `token`: `Option<String>`, carried through as written. Nothing
  resolves them here: `settle` must stay pure and thread-safe (see ticket 03).
- `truncate`: `Option<bool>`, settled to `true`.

`Effective` gains `path: String`, `selector: Option<String>`, `token_env: Option<String>`,
`token: Option<Secret>`, `truncate: bool` — with `Secret` arriving in ticket 03; if
this ticket lands first, use `Option<String>` for `token` here and change it in 03.
Project them in `override_with` too, so a flag layer that sets nothing keeps the
file's values (no flag sets these keys; the projection exists so a future one does
not silently drop them).

## Validation

| key | rule | message shape |
|---|---|---|
| `server.path` | non-empty, starts with `/` | `server.path must be a request path beginning with '/', not {x}` |
| `server.path` | every `{…}` placeholder is `{model}` | `server.path '{x}' has a placeholder that is not {{model}}` |
| `server.selector` | non-empty when set | `server.selector cannot be empty — delete the key to send the model instead` |
| `server.token` | non-empty when set | `server.token cannot be empty — delete the key or put the secret there` |
| `server.token_env` | non-empty, no `=` or whitespace | `server.token_env names an environment variable, not '{x}'` |

The "path needs a model" rule is *not* here: `path = "/{model}"` with no model is
legal in a config file and fatal at settle time, because `--model` can supply the
model for the run (ticket 04 owns the message).

## Tests

`crates/imagecull-core/tests/config.rs`:

- `EVERY_KEY` gains the five keys; `every_key_reaches_the_resolved_values` asserts
  each one arrives.
- `an_empty_source_settles_to_the_specs_defaults` asserts `path == "/v1/systemone"`,
  `truncate == true`, and `selector`/`token_env`/`token` empty.
- `a_value_the_flags_would_reject_is_fatal` gains the table above, one case each.
- `the_file_wins_over_the_built_in_default` for `path` and `truncate`.
- A new `a_server_nobody_named_still_posts_where_it_always_did` — an empty file's
  `Effective` posts exactly as `omlx::DEFAULT_ENDPOINT` + `systemone::PATH` did.

## Done

`cargo test -p imagecull-core --test config` green, `clippy` clean, no behaviour
change for a config that sets none of the new keys.

## Comments

**Implemented 2026-11-28.** One deviation from the shape above: the
`Effective::selector()` accessor is named `Effective::body_model()`. The settled
field `Effective::selector` and the accessor answer different questions — the
field is what a file wrote, the accessor is what the body's `model` should say —
and two things named `selector` with different values in one struct reads as a
bug at every call site. The fallback (selector when a file named one, the model
id whole otherwise) still lives in the accessor, so ticket 04 has one place to
read. Its `None` means no model is settled at all yet, which the doc says: the
rubric's own alias fills the field later.

A brace that never closes (`/v1/{model`) is not a placeholder — the rule above is
about `{…}` — so it is accepted here rather than refused with a message that
would read "has a placeholder that is not {model}" about a string that is one.
