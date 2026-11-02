# 08: Docs — the Cloudflare recipe, and the two places this changes the spec

Status: ready-for-agent
Type: task
Blocked by: 01, 02, 03, 04, 05, 06, 07
Spec: `.scratch/cloudflare-clef/spec.md`

**What to build:** a photographer can set up Workers AI from the files in the repo,
and the repo's own documents stop claiming a fact this change alters.

## `imagecull.toml.example`

- A new block under `[server]`, commented like its neighbours: `path`, `selector`,
  `token_env`, `token`, `truncate`, each with its default and the one sentence that
  makes the default make sense. `token` gets a warning line: **a secret in a file
  that travels with a shoot folder is one `git add` away from being published; the
  variable is the default for a reason.**
- A copy-paste recipe at the bottom of the file, in a comment block, with
  `<ACCOUNT_ID>` and the two model ids spelled out:

  ```toml
  # Rating a shoot with Cloudflare's hosted Clef (Workers AI)
  # export CLOUDFLARE_API_TOKEN=...      # Workers AI:Read is enough
  [server]
  endpoint = "https://api.cloudflare.com/client/v4/accounts/<ACCOUNT_ID>/ai/run"
  path = "/{model}"
  model = "@cf/cloudflare/clef-flash"
  selector = "clef-flash"
  token_env = "CLOUDFLARE_API_TOKEN"
  truncate = false
  [score]
  max_edge = 1024      # Workers AI resizes past ~1 MP, so 1568 buys nothing
  [group]
  max_images = 4       # Workers AI's per-request cap; a bigger burst is split
  ```

- `[group] max_images` documented beside `cull_jobs`, with the "scheduling, not
  algorithm, so it does not enter the grouping hash" line.

## `README.md`

A short section — "A server somewhere else" — with the recipe, the four facts that
differ from oMLX (path, `model` selector, token, 4 images), what `doctor`/`models`
cannot do there, and the one sentence that matters for a photographer switching
mid-shoot: **a different model id is a different rubric hash, so the frames already
rated are re-rated under `--force`; frames are never mixed under one hash.**

## `SPEC.md`

- §3 Non-goals currently says "No cloud calls, no telemetry, no server management".
  Amend to the fact: no cloud calls *by default*; a configured `[server] endpoint`
  may be a service, and the tool then sends previews to it, which is the
  photographer's decision and the reason the token lives in the environment.
- §7 ("Clef over oMLX — the wire contract") retitles to the wire contract *both*
  servers speak, keeps oMLX as the reference, and gains a short table of the four
  differences plus the Workers AI URL. §7.1's `truncate: true` line gains "an oMLX
  request field; `[server] truncate = false` for servers whose schema does not list
  it".

## `docs/adr/0018-the-server-section-describes-the-wire-not-the-vendor.md`

Follow `docs/adr/` and the `domain-modeling` skill's ADR format. Decides, with
alternatives and consequences:

- **No `provider` key.** Four wire facts as config, no vendor name in the crate; a
  preset would be a UX layer over them, never a second code path.
- **The token is a `Secret` resolved from an injected environment lookup.** Why not
  `std::env` in `settle` (threaded tests), why not a flag (shell history), why the
  env wins over the file.
- **`max_images` caps the batch, never refuses the group.** `keeper::chunk` already
  splits; the cap is scheduling.
- **The rubric hash does not gain the endpoint.** Every existing sidecar keeps
  comparing; the cost (two servers serving one alias are indistinguishable) is named.
- **`doctor`/`models` stay oMLX-only and say so.** No second diagnosis to invent.

## `GLOSSARY.md`

Two terms, in the style of their neighbours (one-sentence definition, `_Avoid:_`
line):

- **Server**: the HTTP service a run asks for one rating per request — a local oMLX
  by default, and any host speaking the same decision API when `[server]` names one.
  _Avoid_: backend, provider, model host, cloud.
- **Token**: the bearer credential a configured server may demand, read from the
  environment variable `[server] token_env` names and never written anywhere a run
  writes. _Avoid_: API key, secret, password.

If `domain-modeling` prefers one of these spelled differently, follow it, and update
the spec's wording to match.

## Done

A reader with an account id and a token can copy the example, export the variable,
and run without opening the source. `cargo test --workspace` unaffected.
