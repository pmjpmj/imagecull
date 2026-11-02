# 0019 — The endpoint is the whole address, and `model` is the only name

Status: accepted · 2026-12-01 · Feature `cloudflare-clef` ·
amends [ADR 0018](0018-the-server-section-describes-the-wire-not-the-vendor.md)'s
`[server]` key list (`path` and `selector` retire; `endpoint` absorbs one and
`model` the other) · amends
[ADR 0003](0003-toml-config-file-one-source-at-a-time.md)'s section map and its
"the wire has no flags" clause · amends `SPEC.md` §7 and §11.1

## Context

ADR 0018 gave `[server]` five wire keys so that a hosted SystemOne server needed
config rather than a second code path. Two of them described one address:

```toml
endpoint = "https://api.cloudflare.com/client/v4/accounts/<ID>/ai/run"
path = "/{model}"
model = "@cf/cloudflare/clef-flash"
selector = "clef-flash"
```

The split is by position — `endpoint` is what comes before, `path` is the rest —
and it does not survive being read aloud. A photographer writing a Workers AI
config hand-assembles a single address out of two keys, and the `@cf/cloudflare`
part appears twice in the file: once inside `model`, once implied by it. The
request itself is `endpoint + path`, joined in the client, and the failure row
has to reconstruct that join to print a path (`Endpoint::path_of`) so a failure
reads the same whether or not the endpoint carried a prefix.

`selector` existed for the same reason from the other side: on Workers AI the
body's `model` was the short name while the path carried the `@cf/…` id, so the
two could not be one key.

## Decision

**One address, one name.** `[server] endpoint` is the whole address a request is
posted to, and `[server] model` is one string used twice: the body's `model`, and
whatever `{model}` in the address is replaced with. `path` and `selector` are
retired.

```toml
# oMLX: no placeholder, the address is posted to as written
endpoint = "http://127.0.0.1:8000/v1/systemone"
model = "clef-oQ4e"

# Workers AI: the id's own prefix is in the address, the name is the name
endpoint = "https://api.cloudflare.com/client/v4/accounts/<ID>/ai/run/@cf/cloudflare/{model}"
model = "clef-flash"
```

Every default reproduces the request a file that says nothing already made:
`http://127.0.0.1:8000/v1/systemone`, `truncate: true`, no `Authorization`
header. The default address is defined as the oMLX origin constant plus the
decision-path constant, with a test that says so, so the default cannot drift
away from the path [`systemone::is_omlx`] recognises.

**The address is a string, never a parsed URL.** `{model}` is interpolated
verbatim and unencoded — a Workers AI name contains `/` and `@`, and those *are*
the address — so a URL type is exactly the wrong home for it. What is validated
is shape: an `http://` or `https://` scheme, a host, no placeholder but
`{model}`, no placeholder in the scheme or host (a model name is not a hostname).
The model name is refused if it carries whitespace or braces, because it is both
a path segment and a body value and neither may be guessed at.

**A path-less address is legal and means `/`.** `http://127.0.0.1:9000` is a
config the user may write, and it is posted to as written. It is not oMLX's
decision path, so `doctor` and `models` refuse it, and it is not silently
rewritten into `/v1/systemone`.

**`is_omlx` ends-with rather than equals.** An oMLX behind a prefix
(`https://box.local/omlx/v1/systemone`) was expressible before — the prefix lived
in `endpoint`, the path in `path` — and stays expressible now, because the
question is "does this address end in the decision path and carry no `{model}`".
The probe base is the address with that suffix stripped, so an oMLX behind a
prefix is probed behind its prefix (`…/omlx/health`), and a foreign address is
never probed at all.

**`--endpoint` takes exactly what the key takes**, `{model}` included: the flag
and the file describe the same value, and a rule that refused the placeholder in
one and not the other would be a sentence to remember rather than a shape.

**The retired keys are refused by name.** `server.path` and `server.selector`
are fields on the deserialized section for one reason: so a file copied from an
older README is told where the value went — `path` folds into `endpoint`,
`selector` becomes `model` with the id's prefix in the address — instead of
being told it has an unknown field.

## Considered options

- **Keep `endpoint` + `path`, and let `endpoint` carry a prefix** (the status
  quo, and what makes `path_of` necessary). Rejected: it is the same join spelled
  in two keys, and the photographer is the one doing it.
- **Merge into `url` and rename the flag.** Rejected: the flag is `--endpoint`
  in every muscle memory and every README, and the merged value is still an
  endpoint.
- **Append `/v1/systemone` to a path-less address.** It would preserve today's
  `--endpoint http://host:8000` habit, at the price of the key meaning two
  things and a gateway on a bare host silently posting to a path it never
  served. A path-less address is posted to as written instead.
- **Parse the address with a URL type and rebuild it per request.** Rejected for
  the reason ADR 0018 gives: `@cf/…` must reach the wire unencoded, and a parser
  that normalises is a parser that encodes.
- **Keep `selector` for a host whose path name and body name differ some other
  way.** Rejected as unexpressible-by-construction: no supported host needs it,
  and the shape of such a host is a second placeholder, not a second key.

## Consequences

- `[server]` drops from eight keys to six, and the wire is one string plus a
  body name.
- `Wire` loses the path and the selector, `Effective` loses `path` and
  `selector`, `Effective::body_model`'s fallback disappears (`model` *is* the
  body's value), `Endpoint::url` and `Endpoint::path_of` give way to
  `Endpoint::resolved` and a free `path_of` that strips scheme and host.
- `is_omlx`, `doctor` and `models` keep their meaning for a prefixed oMLX, and
  now also answer for a path-less address, which they refuse.
- One expressiveness loss, named rather than hidden: a host whose address
  segment and body name differ in a way the address cannot compose is no longer
  configurable. No supported host is such a host; a second placeholder is how it
  would come back.
- A Workers AI config's `model` becomes `clef-flash` where it was
  `@cf/cloudflare/clef-flash`, so the rubric hash of an existing Workers AI shoot
  changes once and that shoot re-rates under `--force`. oMLX configs are
  unchanged: their address carries no placeholder and their `model` was already
  the alias.
- SPEC §7 and §11.1, the README's recipe and its four-facts table, and the
  example's recipe are rewritten with it; the example's recipe is still parsed by
  a test, so a renamed key cannot rot it.
