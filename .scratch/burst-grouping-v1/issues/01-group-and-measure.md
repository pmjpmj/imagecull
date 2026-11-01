# 01: Groups and measures, model-free

**What to build:** Two new core modules and one new subcommand. `group` puts the
frames of one shoot into groups — frames that are the same moment — using four
gates in order: capture time within a gap, the same body serial, the same
exposure settings and orientation, and a perceptual similarity of the preview
within a threshold. `measure` computes, per frame and from the camera's own
preview, the things the rubric currently asks the model to guess: acutance as the
95th percentile of contrast-normalised Laplacian variance over 64×64 blocks,
shake as the anisotropy of horizontal against vertical gradient energy, horizon
roll in degrees from a magnitude-weighted gradient-orientation histogram (empty,
with its peak strength recorded, when there is no dominant line), clipping at
both ends, and the camera's own autofocus report — where it focused and whether
it confirmed. `imagecull groups <DIR>` runs both, needs no server and no
`Decider` at all, prints one line per group and writes the measures as a CSV.
Nothing about `score`, the six answers, the ledger or the existing CSV changes in
this ticket.

**Status:** ready-for-human — everything the agent can build is built and pinned; the blind review of a real shoot is the photographer's step (see `.scratch/burst-grouping-v1/evidence/evidence.md`).

## Acceptance

- [x] A frame whose capture time is within the gap but whose preview similarity is below the threshold is in a different group
- [x] A panning burst splits into the compositions it produced, not one group
- [x] A frame with no capture time is a group of its own; a frame with `af_in_focus` unrecorded is *unknown*, never excluded
- [x] Group ids follow capture order, not filename order; shuffling filenames does not change the groups
- [x] Grouping is identical across two runs; `grouping_hash` changes with a threshold and not with a filename
- [x] `roll_deg` recovers a known synthetic rotation within 0.2° (1.505° for a horizon drawn at 1.5°), and a frame with no dominant line yields an empty `roll_deg` with its strength recorded
- [x] `shake` separates horizontal motion blur from isotropic blur of similar acutance (0.140 against 0.001)
- [x] `acutance` is invariant to source resolution: the same scene at two sensor sizes ranks the same
- [x] `af_off_subject` fires when the AF point sits over a soft patch of a synthetic frame, and the point is mapped through the frame's EXIF orientation
- [x] An undecodable preview yields empty measures and a singleton group, with no panic and no zero that reads as "bad"
- [x] `imagecull groups` runs the whole pass with no server running and the test suite contains no HTTP
- [ ] A real burst-heavy shoot has been reviewed blind against the photographer's own picks, with the agreement rate and every disagreement recorded as evidence
- [x] Measured cost per frame is recorded in the evidence, not assumed (69 ms/frame at a 3000×2000 preview, 22 ms/frame at 1600×1067, release)

## Notes

- The synthetic answer key is the whole testing strategy: one small committed JPEG transformed at test time into `sharp`, `soft` (Gaussian σ = 1.0), `shaken` (5 px horizontal), `tilted` (+1.5°), `blown` (+2 EV). That is what makes a ranking testable at all — a real burst has no ground truth about which frame is sharpest. See the spec's Testing Decisions.
- The signature pass keeps only a 32×32 mean-subtracted greyscale `f32` per frame (~4 KB); holding the decoded previews would be gigabytes on a 400-frame shoot. It costs one extra extraction per frame, and if that ever matters it can reuse the previews cache `--dry-run` already writes.
- Fixed working size for measures is 2048 px on the long edge, after orientation: above the 1568 px sent to the model, so the ranking is not reading JPEG-88 artefacts, and fixed so a crop-mode frame does not rank against a full-frame one by size.
- `raw-files` gives `metadata().capture_time()` with subseconds, `serial_number()`, `exposure()` and `focus()` with `in_focus()` and `center()`. All of this ticket's inputs already exist; there is no new dependency. It exposes no roll or pitch tag, which is why the horizon has to be measured from pixels.
- The blind review protocol reuses `.scratch/nef-scoring-v1/evidence/make_review.py` and the `--dry-run` previews: one row per group in `group_rank` order, measure columns hidden, blank "my pick" and "why" columns. This step is the photographer's, so this ticket stays open until it is filled in.

## Comments
## Evidence (2026-11-02)

Built on `main`; the numbers, the acceptance table and what the build changed
from the spec live in `.scratch/burst-grouping-v1/evidence/evidence.md`.

- **What landed:** `imagecull_core::group` (four gates, 32×32 mean-subtracted
  signature, `grouping_hash`, `Plan`, the measures CSV) and
  `imagecull_core::measure` (acutance, shake, roll with its strength, clipping,
  the camera's focus report and `af_off_subject`), plus `imagecull groups <DIR>`.
  `preview::read` gives one open per frame — preview and metadata together — and
  `preprocess::decode` and `preprocess::fit` are now shared by the payload, the
  signature and the measures, so a frame is decoded once however many numbers are
  taken from it.
- **The answer key measures as the transforms say:** acutance 3.759 sharp / 0.464
  soft / 1.938 shaken; shake 0.140 shaken against 0.001 soft; roll 1.505° for a
  horizon drawn at 1.5°; `clip_hi` 74.06 % for +2 EV against 0.00 %; noise peaks
  at 0.012 of the orientation weight, under the 0.05 floor, so it reports *no*
  angle rather than 0°.
- **Cost, measured not assumed:** 69 ms/frame in release over 60 synthetic NEFs
  at 3000×2000 — above the working size, so the 2048 px fit runs — and 22 ms/frame
  at 1600×1067, where it does not. That is the whole pass: extraction, decode,
  orientation, fit, every measure and the signature, reported by the command's own
  summary. The spec budgeted ~50 ms; a 400-frame shoot is about half a minute.
- **Determinism:** two runs over the same folder wrote byte-identical CSVs.
- **Still open (the photographer):** the blind review against their own picks,
  which also settles which acutance formulation wins and where `sharp_floor`,
  `roll_tolerance` and `af_ratio` belong — the two inputs issue 02 is blocked on.
- **Deviation from the spec, recorded:** the answer key's scene is generated at
  test time (`tests/support/burst.rs`) instead of committing one small JPEG. It
  is deterministic, reviewable as code, and needs no binary in git; the
  resolution-invariance test comes free from a generator. The spec's
  `imagecull groups <DIR> --dry-run` invocation is also obsolete: the command is
  model-free by construction and has no `--dry-run`.
- **A finding for issue 02:** the roll estimate drifts by up to 0.33° on blurred
  frames, so angles below about 0.4° are not yet distinguishable from level —
  the same order as the proposed `roll_tolerance` of 0.5°.
- **Reviewed and fixed:** a two-axis review (standards and spec, parallel
  sub-agents) found that the signature was *not* the documented 32×32 — the fit
  preserved the frame's shape, so a 3:2 preview gave 32×21 and mixed aspects
  compared by truncation. It now resamples onto a square grid. Also fixed: the
  algorithm string said `burst-grouping` where the glossary says *group*, the AF
  window scaled by the wrong axis on a quarter-turned frame, duplicated luma,
  three colliding names, a dead rotation helper, fractions without a type, and
  thresholds only validated in the CLI. See the evidence file's review section.
