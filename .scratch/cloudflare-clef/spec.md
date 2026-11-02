# Spec — Cloudflare's Clef over Workers AI, as one more SystemOne server

Status: ready-for-agent
Date: 2026-11-28
Source: `SPEC.md` §7, `docs/adr/0003-toml-config-file-one-source-at-a-time.md`,
`GLOSSARY.md`, `imagecull.toml.example`, and the Workers AI Clef docs
(`developers.cloudflare.com/workers-ai/models/clef/`).

---

## Problem Statement

imagecull can only talk to one server: a local oMLX on `127.0.0.1:8000`, keyless,
with `/v1/systemone` hard-coded as the request path and `truncate: true` hard-coded
in the body. Every fact the client needs about the wire — where to POST, what the
body's `model` field should say, whether the endpoint wants a bearer token — is
either a constant or a `const &str` in `crates/imagecull-core/src/systemone.rs`.

Cloudflare hosts Clef itself, as `@cf/cloudflare/clef` and `@cf/cloudflare/clef-flash`
on Workers AI, behind the same SystemOne decision API. It is the same model family,
the same request shape, and the same answer shape — but not the same addressing:

| | local oMLX | Cloudflare Workers AI |
|---|---|---|
| request URL | `{endpoint}/v1/systemone` | `{endpoint}/{model-id}`, e.g. `…/accounts/<id>/ai/run/@cf/cloudflare/clef` |
| body `model` | the alias, `clef-flash-oQ4e` | a selector, `clef` or `clef-flash` (not the id) |
| auth | none | `Authorization: Bearer <API token>` |
| `truncate: true` | an oMLX request field | not in Workers AI's schema |
| images per request | measured budget of 20 | `maxItems: 4` |
| `GET /health`, `/v1/models`, `/v1/models/status` | oMLX's own probes | absent |

So a photographer who has no Apple Silicon to spare, or who wants the 27B Clef
rather than a 4-bit local checkpoint, cannot rate a shoot at all. The answer is not
a `provider` switch with a Cloudflare branch inside it: it is the four wire facts
above becoming configurables, so that oMLX, Workers AI and any other SystemOne host
(Vultr, DeepInfra) are the same code path with different config.

## Solution

`[server]` stops meaning "the local oMLX" and starts meaning "the server this run
talks to". Four new optional keys and one `[group]` key, all plumbing — none of them
knows Cloudflare's name:

```toml
[server]
endpoint = "https://api.cloudflare.com/client/v4/accounts/<ACCOUNT_ID>/ai/run"
path = "/{model}"                       # default: "/v1/systemone"
model = "@cf/cloudflare/clef-flash"
selector = "clef-flash"                 # default: the model, whole
token_env = "CLOUDFLARE_API_TOKEN"      # default: no token source
truncate = false                        # default: true

[score]
max_edge = 1024                         # Workers AI resizes past ~1 MP anyway

[group]
max_images = 4                          # default: keeper::MAX (20)
```

The default row of every key keeps today's behaviour byte for byte: a file that says
nothing still POSTs `http://127.0.0.1:8000/v1/systemone` with `truncate: true`, no
`Authorization` header, and batches of twenty.

`score` and `groups` honour all of it. `doctor` and `models` do not — they read
oMLX's own probes, which no other server has — so they refuse a config whose path is
not oMLX's rather than inventing a diagnosis, and the run's preflight stops asking
oMLX's questions when the run is not pointed at oMLX.

## User Stories

1. As a photographer without spare Apple Silicon, I want to rate a shoot with
   Cloudflare's hosted Clef, so I can cull at all.
2. As a photographer who started on a local 4-bit checkpoint, I want to switch to
   `@cf/cloudflare/clef` by editing the shoot's config, so the frames already rated
   stay rated and the rest are rated by the bigger model — with the two runs
   distinguishable in the sidecars, never mixed.
3. As a photographer holding an API token, I want it read from the environment, so
   it never sits in a config file that travels with a shoot folder and gets
   committed with it.
4. As a photographer whose token expired, I want the run to stop at the first 401
   and tell me which variable to check, instead of paying for 400 identical failures.
5. As a photographer culling a burst of twelve, I want the cull to compare them in
   batches of four instead of failing the group, so Cloudflare's per-request image
   cap costs me nothing but a few more requests.
6. As a photographer running `doctor` against a Workers AI URL, I want it to say
   plainly that `doctor` speaks oMLX only, so I do not read its silence as a verdict.
7. As a photographer reading `imagecull.toml.example`, I want a Cloudflare recipe I
   can paste and fill in with an account id, so the setup is one file and not a
   scavenger hunt in the docs.

## The config surface

Every key is optional, `#[serde(default, deny_unknown_fields)]`, validated by
`ConfigFile::validate` like every other range check, and absent from the layers a
flag already owns (`--endpoint`, `--model` beat the file; `path`, `selector`,
`token_env`, `token`, `truncate` and `max_images` have no flag — they describe the
server and the shoot, not the invocation).

### `[server] path` — the request path, as a template

A string beginning with `/`, interpolated once per request: `{model}` becomes the
run's model id, verbatim and unencoded — Workers AI ids contain `/` and `@`, and
encoding them breaks the URL that is being addressed.

| value | what it addresses |
|---|---|
| `/v1/systemone` (default) | oMLX, and every other SystemOne host with a fixed path |
| `/{model}` | Workers AI, where the model id *is* the path |

A template with a placeholder other than `{model}`, or one that does not start with
`/`, is fatal and names itself. A template containing `{model}` while no model is
configured is fatal too: it says the path needs a model rather than POSTing to a
path with a brace in it.

### `[server] selector` — the body's `model` value

The request body keeps a `model` field, and on Workers AI it is the selector
(`clef`), not the id (`@cf/cloudflare/clef`). Defaulting to the model keeps oMLX
(.alias), Vultr (`clef`) and DeepInfra (`Cloudflare/clef`) unchanged; Workers AI is
the one server today that needs the key.

### `[server] token_env` and `[server] token` — the bearer credential

`token_env` names the environment variable; `token` is the literal fallback. The
environment wins when both are set, because a secret in a file that travels with a
shoot folder is the one the photographer regrets. A `token_env` that is set but empty
in the environment, with no `token` beside it, is fatal and names the variable — a
typo'd variable must not become an anonymous request that the server then rejects
401 frames in a row for.

Resolution happens through an injected lookup, not `std::env::var` inside the
config: `Env` already uses that discipline for paths, and for the same reason — the
tests run in threads. The front end passes `std::env`, a test passes a closure.

The token is a `Secret`: `Debug` and `Display` redact it, and it never reaches a
sidecar, the ledger header, `--json-scores`, `--dry-run` output or an error message.
A test proves it: a client built with a token, printed with `{:#?}`, does not contain
it.

### `[server] truncate` — whether the body carries `truncate: true`

An oMLX request field (`SPEC.md` §7.1) that Workers AI's schema does not list.
Default `true`, so every existing config sends what it sends today; a Workers AI
config sets `false`. If the spike below shows Workers AI ignores unknown fields, this
key is a wart that can be deleted later, not a mistake now.

### `[group] max_images` — the comparison batch cap

The most previews one comparative request carries, filling `keeper::Budget::max`
(the constant `keeper::MAX` today, measured at 20 on oMLX). Valid 2..=20: a request
that carries one frame is not a comparison, and 20 is the measured budget the rest
of `keeper` is written against.

This is scheduling, like `cull_jobs`: it does not enter the grouping hash, and a
config that says `4` where another says `20` still produces comparable verdicts,
because `keeper::chunk` already splits any group into rounds and passes the winners
back through the same function. A twelve-frame burst against Workers AI becomes three
requests of four and one decided group — not a 422.

## The wire, as Cloudflare states it

Facts from `developers.cloudflare.com/workers-ai/models/clef/` and its
`schema-input.json` / `schema-output.json`, which the tickets cite rather than
re-derive:

- `POST https://api.cloudflare.com/client/v4/accounts/{account_id}/ai/run/@cf/cloudflare/clef`
  with `Authorization: Bearer <token>`.
- Body: `model` (required, `^\s*(clef|clef-flash)\s*$`), `state` (required, any JSON —
  our object is fine), `questions` (map of 1–64; `noul`/`choice`/`score` — the rubric
  already emits exactly these), `images` (0–4; a data URL starting `data:`, or
  `{content_type, base64}` — ours are data URLs already).
- No `truncate` in the schema; no `/health`, `/v1/models` or `/v1/models/status`.
- Answer: `{model, answers: {id: {type, choice|noul|score, probabilities, confidence}},
  usage: {input_tokens, output_tokens}}` — which `reading::read` already parses,
  `usage.input_tokens` included.

Two facts the docs do not settle, and a spike does: whether the HTTP body is that
object at the top level or wrapped in `{"result": …}` like other Workers AI models,
and what a request with five images answers (400 or 422). Tickets 04 and 05 are
written to read either envelope and to classify either status.

## Decisions taken

1. **No `provider` key, no vendor branch.** The four wire facts are config; nothing
   in the crate learns the string `cloudflare`. A future `[server] provider` preset
   is a UX layer over these keys, never a second code path.
2. **The token is a `Secret` from the boundary inward**, resolved once at settle
   time from an injected environment lookup, redacted in `Debug`, never written.
3. **The oMLX probes stay oMLX-only.** `doctor` and `models` refuse a path that is
   not `/v1/systemone` with a message that names the path and says what they can
   check, instead of guessing at a server they have no endpoint for.
4. **The run's preflight is skipped, not faked.** When the path is not oMLX's,
   `Adapter::preflight` returns `Ok(())`: there is no cheap "is this server alive"
   probe on Workers AI, and inventing a probe request would cost a token per run.
   The 401/403 rule below is what replaces it.
5. **401 and 403 abort the run.** They arrive today as `BadResponse`, which is
   `Recovery::Continue` — 400 rows of the same failure for the price of 400 requests.
   Two new kinds, `Http401` and `Http403`, both `Recovery::AbortRun`, both advising
   the token source (`[server] token_env`), is the honest fix and it costs one match
   arm.
6. **`max_images` is the batch cap, never a refusal.** A group larger than the cap is
   split, not reported undecided — `keeper::chunk` already does the splitting for
   oMLX's 20, and Cloudflare's 4 is the same problem with a smaller number.
7. **The rubric hash does not gain the endpoint.** A rating's identity stays the
   wording plus the model alias, so every sidecar written before this change still
   compares. The cost is stated in the docs: two servers serving the *same alias*
   (`clef-flash-oQ4e`) are indistinguishable to a re-run, and Cloudflare's ids are
   distinct strings, so the local-27B-to-hosted move is a new hash and a full re-rate
   — which is what the photographer wants anyway.

## Non-goals

- No `provider` presets, no model-id translation table, no Cloudflare account lookup.
- No AI Gateway support (a gateway URL is just another `endpoint` + header, for a
  later spec).
- No `--token` flag: a token on a command line is in the shell's history.
- No change to `doctor`'s and `models`' reach, error taxonomy beyond 401/403, or the
  sidecar format.
- No retry-on-429 tuning: 429 keeps its current classification (unmapped, so
  `BadResponse`), and is worth a follow-up only once someone has hit it.

## Acceptance

- `cargo test --workspace` green, and a Workers AI config in `imagecull.toml.example`
  that a photographer can paste.
- A file that sets none of the new keys produces byte-identical requests to today's
  (proven by the existing body/contract tests, unmodified).
- A Workers AI config produces: `POST {endpoint}/@cf/cloudflare/clef-flash`,
  `Authorization: Bearer …`, body `model: "clef-flash"`, no `truncate`, one image per
  submission, four per comparison — proven by `httpmock` contract tests.
- `imagecull doctor` against a Workers AI config exits fatal with a message naming
  the path, without a request.
- A 401 from the server aborts the run after one request, and the row says which
  variable to check.

## Tickets

| # | Ticket | Blocked by |
|---|---|---|
| 01 | `[server]` gains `path`, `selector`, `token_env`, `token`, `truncate` | — |
| 02 | `[group] max_images` fills `keeper::Budget::max` | — |
| 03 | The client carries a token and a path template; `Secret` redacts | 01 |
| 04 | The adapter sends the request the config describes | 03 |
| 05 | Reading accepts either answer envelope | 04, 09 |
| 06 | 401 and 403 abort the run | — |
| 07 | `doctor`/`models` refuse a non-oMLX path; the preflight is skipped | 04 |
| 08 | Docs: example recipe, README, `SPEC.md` §3/§7, ADR 0018, `GLOSSARY.md` | 01–07 |
| 09 | Spike: one real Workers AI call settles envelope, `truncate`, 5-image status | — |
