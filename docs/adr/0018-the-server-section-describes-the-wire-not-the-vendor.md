# 0018 — The `[server]` section describes the wire, not the vendor

Status: accepted · 2026-11-28 · Feature `cloudflare-clef` ·
amended by [ADR 0020](0020-the-cleanup-drops-doctor-and-the-measures-csv.md),
which removes `doctor` and the preflight ·
amended by [ADR 0019](0019-the-endpoint-is-the-whole-address.md), which retires
`path` and `selector` and makes `endpoint` the whole address ·
amends [ADR 0003](0003-toml-config-file-one-source-at-a-time.md)'s section map
(`[server]` grows five keys and gains a resolution rule of its own) ·
amends [ADR 0014](0014-the-comparison-has-its-own-preview-config.md)'s
"`Budget` keeps what is measured about the comparison's answer — `max` frames per
request" · amends `SPEC.md` §3, §7 and §11.1

## Context

imagecull knew one server. A local oMLX, keyless, on `127.0.0.1:8000`; the
decision path `/v1/systemone` was a `const &str`, the body's `model` was always
the alias, and `truncate: true` was written unconditionally. `[server]` held
`endpoint`, `model` and `timeout` and nothing about the request itself.

Cloudflare serves the same model family — Clef, the model this tool was built
around — on Workers AI, behind the same decision API. Same body, same answer, a
different address: the model id is the path (`…/ai/run/@cf/cloudflare/clef`), the
body carries a selector (`clef`) rather than the id, the request needs a bearer
token, the schema lists no `truncate`, and one request takes four images rather
than twenty. A photographer with no Apple Silicon to spare, or who wants the 27B
Clef rather than a 4-bit checkpoint, could not rate a shoot at all.

Every one of those is a fact about the **wire**, not about a company. The
temptation is a `provider = "cloudflare"` switch: it reads well in a config file
and puts a vendor's name and its quirks in the crate, where the next host
(Vultr, DeepInfra, an AI gateway) is a second branch, and the branch after that
is a table of every vendor's quirks inside a tool that is supposed to know about
pixels.

## Decision

**Four wire facts become `[server]` keys, and no code learns a vendor's name.**
`path` (a template, `{model}` interpolated verbatim and unencoded — a Workers AI
id contains `/` and `@`, and those *are* the address), `selector` (what the
body's `model` field says when it differs from the id), `token_env` and `token`
(the bearer credential), `truncate` (whether the body carries oMLX's field). Plus
`[group] max_images`, the comparison's batch cap. Every default reproduces
today's request byte for byte, so a file that says nothing still posts
`{endpoint}/v1/systemone` with `truncate: true`, no `Authorization` header, and
batches of twenty.

**The token is a `Secret`, resolved once from an injected environment lookup.**
`Effective::resolve_token(lookup)` takes a closure; the front end passes
`|name| std::env::var(name).ok()` and a test passes a map. `std::env::var` does
not live in `settle` because the tests run in threads: a test that set a process
variable would race every other test in the binary, which is why `Env` already
takes its paths as arguments. There is no `--token` flag, because a token on a
command line is in the shell's history. The environment wins over the file when
both are set, because a secret in a file that travels with a shoot folder is one
`git add` away from being published. A `token_env` that names a variable with
nothing behind it is fatal and names the variable — the alternative is an
anonymous request the server rejects 401 frames in a row for. `Secret` redacts
`Debug` and `Display`, has no `Deref`, no `Serialize` and no conversion into a
`String`, so the value cannot reach a sidecar, a ledger row, `--json-scores`,
`--dry-run` output or an error message by accident; `as_str` is the one way out
and the one call a review has to check.

**`max_images` caps the batch; it never refuses a group.** `keeper::chunk`
already splits a group larger than the cap into rounds and passes the winners
back through the same function, because oMLX's measured twenty was always a cap
rather than a limit of the algorithm. Four is the same problem with a smaller
number: a twelve-frame burst against Workers AI is three requests of four and one
decided group, not a 422. Like `cull_jobs` it is **scheduling**, so it does not
enter the grouping hash, and a config saying `4` where another says `20` still
produces comparable verdicts.

**The rubric hash does not gain the endpoint.** A rating's identity stays the
rubric's wording plus the model alias (spec §8.3), so every sidecar written
before this change still compares. The cost is named rather than hidden: two
servers serving the *same alias* (`clef-flash-oQ4e`) are indistinguishable to a
re-run, so moving a shoot from one machine to another behind the same alias
re-rates nothing. Cloudflare's ids are distinct strings, so the local-27B to
hosted move is a new hash and a full re-rate under `--force` — which is what the
photographer wants, since two models' stars are not comparable.

**`doctor` and `models` stay oMLX-only, and say so.** They read `/health`,
`/v1/models` and `/v1/models/status`, which no other SystemOne host serves;
asking for them where the model *is* the path would report a 404 as "nothing
loaded". So both refuse a config whose `path` is not oMLX's, naming the endpoint
and template they would have read, and make no request. The run's preflight is
**skipped, not faked**: there is no cheap "is this server alive" on Workers AI,
and spending a decision request per run to prove a token works is worse than the
401/403 rule, which costs one request and names the variable to check.

## Considered options

- **A `provider` key with a Cloudflare branch.** Rejected: it puts a vendor's
  name in the crate and makes the next host a second code path. A future preset
  is a UX layer that fills these five keys, never a branch under them.
- **Deriving the wire from the endpoint's hostname.** Fewer keys, and wrong: a
  gateway proxies Workers AI, an account id is in the path, and a tunnel serves
  oMLX from somewhere else. Hostname sniffing also fails silently the first time
  someone fronts oMLX with nginx.
- **A `--token` flag.** Fast to add, and it writes the token into shell history,
  the one place a photographer is least likely to look for a leak.
- **Reading `std::env` inside `settle`.** One call site fewer, and it makes the
  resolver a function of a machine's state — untestable in threads, and the
  reason `Env` takes arguments today.
- **Refusing a group larger than the cap.** Honest to the server's schema, but it
  charges the photographer for a fact about scheduling: `chunk` already answers
  the same question with more requests.
- **Putting the endpoint in the rubric hash.** It would make "same alias, other
  server" distinguishable, at the price of re-rating every existing shoot once
  for no change in judgement — and the alias is the string that actually names
  the weights.
- **Teaching `doctor` Workers AI's probes.** There are none to teach: no health
  endpoint, no listing, and a diagnosis invented from a decision request costs a
  token per run.

## Consequences

- `[server]` grows from three keys to eight, and its comment in
  `imagecull.toml.example` becomes the wire's four facts plus a timeout. ADR
  0003's "a section mirrors the core type it fills" is unchanged; what changed is
  that `[server]` now fills the wire as well as the address.
- `keeper::Budget`'s `max` is a **default a run may move**, not a measurement a
  run cannot touch — ADR 0014's line about `Budget` holding what is measured
  loses `max` and keeps the confidence floor.
- `imagecull.toml.example` ends with a Cloudflare recipe a photographer pastes
  over their account id, and a test parses that recipe, so a renamed key cannot
  rot it into a file imagecull refuses after an account id has been filled in.
- The README's "nothing leaves your machine" becomes "nothing leaves your
  machine by default", and its new section says which four facts differ, what
  `doctor` and `models` cannot do there, and that a different model id is a
  different rubric hash.
- `ErrorKind` grows `Http401` and `Http403`, both `Recovery::AbortRun`, because
  against a keyed service a refused credential is the most likely failure there
  is and `BadResponse`'s `Continue` would pay for it four hundred times.
- Two servers serving one alias are indistinguishable to a re-run. Recorded
  here so a future "which server rated this" question reopens the hash
  deliberately rather than by surprise.
