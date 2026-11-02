# 04: The adapter sends the request the config describes

Status: ready-for-agent
Type: task
Blocked by: 03
Spec: `.scratch/cloudflare-clef/spec.md`

**What to build:** `systemone::Adapter` stops using the constant `PATH` and the
constant `truncate: true`, and posts to the path and body the run's `[server]`
section describes.

## The work

`Adapter::new(client, model)` gains the wire config it needs — path template,
selector, truncate — or takes the `Effective`'s relevant fields as a small
`Wire` struct built in `cli.rs`. Keep it a struct of owned values: the adapter is
`Clone` and shared across the cull's workers.

```rust
pub struct Adapter {
    client: Client,
    model: String,          // the id: interpolated into the path, in the body by default
    path: String,           // the template, unresolved
    selector: String,       // the body's `model`
    truncate: bool,
}
```

- `path_for(&self) -> String`: `self.path.replace("{model}", &self.model)`. No
  escaping — Workers AI ids contain `/` and `@` and must go on the wire verbatim.
- Empty model with `{model}` in the template is fatal at construction, before any
  request: `the configured request path needs a model — pass --model or set
  [server] model` (a config can be legal and still describe nothing).
- `body()` drops `"truncate": true` when the config says `false`; the key is absent,
  not `false`.
- `body()` writes `"model": selector` instead of the model id. With `selector`
  defaulting to the model, oMLX's body is unchanged byte for byte.
- `failure()` stops printing the constant `PATH` and prints the path that was
  actually POSTed. `Error::message()` today reads `POST /v1/systemone returned HTTP
  400`; against Workers AI it must read `POST /@cf/cloudflare/clef-flash returned
  HTTP 400`, because that is the request the photographer has to reproduce with
  `curl`. Thread the path into `failure` (and into the 413 advice, whose text stays).
- `comparison_body` gets the same treatment: selector, no `truncate`.

## Tests

`crates/imagecull-core/tests/systemone_http.rs` (httpmock):

- `a_workers_ai_config_posts_to_the_model_path` — endpoint
  `https://api.cloudflare.com/client/v4/accounts/abc/ai/run`, path `/{model}`,
  model `@cf/cloudflare/clef-flash`, token set: the mock asserts
  `POST /client/v4/accounts/abc/ai/run/@cf/cloudflare/clef-flash`, the
  `authorization` header, body `model == "clef-flash"`, `truncate` **absent**, and
  `images` of length 1.
- `a_comparison_against_workers_ai_carries_no_truncate` — same for `compare`, four
  previews.
- `an_omlx_config_still_sends_truncate` — the existing body assertions, unmodified,
  are the proof; add the explicit `truncate == true` assertion if it is only implied.
- `a_model_in_the_path_with_no_model_is_refused_before_the_request` — the fatal above,
  with a mock that asserts zero requests.
- `a_400_names_the_path_it_posted` — the message contains
  `/client/v4/accounts/abc/ai/run/@cf/cloudflare/clef-flash`, not `/v1/systemone`.

## Notes

The `PATH` constant stays where the oMLX default lives (`systemone::PATH`), because
`config.rs` settles to it; nothing else reads it after this ticket.
