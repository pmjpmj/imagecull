# Spec — burst grouping and best-of-group, report only

Status: issue 01 ready-for-human (its blind review is the photographer's step) ·
issue 02 resolved (2026-11-21)
Date: 2026-11-02 (amended 2026-11-19 and 2026-11-21)
Source: `SPEC.md` §17 open question 4 (bursts in the CSV) and the ticket 08
evidence (`.scratch/nef-scoring-v1/evidence/evidence.md`)

---

## Problem Statement

v1 scores one frame at a time against the words of a rubric. Two things follow
from that, and both hurt most on exactly the shoots where culling is most
expensive — the burst-heavy ones.

**The model is asked to measure things it cannot measure.** `technical` asks
whether the frame is in focus, whether there is motion blur, and (through
`composition`) whether the horizon is level. The ticket 08 evidence says what
that is worth: `technical` was the only axis whose correlations could be moved
by rewording, mean top-option confidence for `overall` was 0.23 against a 0.09
chance floor, and the whole shoot's `overall` spread was 0.67 sd. A vision model
reading a 1568 px q88 preview is making a coarse aesthetic call. It is not
measuring acutance, and it cannot see the difference between "sharp" and
"sharpest" across two frames of the same moment.

**Bursts are only deduplicated when they are byte-identical.** `preview_sha256`
(SPEC §9.2, ADR 0002) fires only on identical sent bytes. On a real burst —
10 fps, the sensor noise, the AF micro-adjustments and the subject all moving —
that almost never happens. So a 20-frame burst costs 20 full requests, and the
photographer still has to open all 20 frames side by side to answer the one
question they actually have: *which one?*

The photographer's real question about a burst is **relative** — "of these
twelve, which is the keeper" — and the pipeline has no concept of a group to ask
it in.

---

## Solution

Two additions, both deterministic and both local: a **group** of frames that are
the same moment, and a set of **measures** computed from the preview with no
model involved. Grouping and measures are recorded in the report and the ledger
beside the existing six answers; a declared, visible **ranking rule** names the
best frame of each group.

Nothing about the six answers changes. No frame is skipped, no request is
saved, no row is collapsed, no score is combined into a composite. This slice is
**report-only**: it adds information to the run, it never takes any away.

The reason for that restraint is the evidence, not caution for its own sake. The
honest failure mode of a sharpness metric is a plausible-looking number that is
wrong, and a metric that silently decided which frames get a model request would
turn that wrong number into missing scores. So the measures ship first as a
**model-free command** (`imagecull groups`), get validated against the
photographer's own eye using the review-sheet harness that already exists
(`.scratch/nef-scoring-v1/evidence/make_review.py`), and only then appear as
report columns on `score`.

---

## User Stories

### Same moment

1. As a photographer shooting bursts, I want frames taken within a moment of each
   other, with the same body and the same exposure settings, to be reported as
   one group, so that the tool's output has the shape of the decision I am
   actually making.
2. As a photographer, I want grouping to require the frames to *look* like the
   same scene as well as being close in time, so that two unrelated frames that
   happen to be a second apart never end up in one group.
3. As a photographer who pans across a subject while bursting, I want that burst
   to split into the groups the pictures actually form, so that the grouping
   never claims two different compositions are one choice.
4. As a photographer, I want a frame with no capture time to be a group of its
   own rather than guessed at by filename, so that missing metadata never invents
   a decision.
5. As a photographer, I want groups numbered in capture order and each frame to
   carry its group and its index within it, so that "3 of 7" means what it looks
   like.
6. As a photographer, I want `--no-group` to turn the whole feature off, so that
   a report can be exactly what v1 produced.

### The measures

7. As a photographer, I want the shake, focus, level and clipping of each frame
   measured from the image rather than asked of the model, so that the number is
   reproducible and does not depend on the model's confidence that day.
8. As a photographer, I want shake reported separately from blur, so that a
   camera that moved and a lens that missed tell me different things.
9. As a photographer, I want focus measured as the sharpest region of the frame
   rather than the average, so that a sharp subject against a soft background
   is not scored as blurred.
10. As a photographer, I want the camera's own autofocus report — where it
    focused and whether it confirmed — read out of the file, so that "focused
    behind the subject" is visible as a fact rather than an impression.
11. As a photographer, I want the horizon roll measured in degrees, so that a
    tilted frame is a number I can sort by.
12. As a photographer, I want a frame with no dominant lines to report no roll
    rather than 0°, so that a portrait against a plain wall is never marked down
    for a horizon it does not have.
13. As a photographer, I want the measures taken from the camera's preview at a
    resolution above what is sent to the model, so that I am not ranking frames
    by JPEG-88 artefacts.
14. As a photographer, I want the measure of a frame to be comparable with the
    measure of any other frame, whatever the camera's resolution or crop mode,
    so that a group spanning a crop-mode frame does not rank by size.
15. As a photographer, I want to read every measure in a command that needs no
    server, so that I can run the whole technical pass on a 400-frame shoot in
    seconds and look at it before spending any model time.

### The pick

16. As a photographer, I want one frame of each group named as its best, so that
    a burst-heavy shoot arrives as a set of decisions rather than a pile of
    frames.
17. As a photographer, I want the reason a frame was or was not picked recorded
    next to it, so that I can disagree with the rule and see why, rather than
    trusting a black box.
18. As a photographer, I want the rule to be an ordered list of criteria over
    named measures, never a weighted composite, so that no invented weighting
    can hide inside a number.
19. As a photographer, I want the model's rating to decide between frames that
    are technically equal, so that the judgement the model is actually good at
    makes the call the measures cannot.
20. As a photographer, I want the rule's exclusions reported per frame, so that a
    bad rule shows up in `group_note` instead of being reconciled away.
21. As a photographer, I want the grouping algorithm and its thresholds hashed
    into the ledger header, so that changing a threshold invalidates group ranks
    exactly as changing rubric wording invalidates scores — and never silently.
22. As a photographer, I want the CSV to keep one row per frame with the group
    columns added, so that no filename ever disappears from a report.

---

## Implementation Decisions

### Where it sits

A new core module, `group`, plus a measure module, `measure`. Both are pure
functions over already-extracted previews and already-read metadata: no request,
no `Decider`, no reporter events, no filesystem writes. `group` sits between
`preview` and `scoring` — it needs the preview and the metadata, and everything
after it can carry a group id.

**Hard rule carried forward:** nothing here may consult the model. Grouping and
measures must be computable with no server running, and issue 01's whole test
suite runs with no HTTP at all.

### Grouping — three signals, one gate order

The gate runs in capture order (not filename order), and a candidate group is
closed as soon as any gate fails. All gates are cheap; the perceptual one is the
only one that needs the preview.

| # | signal | source | rule (defaults) |
|---|---|---|---|
| 1 | capture time | `metadata().capture_time()`, subsecond precision | `Δt ≤ group_gap` (2.0 s) from the previous frame in the group |
| 2 | same body | `metadata().serial_number()` | equal; a file with no serial only groups with another file with no serial |
| 3 | same settings | `metadata().exposure()` | equal ISO, exposure time, f-number, focal length; equal orientation |
| 4 | same scene | preview signature | cosine similarity ≥ `group_similarity` (0.90) against the group's first frame |

Signal 4 is the one that makes the feature safe. Without it, "a second apart"
merges a subject who moved behind a tree, a hat that came off, a pan. With it, a
pan splits into the compositions it actually produced — which is the correct
answer even though it is not what a naive burst detector would say.

**Signature.** Decode the largest embedded preview (already the code path in
`preview`), apply orientation, convert to greyscale, Lanczos3-resize to 32×32
(`fast_image_resize` is already a dependency), subtract the mean, store 1024
`f32` — about 4 KB per frame, so a 400-frame shoot's signatures are 1.6 MB.
Similarity is the cosine of two signatures. Grouping is a **separate pass** over
the folder that keeps only signatures, never preview bytes: holding 400 decoded
previews would be gigabytes.

Cost is one extra preview extraction and decode per frame — roughly 40 ms, which
is 16 s over 400 frames and two orders of magnitude below the model time it is
there to inform. Issue 01 must state the measured cost, not assume it.

**Determinism.** Grouping is a pure function of the files' metadata, their
preview bytes and the two thresholds. `grouping_hash` = SHA-256 over an
algorithm version string, the signature parameters and the thresholds — never
over a filename, never over run options that do not change the answer. It is
recorded in the ledger header beside `rubric_hash`, and a mismatch on `--resume`
is handled exactly as a rubric mismatch is (ADR 0002's pattern: hard error, or
`--fresh`).

**Relation to dedupe.** Dedupe (§9.2) is a *cache*: identical bytes, reuse the
answer. Grouping is a *fact about the shoot*: these frames are the same moment.
They are independent, and neither replaces the other. A group is not a duplicate;
a duplicate is not necessarily the only member of its group.

### Measures

Computed from the largest embedded preview, oriented, downscaled to a **fixed
working size** of 2048 px on the long edge, as `f32` greyscale. The fixed size is
not arbitrary: it makes measures comparable across bodies, crop modes and
resolutions, and 2048 is comfortably above the 1568 px sent to the model, so the
ranking is not reading the encoder's artefacts. All measures are recorded at full
precision, never rounded in the CSV.

| column | what it measures | how |
|---|---|---|
| `acutance` | is anything in this frame genuinely sharp | p95 over 64×64 blocks of `var(∇²I)/var(I)` per block — contrast-normalised, so a flat grey wall and a busy street are on one scale, and a percentile so a sharp subject against a soft background reads as sharp |
| `shake` | did the camera move, and in which axis | `abs(E_x − E_y) / (E_x + E_y)` where `E_x`, `E_y` are total gradient energy per axis; isotropic defocus moves both equally, directional blur does not |
| `roll_deg` | is the horizon level | magnitude-weighted gradient-orientation histogram, peak within ±10° of horizontal (and the vertical counterpart); the offset of that peak. **Empty when no peak clears `roll_min_strength`** |
| `roll_strength` | was there a horizon at all | the winning peak's share of orientation weight, so "no lines" is distinguishable from "0.0°" |
| `clip_hi`, `clip_lo` | blown highlights, crushed shadows | percentage of pixels at the extreme, per channel, over the RGB preview |
| `af_in_focus` | did the camera confirm focus | `metadata().focus().in_focus()` — `true` / `false` / empty when unrecorded |
| `af_off_subject` | did it focus on the wrong thing | flag: acutance in the window around `focus().center()` is below `af_ratio` (0.6) of the frame's p95 `acutance` |

`roll_deg` never causes anything to be changed — this is report-only, and
straightening is out of scope (§Out of Scope). The measure exists so a
photographer can sort by it and so the ranking rule can use it.

A frame whose preview cannot be decoded gets its measures empty and a group of
its own: a measure failure is never silently a zero, and never a reason to
change a score.

### The ranking rule

Lexicographic, over named measures, every clause reflected in an output column.
It is a small, explicit function with a version string. Default order, subject to
issue 01's validation:

1. **Exclude** frames the camera reports `af_in_focus = false`.
2. **Exclude** frames more than `sharp_floor` (0.85) below the group's best
   `acutance` — the "somebody blinked the camera" frames.
3. **Exclude** frames whose `roll_deg` deviates more than `roll_tolerance`
   (0.5°) from the group's median roll, **only when the group has a horizon**
   (`roll_strength` above the strength floor on a majority of the group).
4. **Among survivors**, choose the highest rating; tie-break the lowest `shake`,
   then filename for stability.

Recorded per frame: `group`, `group_size`, `group_rank` (1 = best), `is_best`,
and `group_note` naming the deciding clause (`rating`, `only_survivor`,
`excluded:focus`, `excluded:sharpness`, `excluded:roll`) — the same spirit as
`duplicate_of`: a column that explains itself.

> **Amended 2026-11-19 by [ADR 0007](../../docs/adr/0007-the-red-label-marks-the-non-keepers.md).**
> The columns above have no container left: the CSV and the ledger are deleted, and
> a group rank in a sidecar is unreadable without the folder. The winner is now
> chosen by one **comparative request** over the group's previews, the keeper
> gets no label, and every other frame of the group gets `xmp:Label="Red"`;
> the verdict and its provenance are recorded in the `imagecull:` namespace.
> The ordered criteria below survive as the **shortlist** that decides who enters
> the comparison when the request's context budget binds — they no longer choose
> a winner, and they never change a rating.

The summary prints the group count and the mean and largest group size. (The
rule-disagreement rate this section used to specify compared the group winner to
the frame with the highest `overall`; [ADR 0005](../../docs/adr/0005-one-rating-replaces-six-questions.md)
made the rating the only answer, so there is no second number to disagree with.)

---

## Testing Decisions

**What makes a good test here:** a test builds a small synthetic burst on disk,
runs the core entry point, and asserts on the ordering and the recorded values —
never on an internal helper. The feature's entire value is a *ranking*, so the
tests are ranking tests.

**Synthetic bursts, with ground truth.** Real NEF fixtures are not committed (they
are large) and — more importantly — a real burst has no ground truth about which
frame is sharpest. A synthetic one does. Tests generate a burst at runtime from a
single small committed JPEG by applying known transforms:

| frame | transform | expected |
|---|---|---|
| `sharp` | none | best `acutance` |
| `soft` | Gaussian blur, σ = 1.0 | below `sharp` |
| `shaken` | 5 px directional (horizontal) motion blur | `shake` high, `acutance` below `sharp` |
| `tilted` | rotate +1.5° | `roll_deg` ≈ 1.5 ± 0.2, rank below `sharp` when the group has a horizon |
| `blown` | +2 EV | `clip_hi` high |

Generated at test time from one committed file, so the suite is small,
deterministic and has an answer key. This is the only honest way to test a
sharpness ranking, and it is why this feature is testable at all.

**What is pinned:**

- Grouping: time gap, body, settings and similarity gates each split a
  synthetic burst on the boundary; a pan splits a burst into the compositions it
  produced; a missing capture time yields a singleton; group ids follow capture
  order; grouping is identical across two runs and independent of filename
  (`DSC_9` vs `DSC_10` ordering, and a folder whose names are shuffled).
- Measures: `roll_deg` recovers a known rotation within 0.2°; a no-lines frame
  yields an empty `roll_deg`, not 0; `shake` separates a horizontal motion blur
  from an isotropic one of similar acutance; `acutance` is invariant to the
  source resolution after the fixed working size; `af_off_subject` fires on a
  synthetic frame whose AF point sits over a soft patch.
- Ranking: the answer key above, plus each exclusion clause exercised alone;
  `group_note` names the clause that decided; an all-excluded group leaves the
  least-bad frame as `group_rank = 1` and says so rather than leaving the group
  without a winner.
- Robustness: an undecodable preview gets empty measures and a singleton group,
  never a panic and never a 0 that reads as "bad".
- Determinism: two runs over the same folder produce identical group and measure
  columns; `grouping_hash` changes when a threshold changes and not when a
  filename changes.
- No HTTP: issue 01's suite never starts a server. If a test needs one, the
  module boundary is wrong.

**Fixtures:** `tests/fixtures/burst/` holds the single small JPEG the synthetic
transforms are derived from, which is committable. Real-capture bursts remain
fetch-script territory and, per the existing convention, tests needing them skip
loudly.

---

## Out of Scope

- **Deciding which frames get a model request.** No gating, no `--best-of-group`
  scoring, no cost saving. Explicitly deferred until the measures have been
  validated against a photographer's eye; a wrong metric must not be able to
  remove scores from a report.
- **Multi-image requests.** Sending a group in one `/v1/systemone` call with a
  `choice` question ("which of these is sharpest?") is a plausible later win —
  one request per burst instead of N — but it breaks the one-frame-per-decision
  attributability of SPEC §7.1 and needs a context-budget experiment (~512 px per
  frame for eight frames). Not this slice, and not assumed by it.
- **Straightening, cropping, retouching, or any write to an image.** Report-only.
- **Collapsing a burst to one CSV row.** Answering SPEC §17 open question 4: no.
  One row per frame, group columns added; a filename never disappears.
- **Face, eye or expression detection.** "Eyes closed" is a model judgement in
  this slice — it reaches the ranking only through the rating.
- **Aesthetic quality from the measures.** `acutance` is not a score out of ten
  and is never to be presented as one; it ranks, in a group, against frames of
  the same scene.
- **Per-genre measure thresholds.** One set of defaults, validated on one shoot.

---

## Further Notes

**Two issues, in order.**

| issue | scope | done when |
|---|---|---|
| 01 | `group` + `measure` in core, and `imagecull groups <DIR>` — grouping and measures, model-free, no report wiring | the synthetic answer key ranks as expected, and a real shoot's groups have been eyeballed against the photographer's own picks |
| 02 | `score` groups and measures; one comparative request per group; the keeper unmarked and every other frame `xmp:Label="Red"`; verdict in the `imagecull:` namespace; summary warnings for unmarked and stale reds | a burst-heavy shoot culls by filtering Red, and the photographer disagrees with the keeper more often than not only on frames they would have kept anyway |

Issue 02 is `needs-info` until 01's evidence lands, because two of its inputs are
numbers the evidence has to supply: which acutance formulation wins (`p95` local
vs a plain global variance), and where `sharp_floor`, `roll_tolerance` and
`af_ratio` belong. Writing those down before the measurement would be exactly the
kind of invented weighting SPEC §8.4 refuses.

**Validation protocol for issue 01** (uses the harness that already exists): run
`imagecull groups <DIR> --dry-run` over a real burst-heavy shoot, extend
`make_review.py` to lay out one row per group ordered by `group_rank` with blank
"my pick" and "why" columns, and have the photographer fill it in blind — the
measure columns hidden. Agreement rate, and every disagreement read for which
measure was wrong, is the evidence. This is a `ready-for-human` step inside
issue 01, and issue 01 stays open until it is done.

**Performance:** ~10 ms of measures plus ~40 ms of extra extraction per frame, so
a 400-frame shoot's technical pass is well under a minute against the 5–25
minutes of model time. A run that groups pays the extraction twice (once for the
signature pass, once for the preview it sends); if that turns out to matter, the
signature pass can write to the previews cache `--dry-run` already produces.

**Dependency choices:** none new. `raw-files` already exposes `capture_time`
(with subseconds), `serial_number`, `exposure` and `focus()` with `in_focus` and
`center()`; `image` already decodes and converts; `fast_image_resize` already
resizes; `sha2` already hashes. The gradient, block-variance and
orientation-histogram work is a few hundred lines in the core.

**Risks carried in:**

- **A plausible wrong number.** The measures look authoritative and are not
  validated until the protocol above runs. Mitigation: report-only, full
  precision, no composite, and every rank carries its reason.
- **The horizon detector on textured scenes.** Foliage, crowds, brickwork and
  rain produce no stable peak; a naive detector reads noise as a tilt. Mitigation:
  `roll_strength` is recorded and a missing peak is empty, not 0°, and the
  ranking's roll clause only applies to groups that have a horizon on a majority.
- **Shallow depth of field read as blur.** A deliberately soft subject is not a
  mistake. Mitigation: p95-of-blocks rather than a mean, and the AF point's own
  report as a separate fact.
- **Panning bursts split.** Correct behaviour, surprising output; the summary's
  group count is where it shows up.
- **`raw-files` AF metadata is Nikon-centric.** `in_focus` is `None` on Sony and
  on manual focus. Every clause that depends on it must degrade to "unknown",
  never to "excluded".
- **Crop-mode and mixed-resolution groups.** Handled by the fixed working size,
  but a mixed group is the case most likely to rank by size if that is ever
  changed; pin it with a test.
