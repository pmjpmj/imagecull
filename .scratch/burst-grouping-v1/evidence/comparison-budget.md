# The comparative call's context budget — measured

Exploration for ticket 02 (`needs-info`: "the context budget of the comparative
call"). Server: `http://127.0.0.1:8000`, `clef-flash-oQ4e`, oMLX `/v1/systemone`.
Nothing tracked was edited; artefacts are in `.pi/group-pick/`.

## Answer in short

| | finding |
|---|---|
| **cap** | **16,384 image tokens, hard.** The count is not a frame limit but a token limit: `T(n) ≈ 160 + k·n` where `k` = per-preview cost. Max frames: **9 @1568 px, 23 @1024 px, 80 @512 px** — all three boundaries measured to the frame. |
| **over the cap** | **HTTP 413, nothing dropped.** `{"message": "the schema needs 19810 tokens before the state; the limit is 16384"}`. Images are *never* silently truncated, so a truncated 20-frame burst cannot produce a silently meaningless answer. `truncate: true` applies only to `state`. |
| **the answer shape** | `answers.<question-id>.{type, choice, confidence, probabilities}`; `choice` is a **string** key, `confidence` is always present. 413/422/400 are pre-inference and free (0.3 s). |
| **concurrency** | **serialized server-side, confirmed.** 2 concurrent calls → 5.11 s + 10.08 s; 3 → 5.12 / 10.27 / 15.20 s (a clean queue, one at a time). SPEC §7.1 is correct. |
| **sensible?** | Yes on a real difference (sharp vs. mushed frame: 0.98 / 0.96, follows the *image*, both orders). **No on a true burst near-tie**: on DSC_7956/57/58 the pick changes with preview size (512→frame 1, 1024→frame 1, 1568→frame 2) at confidence 0.70/0.55/0.51 — i.e. those frames are a coin flip. That is the finding this ticket actually has to reckon with. |

## 1. How many previews fit — the measurements

`sips -Z{1568,1024,512}` from `.pi/previews/DSC_795{6,7,8}.jpg`; the sample set is
padded by repeating copies. One `choice` question, keys `"1".."n"`, `truncate:
true`, ~160 tokens of `state` + instructions. Wall clock is one request, warm
server (`python3 .pi/group-pick/sweep.py <edges> <counts>`; raw rows in
`.pi/group-pick/sweep.jsonl`).

### 1568 px — ~465 KB/file, ~1639 tok/frame, ~5.7 s/frame

| n | input_tokens | secs | http |
|---|---|---|---|
| 1 | 1800 | 5.92 | 200 |
| 2 | 3439 | 11.20 | 200 |
| 4 | 6717 | 22.09 | 200 |
| 8 | 13273 | 45.42 | 200 |
| **9** | **14912** | **52.70** | **200 ← max** |
| 10 | — | 0.34 | **413** `schema needs 16526 tokens before the state; the limit is 16384` |
| 12 | — | 0.42 | 413 (19810) |
| 20 | — | 0.64 | 413 (32946) |

### 1024 px — ~170 KB/file, ~696 tok/frame, ~2.2 s/frame

| n | input_tokens | secs | http |
|---|---|---|---|
| 1 | 855 | 2.50 | 200 |
| 2 | 1549 | 4.57 | 200 |
| 4 | 2937 | 8.63 | 200 |
| 8 | 5713 | 17.58 | 200 |
| 12 | 8498 | 26.39 | 200 |
| 16 | 11286 | 36.02 | 200 |
| 20 | 14074 | 44.29 | 200 |
| 22 | 15468 | 49.64 | 200 |
| **23** | **16165** | **51.43** | **200 ← max** |
| 24 | — | 0.33 | **413** (16834) |
| 26 | — | 0.35 | 413 (18228) |

### 512 px — ~40 KB/file, ~201 tok/frame, ~0.58 s/frame

| n | input_tokens | secs | http |
|---|---|---|---|
| 1 | 359 | 1.02 | 200 (the existing per-frame shape) |
| 2 | 557 | 1.52 | 200 |
| 4 | 953 | 2.62 | 200 |
| 8 | 1745 | 4.91 | 200 |
| 12 | 2546 | 7.01 | 200 |
| 20 | 4154 | 11.52 | 200 |
| 40 | 8174 | 23.39 | 200 |
| **80** | **16214** | **48.72** | **200 ← max** |
| 81 | — | 0.31 | **413** (16387 — one token over) |
| 88 | — | 0.31 | 413 (17794) |

**Per-image cost.** 1639 tok @1568, 696 @1024, 201 @512 — 2.35× per size step,
i.e. tokens scale with *pixel area*, not bytes (a preview is not compressed
proportionally). Constant overhead ≈ 160 tok (state + instructions + criteria).

**Latency is prefill-bound and linear in tokens**, ~2.9–3.5 ms/token, so ~3 ms ×
tokens + ~1 s. Nothing here times out: the slowest request measured is 52.7 s,
inside the 120 s read timeout. A 20-frame burst at 1568 would have needed ~115 s
had it fit — it does not, and it 413s in 0.6 s instead.

**Largest (count × size) that answers correctly and cheaply:** 512 px × 20
frames = 4,154 tok, 11.5 s — 8% of the budget, and (see §2) still decisive on a
real quality difference. 1024 px × 16 = 11,286 tok, 36 s is the comfortable
"more detail" option; 1568 px × 9 = 14,912 tok, 53 s is the largest *detailed*
set and uses 91% of the budget with no room for state.

## 2. The exact request and response for "which of these is the keeper"

Working pair saved verbatim: `.pi/group-pick/request.json`,
`.pi/group-pick/response.json` (3 frames @1024; shape-only copy in
`request-shape.json`).

```json
POST http://127.0.0.1:8000/v1/systemone
{
  "model": "clef-flash-oQ4e",
  "state": { "task": "photo_cull", "filename": "burst-01",
             "shoot_context": "a burst of the same subject, choose the keeper" },
  "images": ["data:image/jpeg;base64,/9j/…", "data:image/jpeg;base64,/9j/…", "…"],
  "truncate": true,
  "questions": {
    "keeper": {
      "type": "choice",
      "instructions": "Several frames of one burst are shown, in order. Decide which single frame is the one to keep: the sharpest and the one whose subject, expression and light hold together best. Answer with the key of that frame.",
      "criteria": { "1": "image at position 1", "2": "image at position 2", "3": "image at position 3" }
    }
  }
}
```

```json
200
{ "model": "clef-flash-oQ4e",
  "answers": { "keeper": { "type": "choice", "choice": "1", "confidence": 0.5483,
                           "probabilities": { "1": 0.5483, "2": 0.427, "3": 0.0247 } } },
  "usage": { "input_tokens": 2243, "output_tokens": 0 } }
```

Corrections to the contract as written in the ticket/SPEC:

- **`questions` is a map keyed by question id**, not `{"question": [ … ]}` —
  `{"question": […]}` is 422 `Input should be a valid dictionary or object`.
- **`criteria` is a map `{key: description}`**, *not* an array of
  `{key, description}` **as the ticket's brief describes it**: the array form is
  400 `choice criteria must be a non-empty object`. This matches `rubric.rs:372-388`.
- **Images must be data URIs.** Bare base64 is 400: `Image inputs must be base64
  data URIs (data:image/...;base64,...)`.
- **`state` is required but free-form**: `{}` extra keys pass through unvalidated
  (`"task": "burst_keeper"`, an invented `"extra_field"` both 200). Deleting
  `state` is 422 `body -> state: Field required`.
- **`usage` has no `latency_ms`** on this server (SPEC §7.2 says "on the reference
  server") — only `input_tokens` and `output_tokens`, and `output_tokens` is
  always **0** for a decision model. Client-side timing is the only latency
  source for telemetry.
- **No `truncated` flag in the response.** Truncation is detectable only as
  `input_tokens == 16384` exactly.

`choice` is a string (`"1"`) ✔; `confidence` is present on every 200 ✔;
`probabilities` is an object keyed by option ✔ (parse-and-display only).

**Does it follow the image or the position?** Sharp-vs-mushed pairs (same frame,
second copy round-tripped through 128 px), both orders, at every edge:

| pair | choice | confidence |
|---|---|---|
| sharp, mush @512 | 1 (sharp) | 0.9804 |
| mush, sharp @512 | 2 (sharp) | 0.9761 |
| sharp, mush @1568 | 1 (sharp) | 0.9795 |
| mush, sharp @1568 | 2 (sharp) | 0.9638 |

The answer tracks the **image**, not the slot, and 512 px loses nothing on a
focus/detail call. On real burst frames, though, it is unstable:

| same 3 frames (7956, 7957, 7958) | choice | confidence | runner-up |
|---|---|---|---|
| 512 px | 1 = 7956 | 0.6955 | 0.264 |
| 1024 px | 1 = 7956 | 0.5483 | 0.427 |
| 1568 px | 2 = **7957** | 0.5148 | 0.454 |

and permuting three frames (all @1024) gave 7956, 7957, 7957, 7956 for
`abc`, `cba`, `bac`, `acb` — the winner is inside {7956, 7957} with the third
frame never winning, but the top two swap under a reorder and a 0.43-vs-0.55
margin. **The confidence field is the only thing separating "clef chose" from
"clef guessed"**, and on a genuine burst (near-identical frames) it is routinely
below 0.6.

## 3. Serialized or concurrent

`python3 .pi/group-pick/concurrency.py`, identical 8×512 payload:

| calls in flight | wall | per-call |
|---|---|---|
| 1 | 5.34 s | 5.34 |
| 2 | 10.08 s | 5.11, 10.08 |
| 3 | 15.20 s | 5.12, 10.27, 15.20 |

Strict FIFO, one at a time — SPEC §7.1 confirmed, no correction needed. The
client's serial design costs nothing, and a parallel group fan-out would buy
nothing either.

## 4. What the cap implies for a 20-frame burst

**It errors; it does not silently drop images.** Every over-budget request is a
413, pre-inference, in 0.3 s, with the exact token count in `detail`:

```
{"message": "the schema needs 32946 tokens before the state; the limit is 16384"}
```

Only `state` is truncatable. Measured: a 20,000-word `shoot_context` with one
1568 preview returns **200 with `input_tokens` pinned to 16384** (silently cut,
no flag) under `truncate: true`, and **413** `the request needs 20346 tokens`
under `truncate: false`. Default is `true` (absent key behaves like `true`).

So the failure mode the ticket feared — a 20-frame burst silently losing frames
and answering about a subset — **does not exist**. A group either gets a
whole-group answer or an error the run can report as undecided. The cap is
therefore a *hard* limit that must be respected by construction (chunking at 20,
or shortlisting), not a corruption risk.

## Recommendation: the cap

**Render the comparison's previews at 512 px and let up to 20 candidates into
one call** — 4,154 tokens (25% of the 16,384 budget), 11.5 s, and it is the
cheapest configuration that still tells a sharp frame from a soft one at 0.98
confidence. The context cap is not what binds at that point: 512 px does not 413
until 81 frames, so a 20-frame burst — the ticket's stated worst case — fits
with 4× headroom and needs **no** measure-based shortlist for context reasons.
Where it does bind is the existing preview, not the model: at the tool's current
`--max-edge` of 1568, one request holds only **9** frames (14,912 tok, 52.7 s,
91% of budget), so a 20-frame burst would need a second, smaller render or three
chunks. Two renders per burst frame is nearly free (an already-decoded image,
`preprocess` at 512), so pay it rather than capping at 9 or splitting a burst
into chunks that can never be compared across the split. If a shortlist is
wanted anyway, the honest reason to have one is **answer quality, not context**:
measured confidence decayed from 0.72 (3 candidates) to 0.35 (6 candidates) on
distinct frames, and on the real burst the pick flips between orderings at
0.55/0.43. That is a decision for the ticket (a `pickConf` floor below which the
group is reported *undecided* rather than marked, or a rule that only the top-2
narrowed by rating are compared), and it should not be papered over with a
smaller edge — 512 and 1568 give the same answer on a real difference.

## Files

- `sweep.jsonl` — every row of the three tables (append-only log).
- `request.json`, `response.json`, `request-shape.json` — the working pair.
- `swap/results.json` — the four order-permutation probes, full request+response.
- `probe.py`, `sweep.py`, `swap_orders.py`, `concurrency.py` — the harnesses.
- `img1568/ img1024/ img512/ swap/ distinct/` — inputs (regenerate with `sips`).

## Residual risks / open questions

- The padding trick repeats pixels, so `input_tokens` vs. count is exact but the
  *quality* at n>8 was measured on distinct crops only up to n=6; the answer
  decay at high n is measured but weakly supported (6 candidates, one rubric).
- Per-image tokens vary with image content (these samples are 1639/696/201; a
  different JPEG could be ±5%). A production cap must leave slack — do not cut
  it at 16000.
- `output_tokens: 0` and no `latency_ms` mean the run's own clock is the only
  latency telemetry available; SPEC §7.2 should stop expecting `latency_ms`.
- Whether a low-confidence comparison should mark 11 frames Red is a product
  decision this exploration cannot make; the numbers above say the 0.43-vs-0.55
  case is real and not rare.

---

## Where the harnesses live

`.pi/group-pick/` (gitignored, on the machine that made the measurement):
`sweep.py`, `probe.py`, `swap_orders.py`, `concurrency.py`, `sweep.jsonl` (every
row), `swap/results.json` (the four order permutations). Regenerate the inputs
with `sips -Z{1568,1024,512}` from `.pi/previews/`.
