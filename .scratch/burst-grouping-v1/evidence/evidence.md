# Burst grouping issue 01 — groups and measures, model-free

Status: **built and pinned; the blind review is the photographer's.**
`.scratch/burst-grouping-v1/issues/01-group-and-measure.md` stays open for one
box — the frames reviewed against the photographer's own picks — because that
step is a human judgement, not an agent's. Everything an agent can prove is
below, with the test that proves it.

Date: 2026-11-02 · Branch: `main` · Binary: `target/release/imagecull` built
from that branch.

Artifacts:

- `crates/imagecull-core/src/group.rs` — the gates, the signature, the plan, the
  CSV.
- `crates/imagecull-core/src/measure.rs` — acutance, shake, roll, clipping, the
  camera's focus report.
- `crates/imagecull-core/tests/groups.rs` — the on-disk seam (13 tests).
- `crates/imagecull-core/tests/measures.rs` — the pixel seam (9 tests).
- `crates/imagecull-core/tests/support/burst.rs` — the synthetic answer key.
- `crates/imagecull-cli/src/groups.rs` — `imagecull groups`.

## What it does

```
imagecull groups <DIR> [--out FILE] [--recursive] [--limit N]
                       [--group-gap SECS] [--group-similarity COSINE]
```

Scans the folder, reads each file's largest embedded preview and its metadata
once, and prints one line per group plus a summary, writing one CSV row per
frame. No server is contacted; the command has no `--endpoint` flag to give it
one (pinned by `groups_asks_for_no_server_at_all`).

The CSV's columns, in order: `file, group, group_index, group_size, acutance,
shake, roll_deg, roll_strength, clip_hi, clip_lo, af_in_focus, af_off_subject,
status, error`. One row per frame, always; a frame without measures keeps its row
and leaves the measure cells empty.

## Acceptance evidence

| Acceptance | Where it is pinned |
|---|---|
| Time inside the gap, similarity below the threshold → different group | `a_pan_splits_the_burst_into_the_compositions_it_produced` |
| A panning burst splits into the compositions it produced | same test: 2 + 2 frames, two groups |
| No capture time → a group of its own | `a_frame_with_no_capture_time_is_a_group_of_its_own`, `an_unrecorded_fact_gates_nothing_and_never_invents_a_decision` |
| `af_in_focus` unrecorded is unknown, never excluded | `an_unrecorded_focus_report_is_unknown_not_excluded` |
| Group ids follow capture order, not filename order | `groups_follow_capture_order_and_not_the_order_of_their_names` |
| Shuffling filenames does not change the groups | `renaming_the_files_changes_nothing_about_the_groups` |
| Two runs agree; the hash moves with a threshold, not a filename | `two_runs_over_the_same_folder_agree_column_for_column`, `the_grouping_hash_moves_with_a_threshold_and_not_with_a_name`, `the_hash_is_the_algorithm_and_the_thresholds_and_nothing_else` |
| `roll_deg` recovers a known rotation within 0.2° | `roll_recovers_a_known_rotation` — a drawn 1.5° measures 1.505°, and a level scene 0.000° |
| No dominant line → empty `roll_deg` with its strength recorded | `a_frame_with_no_dominant_line_reports_no_roll_rather_than_zero` |
| `shake` separates horizontal motion from isotropic blur | `shake_tells_a_moving_camera_from_a_lens_that_missed_focus` |
| `acutance` is invariant to source resolution | `acutance_ranks_a_sharp_frame_above_a_soft_one_at_two_sensor_sizes` (1600 px and 800 px rank the same) |
| `af_off_subject` fires over a soft patch | `af_off_subject_fires_when_the_camera_focused_on_a_soft_patch`; the point is mapped through EXIF orientation by `the_focus_point_is_mapped_through_the_frames_orientation` |
| An undecodable preview → empty measures, a singleton, no panic, no zero | `an_undecodable_preview_is_an_error_rather_than_a_zero` (measures) and `a_frame_with_no_readable_preview_is_a_singleton_with_empty_measures_and_a_status` (the pass) |
| The whole pass with no server, no HTTP in the suite | every test in `tests/measures.rs` and `tests/groups.rs`; the modules have no `Decider` to call |
| Measured cost per frame recorded, not assumed | below |
| A real shoot reviewed blind against the photographer's picks | **not done — the photographer's step** |

## The synthetic answer key, as measured

One scene, transformed five ways, measured at 1600×1067 (release build). The
numbers are here so a reader can see what the measures do before trusting them:

| frame | acutance | shake | roll_deg | roll_strength | clip_hi |
|---|---|---|---|---|---|
| sharp | 3.759 | 0.003 | 0.014 | 0.225 | 0.00 |
| soft (Gaussian σ = 1.0) | 0.464 | 0.001 | 0.323 | 0.067 | 0.00 |
| shaken (5 px horizontal) | 1.938 | 0.140 | −0.030 | 0.103 | 0.00 |
| blown (+2 EV) | 3.713 | 0.005 | 0.327 | 0.087 | 74.06 |
| tilted (horizon drawn at 1.5°) | 441.572 | 0.997 | 1.505 | 0.328 | 0.00 |
| no lines (seeded noise) | 20.648 | 0.000 | *(empty)* | 0.012 | 0.66 |

- **Acutance ranks the answer key as the transforms do**: 3.759 sharp vs 0.464
  soft (8×), with the shaken frame between them at 1.938. Its absolute scale is
  scene-dependent (the flat two-tone tilted frame reads 441), which is why it
  may only ever rank frames of one group and is never a score out of ten.
- **Shake is the anisotropy, not the blur**: 0.140 for the 5 px horizontal blur
  against 0.001 for the isotropic one whose acutance is *lower*. A camera that
  moved and a lens that missed are different numbers.
- **Roll recovers a known angle**: 1.505° for a horizon drawn at 1.5°, and 0.014°
  for the level one. The detector's remaining noise shows in the blurred frames:
  their horizons are still found but drift to 0.33°, so **an angle below about
  0.4° is not yet distinguishable from no tilt** — which matters when issue 02
  sets `roll_tolerance` (the spec proposes 0.5°).
- **Clipping separates the +2 EV frame** at 74.06 % of channel samples against
  0.00 % for the clean frame.
- **No lines reports no angle**: noise peaks at 0.012 of the orientation weight,
  below the 0.05 floor, so `roll_deg` is empty while `roll_strength` is recorded —
  "no horizon" and "a level horizon" are different answers.

## Measured cost

Release build, one machine (Apple silicon), 100 synthetic NEFs at 1600×1067 with
a ~480 KB embedded preview each:

```
$ time imagecull groups /tmp/imagecull-burst --out /tmp/g1.csv
groups: 1 group in 100 frames (mean 100.0, largest 100) · 2s measured, 22 ms/frame
real 2.398s   user 2.184s   sys 0.020s
```

**22 ms per frame** for the whole pass — preview extraction, JPEG decode,
orientation, the 2048 px working fit, the five measures and the 32×32 signature —
measured by the command itself and reported in its summary (`plan.elapsed`), not
assumed. The spec budgeted ~50 ms/frame (40 ms extraction plus 10 ms of
measures); the pass is a little over twice as fast as that, and it is four
orders of magnitude below the model time it exists to inform.

Two runs over the same 100 frames produced byte-identical CSVs (`diff` empty),
which is the determinism claim: every column is a function of the files and the
two thresholds.

Grouping sanity, same folder (frames one second apart):

| command | result |
|---|---|
| default (`--group-gap 2.0`) | 1 group of 100 |
| `--group-gap 0.5` | 91 groups — the subsecond stamps split a 1 s cadence |
| `--group-similarity 1.0` | 1 group — identical previews are exactly 1.0 similar |

## What the build taught (and changed from the spec)

- **The roll estimate averages its whole search window, not its peak bin.** With
  0.1° bins over 8-bit gradients, a horizon 1.5° off level lands as a scatter from
  90° to 93° with a stray bucket at exactly 90° where the horizontal difference
  rounded to zero. The mode of that scatter, and the mean of a ±1° window cut out
  of one side of it, were off by 0.15–0.3°; the magnitude-weighted mean of the
  whole ±10° window recovers the drawn angle to 0.005°. The routine says so where
  it is done.
- **JPEG blockiness matters when a preview is smaller than the 2048 px working
  size**, because then there is no resampling to smooth it: an 8×8 block grid
  pulls gradients onto the axes. Real previews are usually larger than the
  working size; the synthetic roll fixture draws its horizon at the angle instead
  of rotating a hard-edged picture, so the answer key tests the measure rather
  than the resampler.
- **`roll_strength` is a share of total orientation weight, so heavy texture
  dilutes it.** On a real frame full of fine detail a genuine horizon may hold
  only a few per cent of the weight. The floor is therefore set at 0.05 (five
  times what a uniform spread holds in the ±1° window) rather than at a tidy
  number, and the value needs the blind review to settle it — this is the
  measure's `roll_min_strength` unknown.
- **Similarity of 1.0 is now exactly reachable.** A unit vector's dot with itself
  is 1 − ε in floating point, so identical frames measured 0.99999994 and
  `--group-similarity 1.0` split a burst into singletons. Two equal signatures
  now return exactly 1.0; a frame with no variation at all returns 0.0, because
  it has no scene to compare.
- **The synthetic-file builder stored short ASCII out of line**, the way TIFF does
  not: a two-digit `SubSecTimeOriginal` was read as garbage and every frame of a
  burst shared a timestamp. Fixed in `tests/support/mod.rs`; the tests now rely on
  subsecond capture times, so they would fail again if it regressed.

## The blind review (the photographer's step)

Unchanged from the ticket, and the reason this slice is report-only:

1. Run `imagecull groups <a burst-heavy shoot>` (no `--dry-run` is needed; the
   command never contacts a server) and keep `groups.csv`.
2. Lay the groups out one row per group, in the order the CSV gives them, with
   the measure columns hidden and blank "my pick" and "why" columns.
3. Fill it in blind — pick the keeper of each group from the previews alone.
4. The evidence is the agreement rate plus every disagreement read for which
   measure was wrong.

The review also settles the two numbers issue 02 is blocked on: which acutance
formulation wins (`p95` of local block variance, as built, or a plain global
variance normalised by contrast), and where `sharp_floor`, `roll_tolerance` and
`af_ratio` belong. Nothing here has been tuned to a shoot, because there was no
shoot to tune to.

## Not in this slice

- No ranking, no `group_rank`, no `is_best`, no `group_note` — that is issue 02,
  and it waits on the review above.
- No change to `score`, the six answers, the ledger or the scored CSV.
  `grouping_hash` reaching a ledger header is issue 02.
- No write to an image: `roll_deg` is reported and nothing is straightened.
- The `groups` command reads no config file. Its thresholds are the algorithm's,
  not a shoot's; where they belong in `imagecull.toml` is part of issue 02.
