# 09: Spike — one real Workers AI call settles the three unknowns

Status: ready-for-human
Type: research
Spec: `.scratch/cloudflare-clef/spec.md`

**What to find out:** the docs describe Clef's schema but not its HTTP envelope, its
tolerance for fields outside that schema, or its status for "too many images". One
`curl` against a real account answers all three, and the answers decide ticket 05
and confirm or delete `[server] truncate`.

**What is needed:** a Cloudflare account id and an API token with Workers AI access.
No code changes; record the outcome in the Answer section of this ticket.

## 1. The envelope, and whether `truncate` is tolerated

```sh
curl -sS -i "https://api.cloudflare.com/client/v4/accounts/$CLOUDFLARE_ACCOUNT_ID/ai/run/@cf/cloudflare/clef-flash" \
  -H "Authorization: Bearer $CLOUDFLARE_API_TOKEN" \
  -H "content-type: application/json" \
  -d '{
    "model": "clef-flash",
    "state": {"task": "photo_cull", "filename": "DSC_0421.NEF"},
    "truncate": true,
    "questions": {
      "stars": {"type": "choice", "instructions": "How good a photograph is this?",
                "criteria": {"1": "unsalvageable", "2": "weak", "3": "usable",
                             "4": "good", "5": "the one"}}
    }
  }'
```

Record:

- **Is `answers` at the top level, or under `result`?** (Paste the body.)
- **Does the extra `truncate` get a 200, a 400, or a 200 with a warning?** If 400,
  paste the message — the key `[server] truncate` stays and its doc comment gains it.
  If 200, note that the key can be deleted later.
- The status Line: does `usage` carry `input_tokens` only, or `latency_ms` too?

## 2. Images: a data URL, and five of them

Same request plus one `images: ["data:image/jpeg;base64,<small jpeg>"]`, then the same
with five copies of it. Record:

- Does a data URL work as-is (`data:image/jpeg;base64,...`), or does Workers AI want
  `{content_type, base64}`? Our previews are data URLs already; this decides whether
  `systemone::data_uri` is reusable unchanged on both servers.
- **What status does five images get — 400 or 422?** This measures the blast radius
  of forgetting `[group] max_images`: 422 is a client bug reported per group, 400 is
  the same. Either way a wrong `max_images` is a loud failure rather than a silent
  downgrade, and the ticket says so.

## 3. A 401, for the advice's wording

```sh
curl -sS -i "…same URL…" -H "Authorization: Bearer nonsense" -H "content-type: application/json" -d '{"model":"clef","state":"x","questions":{"a":{"type":"noul","instructions":"Is x true?"}}}'
```

Paste the body. Workers AI wraps errors as `{"success": false, "errors": [{code,
message}], "result": null}`; `systemone::detail` reads a `detail` field, so this is
what a photographer will actually see appended to `POST … returned HTTP 401:`. If the
message is unreadable there, that is a one-line change to `detail` and belongs in
ticket 06.

## Answer

_(to be filled in by whoever runs it: the three findings, plus any paste of a body
that contradicts `schema-input.json` / `schema-output.json`.)_
