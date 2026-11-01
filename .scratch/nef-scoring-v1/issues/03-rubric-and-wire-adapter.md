# 03: The rubric and the wire adapter — one frame, six answers

**What to build:** The heart of the tool: a frame reaches the local Clef decision model as one request and comes back as six typed answers. The rubric is TOML so wording can be tuned without a rebuild — an embedded default ships in the binary and `--rubric` overrides it — holding four independent `score` questions (never combined, never rescaled), a five-way `stars` choice and a `dislike`/`neutral`/`like` `verdict` choice, with choice criteria as an ordered array so option order survives. The effective rubric is hashed over its canonicalised schema plus the model alias, and that hash is available to everything downstream. The request carries exactly one data URI, a non-null state (task framing, filename, the photographer's `--shoot-context`) and truncation on; requests are strictly sequential and bounded by a connect timeout and a configurable read timeout. The response is parsed defensively into an outcome: four full-precision expected values, an integer star rating, a verdict, per-answer confidences and usage numbers — with the raw answer document preserved, a missing or wrongly-typed answer classified as a bad response rather than guessed at, and probabilities treated as display-only. Everything above is observable through the single `Decider` seam, so tests assert on the exact bytes, dimensions and questions the pipeline sends.

**Blocked by:** 01, 02

**Status:** ready-for-agent

- [ ] Scoring a single RAW yields all six answers for it, observable at the `Decider` seam and reported to the caller
- [ ] Embedded default rubric plus `--rubric <FILE>` override; changing whitespace or key order does not change the rubric hash, changing any instruction, criterion or the model alias does
- [ ] Four `score` questions with an ordered `0`–`10` legend; recorded values are the model's expected values at full precision, never rescaled, never combined
- [ ] `stars` parses to an integer, `verdict` to one of the three keys
- [ ] `overall` is framed around photojournalistic keepers, and `technical`, `composition` and `impact` instruct the model to ignore the other two axes
- [ ] Request contains exactly one image, a non-null state including filename and shoot context, `truncate: true`, and the rubric's questions in declaration order
- [ ] `--model` selects the alias and feeds the rubric hash; `--endpoint` selects the server
- [ ] Requests are strictly sequential — never concurrent
- [ ] Defensive parsing handles `probabilities` as an ordered array *and* as an object keyed by option, a float `score`, a string `choice`, and a missing answer key; the recorded answer is never derived from `probabilities`
- [ ] Each answer's top-option confidence and `usage.input_tokens` are captured; server latency is used when present, client-measured otherwise
- [ ] The raw answer document is retained for later `--json-scores`
- [ ] Contract tests drive the adapter against a mocked server with canned and adversarial bodies; no network in CI
