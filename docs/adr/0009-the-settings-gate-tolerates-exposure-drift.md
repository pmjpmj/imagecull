# 0009 — The settings gate tolerates exposure drift and asks for the picture exactly

Status: accepted · 2026-11-26 · Feature `burst-grouping-v1` ·
amends the grouping spec's gate 3 (`.scratch/burst-grouping-v1/spec.md`) and
`GLOSSARY.md`'s Group entry · bumps `group::ALGORITHM` from `grouping/1` to
`grouping/2`

## Context

Gate 3 of the grouping pass asked two frames to record **equal ISO, exposure
time, f-number, focal length and orientation** — `Settings` derived `PartialEq`,
and the gate read `first.settings != candidate.settings`. The spec justified it
in one line ("the same body and settings"), and the glossary inherited it.

On a real shoot that gate is wrong about the thing it is most often handed. A
burst is shot with auto ISO on, or with a bracket, or with the light changing
under a cloud, and the camera writes a different exposure on frame 4 than on
frame 3 without anything about the *picture* changing. The frames are the same
scene a third of a stop apart, the 32×32 mean-subtracted signature scores them at
0.99, the photographer wants the comparison that picks between them — and the
tool splits them into two groups of one, so no comparison happens at all.

The cost is one-directional and easy to miss. Splitting never corrupts anything;
it just quietly declines to cull. The run reports more groups, every group is
smaller, and the feature that exists for bursts does the least work on the
shoot whose exposure was least stable. Nothing in the summary says a gate did it.

Two facts make the fix safe rather than a loosening. The signature already
subtracts the mean and normalises length, so its own doc comment claims "a frame
a stop darker is still the same scene" — the scene gate was already built to
survive exposure drift, and gate 3 was the one place that refused to let it try.
And what actually makes grouping safe is the *picture*, not the metadata: the
similarity gate is what stops a pan merging, and it is unaffected by which half of
the metadata is asked for.

The opposite failure is real too, which is why the answer is a tolerance and not
a deletion. A 5-stop bracket is one moment in capture time but two very
different pictures to compare, and a scene whose lighting changed between frames
(a flash, a light switched on) is not one moment however close the clock says it
was. A gate that never looked at exposure would merge those.

## Decision

**Gate 3 asks for the picture exactly and tolerates the exposure.**
`Settings::within(other, stops)` replaces equality as the gate:

- **Exact:** `orientation` and `focal_length`. A frame turned on its side is not
  the same composition, and a zoom is a different picture — the 32×32 signature
  will usually catch a zoom, but "usually" is not a gate, and focal length is the
  fact that says so without guessing.
- **Tolerated:** the brightness the ISO, exposure time and f-number produce
  together, `log2(t · ISO/100 / N²)`, within `Thresholds::exposure_stops`. Two
  frames whose number differs by more than the tolerance are two moments.

**The tolerance is two stops by default** (`DEFAULT_EXPOSURE_STOPS`), settable
with `--group-exposure-stops`. Two stops covers the auto-ISO walk and the
one-stop-per-frame bracket a photographer actually shoots, and stops well short
of the flash-versus-no-flash case. `0` asks for one exposure exactly, and a very
large number stops the gate having an opinion about exposure at all — both are
the photographer's call, made visible on the command line rather than compiled in.

**The tolerance is part of the grouping hash.** `exposure_stops` joins `gap` and
`similarity` in `grouping_hash`, and `ALGORITHM` becomes `grouping/2`, so a
sidecar verdict computed under one tolerance is not silently readable under
another — the same rule that makes a rubric mismatch a hard error.

**A frame that cannot be placed is not shown to differ.** Brightness needs ISO,
exposure time *and* f-number; if either frame is missing one of the three, or
records a zero denominator, gate 3 passes on the exposure half. This is the rule
the serial number already followed ("two files recording no serial have not been
shown to differ"), and it is the opposite of what a *measure* does with a missing
value: a gate asks whether two frames *may* be one moment, a measure asks what is
true. Its consequence is worth stating plainly: on a body or a lens that writes no
f-number, the exposure half of the gate is inert, and grouping falls back to time,
body, optics and the picture. That is the honest failure mode — such frames are
grouped by what the tool can actually see.

**Equality is no longer the gate, and the code says so.** `Settings` keeps its
derived `PartialEq` (a test that compares two `Settings` should compare their
fields), but `within` is the question the pass asks, and both the type and the
gate carry a comment naming `==` as the thing ADR 0009 replaced.

## Considered options

- **Keep equality and tell the photographer to turn auto ISO off.** True for the
  next shoot and useless for the folder in front of them, which is the only folder
  a cull is ever run on.
- **Drop the exposure fields from `Settings` entirely.** One line, and it groups
  every burst the tolerance does; but it also merges a 5-stop bracket into one
  group and gives away the one fact that distinguishes "the picture was the same"
  from "the camera was pointed at the same thing". The tolerance keeps the fact
  and moves the line where the photographer can see it.
- **Tolerate focal length too.** A zoom rarely happens inside a two-second burst,
  and when it does the frames are different pictures by the photographer's own
  intent. The similarity gate would catch the large ones and miss the small ones,
  which is exactly the "usually" a gate must not have.
- **Ask for aperture exactly, since depth of field changed.** Tempting — an extra
  stop of aperture is a different photograph. But aperture, shutter and ISO are
  one exposure from the photographer's side: stopping down a stop and doubling
  the shutter is one picture a stop darker, and a burst that walked that way is
  one moment. Treating three fields as one number is the whole point of the
  brightness formula.
- **A weighted "how different are these settings" score.** SPEC §8.4 refuses a
  composite for the ranking rule and the same argument applies here: one number
  with an invented weighting inside it cannot be argued with. Two gates, one
  tolerance, both named on the command line.

## Consequences

- **Brackets inside two stops group, and are compared.** Three bracketed frames
  can now be one group, so `groups` asks one comparison over them and marks the
  two it does not keep. That is the intended reading of the feature — a bracket is
  the clearest case of "which of these do I keep" — but it is new behaviour and a
  photographer who brackets deliberately should know the marks land on it.
- **`ALGORITHM` becomes `grouping/2`, so every existing verdict is stale.** A
  sidecar carrying `imagecull:groupingHash` from `grouping/1` is readable but not
  comparable, and `--resume`-style reuse refuses it exactly as a rubric mismatch
  does. There is no migration and none is wanted: the old groups were computed
  without the fact the new gate reads.
- **A body that writes no f-number gets a partially inert gate.** Documented
  above, and the reason the missing-value rule had to be settled rather than
  assumed. If a real shoot shows over-merging on such a body, the answer is the
  time gap and the similarity threshold, both already flags.
- **`--group-exposure-stops` is a command-line flag and not a config key**, in
  line with `--group-gap` and `--group-similarity`: the config file has no
  grouping section, and adding one is a separate decision (ADR 0003).
  *Superseded by [ADR
  0010](0010-the-group-section-holds-the-grouping-thresholds.md), which adds the
  `[group]` section all three thresholds live in.*
- **The spec's gate table and the glossary's Group entry describe the old
  rule** and are corrected with the code, not before it: the docs describe what a
  build does.

## Amendments

**ADR 0010 added `[group]`.** The third bullet under Consequences said the
thresholds were flags only because the config file had no grouping section. That
section now exists (`gap`, `similarity`, `exposure_stops`), and every one of
those keys takes exactly what its flag takes. Nothing else in this ADR changes:
the gate still tolerates exposure drift, and `grouping/2` is still the algorithm
it describes.
