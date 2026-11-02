# 04: The adapter sends the request the config describes

Status: resolved (2026-11-02)
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

## Comments

**Implemented 2026-11-02.** `systemone::Wire`, `Adapter::new(client, model, wire)`,
`Adapter::path_for`, the selector and `truncate` in both bodies, and the posted path
in every failure row landed as written, with the five tests named above. Six notes:

1. **`Wire::selector` is an `Option<String>`, resolved at construction.** `cli.rs`
   fills it from `Effective::body_model()` — the config's selector, or the model the
   config names — and `Adapter::new` falls back to the model it was built for. That
   second fallback cannot move into `cli.rs`: a rating's model is the rubric's alias
   when nothing named one (`scoring::Config::new`), and only the command knows it.
   `Wire` is still a struct of owned values, so a cull's workers share it.
2. **`Adapter::new` returns `Result<Adapter>`.** The model-less path is refused at
   construction, before any request, so the sketch's plain `-> Adapter` could not
   hold. The message is the ticket's, byte for byte, including the em dash.
3. **`failure()` prints the endpoint-prefixed path, which reconciles the ticket's own
   two lines.** The design section shows `POST /@cf/cloudflare/clef-flash`; the Tests
   section requires `/client/v4/accounts/abc/ai/run/@cf/cloudflare/clef-flash`. The
   Tests section wins, because the row exists to be reproduced with `curl`, and the
   prefix is part of the request. `Endpoint::path_of` is the one new public method: an
   endpoint with no path of its own (oMLX, and every fixed-path SystemOne host) prints
   exactly what it printed before, `POST /v1/systemone`.
4. **The fatal fires on an empty settled model, not on an absent `[server] model`.**
   A `/{model}` path with no model in the file still runs against the rubric's alias
   (spec §8.3) or `omlx::DEFAULT_MODEL`, which is a legal configuration: a photographer
   may put the Worker's id in a custom rubric. Refusing that would refuse a legal
   setup, so the guard is the empty string the adapter is actually asked for.
5. **`body`/`comparison_body` are private methods sharing one `envelope`.** The two
   bodies differ only in their questions and images, so the envelope — model, state,
   images, `truncate` when the wire wants it, questions last — is one builder, and the
   `truncate` insertion keeps its original position. `serde_json` is built with
   `preserve_order`, so `an_omlx_config_still_sends_truncate` now also pins the key
   order: an oMLX body is byte for byte what it was.
6. **The front end builds the wire once.** `CommonArgs::wire` and
   `CommonArgs::adapter` are the only places a command turns an `Effective` into an
   `Adapter`, so `score` and `groups` cannot disagree about the three facts, and the
   contract tests are the only place that prove what the wire becomes on the socket.
