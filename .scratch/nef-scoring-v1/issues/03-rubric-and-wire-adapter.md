# 03: The rubric and the wire adapter — one frame, six answers

**What to build:** The heart of the tool: a frame reaches the local Clef decision model as one request and comes back as six typed answers. The rubric is TOML so wording can be tuned without a rebuild — an embedded default ships in the binary and `--rubric` overrides it — holding four independent `score` questions (never combined, never rescaled), a five-way `stars` choice and a `dislike`/`neutral`/`like` `verdict` choice, with choice criteria as an ordered array so option order survives. The effective rubric is hashed over its canonicalised schema plus the model alias, and that hash is available to everything downstream. The request carries exactly one data URI, a non-null state (task framing, filename, the photographer's `--shoot-context`) and truncation on; requests are strictly sequential and bounded by a connect timeout and a configurable read timeout. The response is parsed defensively into an outcome: four full-precision expected values, an integer star rating, a verdict, per-answer confidences and usage numbers — with the raw answer document preserved, a missing or wrongly-typed answer classified as a bad response rather than guessed at, and probabilities treated as display-only. Everything above is observable through the single `Decider` seam, so tests assert on the exact bytes, dimensions and questions the pipeline sends.

**Blocked by:** 01, 02

**Status:** resolved

- [x] Scoring a single RAW yields all six answers for it, observable at the `Decider` seam and reported to the caller
- [x] Embedded default rubric plus `--rubric <FILE>` override; changing whitespace or key order does not change the rubric hash, changing any instruction, criterion or the model alias does
- [x] Four `score` questions with an ordered `0`–`10` legend; recorded values are the model's expected values at full precision, never rescaled, never combined
- [x] `stars` parses to an integer, `verdict` to one of the three keys
- [x] `overall` is framed around photojournalistic keepers, and `technical`, `composition` and `impact` instruct the model to ignore the other two axes
- [x] Request contains exactly one image, a non-null state including filename and shoot context, `truncate: true`, and the rubric's questions in declaration order
- [x] `--model` selects the alias and feeds the rubric hash; `--endpoint` selects the server
- [x] Requests are strictly sequential — never concurrent
- [x] Defensive parsing handles `probabilities` as an ordered array *and* as an object keyed by option, a float `score`, a string `choice`, and a missing answer key; the recorded answer is never derived from `probabilities`
- [x] Each answer's top-option confidence and `usage.input_tokens` are captured; server latency is used when present, client-measured otherwise
- [x] The raw answer document is retained for later `--json-scores`
- [x] Contract tests drive the adapter against a mocked server with canned and adversarial bodies; no network in CI

## Comments

Implemented. M2 is in: one frame becomes one `/v1/systemone` request and comes
back as six typed answers, and the rubric that asks them is a file a
photographer can edit.

- **Rubric** — `core::rubric`. The embedded default is
  `crates/imagecull-core/rubrics/default.toml`, `--rubric <FILE>` overrides it
  (`Rubric::load`). It asks exactly the six questions the CSV needs, in
  declaration order, and the loader rejects anything else as **fatal**: the four
  scores must be `score` questions offering `"0"`…`"10"` in order (the answer is
  the expected value over those indices, so a shifted legend would silently move
  every score), `stars` must offer `1`–`5`, `verdict` `dislike`/`neutral`/`like`,
  and unknown fields are refused so a misspelled `instrcutions` cannot silently
  vanish. Wording is draft v1 — `overall` is framed around photojournalistic
  keepers, and `technical`/`composition`/`impact` each tell the model which two
  axes belong to other questions; ticket 08 tunes and freezes it.
- **Hash** — SHA-256 over the *rendered* question schema plus the effective
  model alias. Because it hashes the parsed schema rather than the file,
  whitespace, comments, quoting style and table-vs-inline layout do not move it;
  an instruction, a criterion, an option's order, the alias, or how the schema
  is expressed to the model does. See `docs/adr/0001`.
- **Wire** — `core::systemone`. `body()` is the single place the request is
  assembled (crate-internal: the one seam is `Decider`): `model`, a non-null `state` (`task`: `photo_cull`,
  `filename`, `shoot_context` only when `--shoot-context` was given), exactly one
  `images` entry as a full `data:image/jpeg;base64,…` URI, `truncate: true`, and
  the questions in declaration order. `criteria` renders per type — the ordered
  legend for `score`, the option→description object for `choice` — because that
  is what the Clef heads read (`jundot/omlx#4315`); the authored TOML keeps one
  ordered-array form either way.
- **Adapter** — `Adapter` implements `Decider` over the existing `omlx::Client`
  (`POST` added to it, same timeouts, `send_json`). Statuses map into the
  taxonomy (400/413/422) with the server's own `detail` riding along in the
  message; anything else is `bad_response` with the status kept. A body that is
  not JSON is kept as the document (a JSON string) so `--json-scores` still has
  something to write.
- **Reading** — `core::reading`. `read()` is infallible: six answers or
  `bad_response`, and the raw document in both cases. `score` is read as a float
  at full precision (an integer is accepted), `choice` as the rubric's key, and
  confidence per answer; `probabilities` and `legend` are never read, so a
  document with only probabilities is a bad response rather than a guess.
  `usage.input_tokens` and `usage.latency_ms` come from the document;
  `Usage::latency_ms()` prefers the server's number and falls back to the client
  measurement.
- **Seam** — `core::scoring`. `Decider` takes `Request { model, rubric, state,
  preview }` and returns `Response { document, client_latency_ms }`;
  `scoring::run` scans, then judges every frame in ingest order, one at a time.
  `Frame.outcome` is `Result<Reading>`, so a prepared-frame or transport failure
  is an `Err` and an unreadable document is an `Ok` with `answers: Err` — which
  is what keeps the raw document attached to a `bad_response` row.
  `scoring::prepare` is the per-frame step (extract → orient → downscale →
  encode) and is now the only place previews are prepared.
- **CLI** — `score` gains `--endpoint`, `--model` (defaults to the rubric's
  alias), `--rubric`, `--shoot-context` and `--timeout`, and without `--dry-run`
  it runs the shoot and prints each frame's six answers plus a summary carrying
  the rubric hash and model. `--version` now prints the version, the embedded
  rubric's short hash and its alias.

Tests: 119 green (`cargo test`), clippy `-D warnings` and `cargo fmt --check`
clean.

Coverage lands at the seams the spec names: `tests/rubric.rs` (embedded shape,
hash stability rules, every rejection), `tests/systemone_wire.rs` (the exact
body — one image whose data URI decodes to the encoded bytes, the state, the
declaration order, the legend — and canned/adversarial documents read back),
`tests/systemone_http.rs` (a hand-rolled stub: canned and adversarial bodies,
400/413/422/5xx, timeout, unreachable, and a three-frame run whose deepest
observed overlap is 1), and `tests/scoring.rs` (fake `Decider`: questions, state,
alias, all eight orientations' worth of uprightness through the seam, the size
cap, no upscaling, per-file classification, sequential order, and `--limit`).
The `--dry-run`/real-run fold is pinned by comparing the written previews to the
bytes the decider received for the same frame and parameters.

Verified by hand against a Python stub: two synthetic NEFs scored (exit 0) with
the stub's log confirming one data URI per request, `truncate: true`, the
shoot context in `state`, and the six questions in declaration order;
`--model clef-27b-oQ4e` moved the hash; an unreachable endpoint exited 1 with
both rows marked `server_unreachable`; a missing folder and a broken rubric each
exited 2 with an actionable message; `--dry-run` wrote upright previews
(400x300 stored, orientation 6, 300x400 out); `--version` printed
`imagecull 0.1.0 (rubric 3b1c2d47, model clef-flash-oQ4e)`.

Three things to know:

- **`--dry-run` does not travel through `Decider`.** Ticket 02's review asked
  for the per-file step to be folded under the scoring entry point; it is —
  `dry_run` calls `scoring::prepare`, and a test asserts the written bytes equal
  the bytes the decider saw. What it does *not* do is masquerade as a decider: a
  decider that writes files and returns no answer would have to lie about the
  answer document (every row would read as `bad_response`), so the shared thing
  is the preparation, not the illusion.
- **Abort and retry are not here.** `server_unreachable` currently records a row
  per file and the run "completes", which is exactly what ticket 06 exists to
  stop: it owns preflight, three-attempt backoff, 5xx classification and the
  `Recovery::aborts_run` action that is declared in `error.rs` and unused here.
  A 5xx is classified `bad_response` with its status kept.
- **Everything here is against synthetic fixtures and a stub**, so the *wording*
  of the rubric is unvalidated in practice — that is M4's job (ticket 08), and
  the hash is what makes each tuning pass comparable.

Also fixed while here: `tests/doctor_http.rs`'s intermittent `Connection reset`
flake (noted in ticket 02) was the stub's listener being non-blocking — BSD
`accept` inherits the flag, so a handler could race the client's first bytes and
close the connection under it. One `set_nonblocking(false)` on the accepted
stream removes it; the new `/v1/systemone` stub does the same.

### Review round (two axes: standards, spec)

Both axes ran green on substance — all twelve checkboxes hold, nothing belongs to
a later ticket and nothing later was pre-empted except the one noted below — and
the cheap findings were fixed in the same ticket:

- **One HTTP stub, not two.** The new `/v1/systemone` stub and `doctor_http.rs`'s
  stub were the same accept loop, header parser, response writer and BSD
  non-blocking fix written twice. Both now use `tests/support/stub.rs`, which
  routes by path for `doctor` and scripts a sequence for the adapter, and which
  records every request and the deepest overlap it ever saw.
- **The request body is no longer a public function.** `systemone::body` is
  `pub(crate)`: the spec's rule is one seam (`Decider`), so the body assertions
  moved into `systemone_http.rs` and now read the body the real client actually
  sent, rather than calling the renderer behind the adapter's back. The
  odd file out — `systemone_wire.rs` — is now `tests/reading.rs`, which is all it
  tests.
- **Unused surface removed**: `Adapter::client()`, the duplicated id constants
  (the rubric now owns `SCORE_IDS`/`STARS_ID`/`VERDICT_ID`), `scoring::prepare`
  and `display_name` made crate-internal, and the unit-test modules in
  `rubric.rs`/`scoring.rs` (their claims are all pinned at the public seams) —
  along with `cli::score`'s test of `reading::read`, which belongs to core.
- **The dry-run summary shares `left_out()`** with the run summary instead of
  restating the three count lines.
- **The spec now points at ADR 0001**, so the authored form (`{key, description}`
  arrays) and the wire form (legend for `score`, option object for `choice`) no
  longer contradict each other in the contract text.

Three findings are recorded and deliberately not fixed here:

- **`confidence` is mandatory.** A document with six good answers and no
  `confidence` is a `bad_response` and the frame's scores are lost. That is the
  contract-strict reading of "missing or wrongly-typed answer": `confidence` is
  part of the answer, the raw document is preserved for diagnosis, and silently
  recording a score whose uncertainty is unknown would make the CSV's conf
  columns lie. If M4 shows the field drifting, the change is in
  `reading::confidence` and the CSV's empty-cell rule.
- **The rubric hash does not cover `state`.** `filename` and `--shoot-context`
  are part of what the model sees, so ticket 05's dedupe and resume keys must
  include the state; `docs/adr/0001` now says so, because
  `(preview_sha256, rubric_hash, model)` alone would reuse an outcome judged
  under different framing.
- **`score`'s per-frame report and summary pre-empt ticket 07.** They print
  unconditionally, with no TTY gate and no `--quiet`. That is the minimum for
  "reported to the caller"; ticket 07 replaces them with the `Reporter` seam
  rather than extending them.

### Resolution

Merged to `feat/nef-scoring-v1`. The branch is green: `cargo test --locked` (204 tests), `cargo fmt --check` and `cargo clippy --all-targets --locked -- -D warnings`.
