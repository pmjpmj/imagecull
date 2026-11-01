# Ticket 08 evidence — a real shoot through the built tool

Status: **evidence gathered, decision outstanding.** Ticket 08 is `ready-for-human`
and stays that way: the frame-by-frame judgement and the rubric freeze are the
photographer's, not an agent's. This file is what they need to make that call.

Date: 2026-11-01 · Branch: `feat/nef-scoring-v1` · Binary: `target/debug/imagecull`
built from that branch.

## The run

| | |
|---|---|
| Shoot | `~/Downloads/import-251221` — 11 NEFs + 10 JPEGs (family snapshots: a child in Christmas pyjamas, a parent at a laptop, a card on a desk) |
| Command | `imagecull score <DIR> --fresh --out scores.csv --json-scores answers.json` |
| Endpoint | `http://127.0.0.1:8000` (the oMLX on this machine) |
| Model | `clef-flash-oQ4e` (the Flash 9B checkpoint the spec defaults to) |
| Rubric | embedded draft v1, hash `69139540` |
| Result | 11 of 11 scored, status `ok` on every row, 2m44s (mean 9.6 s/frame — this server is slower than the spec's 0.5–1.5 s budget, which changes nothing but the wall clock) |
| Left out | 10 skipped (the JPEG sidecars), 0 ignored, 0 hidden directories |

Artifacts in this directory:

- `scores.csv` — the run's CSV, exactly as the tool wrote it.
- `review-sheet.md` — every frame with its six answers, sorted by `overall`, and
  blank `your stars` / `note` columns for the photographer's own judgement.

Outside the repo (too large to commit, regenerable — see "Regenerating" below):

- `../.pi/evidence/review-sheet-A.html` and `review-sheet-B.html` — the same
  sheets as one self-contained page with the exact preview bytes the model saw,
  for side-by-side comparison with your own rating.
- `.pi/evidence/previews/` — the 11 `--dry-run` previews.
- `.pi/evidence/scores-v2.csv`, `rubric-v2.toml` — the wording experiment below.
- `.pi/evidence/scores.ledger.jsonl`, `answers.json` — the ledger and the raw
  answer documents from the run (not committed: they carry the full documents).

## Acceptance evidence (spec §Testing Decisions, acceptance 1–5)

- **Acceptance 1 — a long run completes with a status on every row.** Every one
  of the 11 rows carries `ok`; the synthetic 400-file case is pinned in
  `crates/imagecull-core/tests/failure_taxonomy.rs`.
- **Acceptance 2 — cancel then resume equals one uninterrupted run.** Pinned in
  `tests/ledger.rs`; exercised by hand here as well (a `SIGINT` mid-shoot left a
  usable CSV and an intact ledger, and `--resume` finished the folder with
  `resumed: 11` and no re-requests).
- **Acceptance 3 — two full runs produce the same CSV.** Two `--fresh` runs over
  the same 11 NEFs, compared cell by cell: **0 differences apart from
  `latency_ms`**, which is the run's own measurement (the claim and its
  amendment are in spec §Determinism; the test is
  `two_full_runs_over_the_same_folder_agree_apart_from_the_latency_they_measured`).
  Same scores, same confidences, same tokens, same hash, same order.
- **Acceptance 4 — `--dry-run` previews are upright for portrait frames.** Both
  portrait NEFs in this shoot come out 1045×1568, upright, at the 1568 cap.
- **Acceptance 5 — no writes into the shoot folder.** An `mtime`+size scan of
  all 21 files before and after a run is byte-identical (`.pi/evidence/mtimes-{before,after}.txt`).

## Saturation and collapse — the numbers the freeze turns on

`overall` (0–10) and `stars`, 11 frames, draft v1 rubric (`69139540`):

| | overall | technical | composition | impact |
|---|---|---|---|---|
| mean | 5.67 | 5.31 | 5.64 | 5.80 |
| sd | 0.67 | 0.41 | 0.49 | 0.86 |
| range | 2.43 | 1.39 | 1.92 | 3.19 |

- **Stars are saturated.** 10 of 11 frames are 3★, one is 4★, and there is no
  1★, 2★ or 5★ in the whole shoot. The star rating carries almost no signal on
  this set.
- **Three of the four axes are collapsed.** Pearson r over the 11 frames:
  `overall`~`composition` 0.94, `overall`~`impact` 0.97,
  `composition`~`impact` 0.94. Only `technical` stands apart
  (`overall`~`technical` 0.74).
- **Confidence is low and flat**: mean top-option probability for `overall` is
  0.23 (an 11-way head answering at chance is 0.09, so the distribution is
  barely peaked). The model is not confident about these judgements.
- **`stars` and `overall` never disagree (0 of 11)** — but the metric cannot
  fire here: with `overall` ≈ 5.7 everywhere, the implied band is 3★, and
  everything is 3★. The 0% is a consequence of saturation, not evidence of
  agreement. This is why the baseline below should be read as provisional.

The ordering that *does* emerge is credible: the frame the model ranks highest
is the one with the child's full, genuine laugh (6.64, 4★, `like`), and the
lowest is the photograph of a Christmas card lying on a desk (4.21, `dislike`).
Level discrimination is the problem; ordering is not obviously wrong.

## One wording experiment (an iteration, not a proposal)

The hypothesis: all four score questions share one generic legend ("competent —
solid and unremarkable" …), so the model has no reason to give the axes
different numbers. Variant B gives each axis a legend written in that axis's own
terms and tells every question to use the whole range. Same checkpoint, same
frames, same day.

| | draft v1 `69139540` | variant B `5773b2c1` |
|---|---|---|
| `technical` sd / range | 0.41 / 1.39 | 0.50 / 1.41 |
| `overall` sd / range | 0.67 / 2.43 | 0.73 / 2.60 |
| `technical`~`overall` r | 0.74 | **0.21** |
| `technical`~`impact` r | 0.62 | **0.01** |
| `composition`~`impact` r | 0.94 | 0.93 |
| `overall`~`composition` r | 0.94 | 0.93 |
| stars 2★ / 3★ / 4★ | 0 / 10 / 1 | 1 / 9 / 1 |
| mean `overall` | 5.67 | 6.00 |

What moved: `technical` separated cleanly from the other three — per-axis
legends do isolate an axis. What did not: `composition` and `impact` are still
one number in disguise, the band is just as narrow, the stars are still
saturated, and every `overall` went *up* by roughly a third of a point. So
wording can move an axis apart and shift the level, but it did not widen the
distribution on this shoot.

## Open questions for the freeze (write them down rather than decide implicitly)

1. **Is this shoot the right test?** The rubric asks for photojournalistic
   keepers; this folder is a family snapshot set. A saturation result on frames
   that are all "record of the moment" may be the rubric being *right* rather
   than broken. The freeze needs a shoot from the genre the rubric is written
   for.
2. **Do `composition` and `impact` need separate questions at all?** They moved
   together (r ≥ 0.93) under two different wordings. Either the wording is still
   not doing the work, or on this material the two axes are not separable.
   Worth testing on a documentary/street shoot before merging or dropping them.
3. **Is the scale being used as designed?** A 0–10 legend whose middle is
   "competent" invites 5–7. If the photographer's own ratings also cluster at
   3★, the model is not wrong — the star vocabulary is. Check the review sheet's
   `your stars` column before changing the rubric again.
4. **`clef-oQ4e` (the 27B build) is untested.** This server serves both
   `clef-flash-oQ4e` (used here) and `clef-oQ4e`. The spec leaves the 27B alias
   and whether it is worth the latency undecided; a second run with
   `--model clef-oQ4e` would answer it (and produce a different rubric hash by
   design).
5. **`.dng` is accepted but not decodable in v1** (no DNG reader in the pinned
   `raw-files`; the row says so). A Z-body shooter configured for DNG gets
   `unreadable` rows, not scores. Decide whether v2 needs a DNG path or whether
   the spec should stop promising the format.
6. **Per-genre rubrics.** Nothing here settles whether the frozen default should
   be one rubric or a family (street/documentary/portrait). The spec leaves it
   open on purpose.

## Regenerating

From the repo root, with the server up:

```sh
./target/debug/imagecull score ~/Downloads/import-251221 --dry-run --previews-dir .pi/evidence/previews
./target/debug/imagecull score ~/Downloads/import-251221 --fresh \
    --out .scratch/nef-scoring-v1/evidence/scores.csv --json-scores .pi/evidence/answers.json
python3 .scratch/nef-scoring-v1/evidence/make_review.py .scratch/nef-scoring-v1/evidence/scores.csv \
    .pi/evidence/previews .pi/evidence/review-sheet-A.html \
    .scratch/nef-scoring-v1/evidence/review-sheet.md
```

The variant-B run adds `--rubric .pi/evidence/rubric-v2.toml --out .pi/evidence/scores-v2.csv`.
