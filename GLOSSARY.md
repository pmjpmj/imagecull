# imagecull

Culling a shoot of mirrorless RAW files with a local decision model: a shoot
folder goes in, one row of ratings per frame comes out, alongside a record of
what the run did.

## Language

### The run

**Shoot folder**:
The directory of raw files one run covers.
_Avoid_: input dir, source directory, library

**Frame**:
One photograph in the shoot folder, the unit a rating is given to.
_Avoid_: photo, image, file, shot

**Preview**:
The downscaled JPEG rendered from a frame and sent to the model, after
orientation.
_Avoid_: thumbnail, resized image, payload

**Run**:
One invocation of scoring over a shoot folder, from the scan to the summary.
_Avoid_: session, job, batch, process

**Cull**:
The act of deciding which frames to keep. The tool rates; the photographer
culls.

### How frames relate

**Group**:
The frames of one shoot that are the same moment: close in capture time, the
same body and settings, and the same scene by signature.
_Avoid_: burst, cluster, stack, set, duplicate group

**Signature**:
The 32×32 mean-subtracted greyscale of a frame's preview, compared with another
frame's to decide whether the two are the same scene. A fact about the picture,
unlike a hash of the bytes.
_Avoid_: hash, fingerprint, embedding, thumbnail

**Grouping hash**:
The identity of the grouping algorithm and its thresholds — never of a folder or
a filename. Two runs with different hashes did not group the same way, so a group
rank computed under one is not readable under the other.
_Avoid_: group hash, cluster id, version

**Measure**:
A number taken from a frame's own pixels or metadata rather than asked of the
model — acutance, shake, roll, clipping, the camera's focus report. Comparable
between frames of one group, never a score out of ten.
_Avoid_: metric, score, technical score, quality score

**Ranking rule**:
The ordered criteria over named measures that names a group's best frame —
exclusions first, then the model's rating among survivors. Declared in full,
never a weighted composite.
_Avoid_: ranking score, picker, weighting

**Group rank**:
A frame's place within its group under the ranking rule, 1 being the group's
best. Not comparable between groups, and not a score.
_Avoid_: winner score, rating, position

**Keeper**:
The frame of a group the group's comparison chose to keep, when the group has
more than one. A judgement about a set of frames, made once for the group, never
a claim about the frame's quality on its own — a keeper can be rated below
another shoot's best frame.
_Avoid_: pick, winner, best, hero, selection

**Non-keeper**:
Every other frame of a group. The frame the cull has to get past, not a frame the
tool has judged bad.
_Avoid_: loser, reject, reject flag, duplicate, outtake

**Red mark**:
The `xmp:Label` value `Red`, written to a group's non-keepers so the cull can
filter for them. A statement about the group, not a rating and not a reject:
written only where the label is ours to write, and never removed, because a
label this tool did not write is not a label it may delete.
_Avoid_: flag, reject, colour label, label (unqualified)

### What a run reads

**Rubric**:
The one question a frame is judged by, with the five-point legend the answer is
chosen from.
_Avoid_: prompt, template, config file

**Rating**:
The model's answer to the rubric's question: 1 delete, 2 not great, 3 okay to
keep, 4 good, 5 great. The whole judgement of a frame, written to `xmp:Rating` in
the frame's sidecar and nowhere else.
_Avoid_: score, grade, verdict, aesthetic score

**Built-in rubric**:
One of the rubrics compiled into the binary — `general`, `press`, `holiday` —
selected by name instead of by a path.
_Avoid_: embedded rubric, default rubric, preset, built-in prompt

**General rubric**:
The built-in a run uses when nothing selects another. It assumes no genre and
judges a frame on the terms the frame appears to set for itself.
_Avoid_: default rubric, generic rubric, the default

**Rubric hash**:
The identity of a rubric, alias included. Two runs with different hashes did not
judge the same way, and their ratings are not comparable. Which built-in a run
asked for is not part of it: the same words are the same judgement.
_Avoid_: checksum, fingerprint, version

**Effective config**:
The values a run actually uses — rubric (a built-in's name or a file's path),
alias, preview parameters, scan breadth, output paths — after every source has
been resolved.
_Avoid_: settings, options, arguments, parameters

**Config file**:
A file that supplies values for an effective config, so they need not be typed
as flags on every run. A source of config, never the effective config itself.
_Avoid_: settings file, config, profile, preset, rc file

**Shoot config**:
A config file that travels with a shoot folder.
_Avoid_: project file, local config

**User config**:
A config file that applies to every shoot on the machine.
_Avoid_: global config, default config

### What a run leaves behind

**Sidecar**:
The `.xmp` file beside a frame, carrying its rating, the provenance of that
rating, and — when a group decided one — whether the frame is its keeper and the
red mark that follows from that. The run's only lasting record of a frame.
_Avoid_: metadata file, XMP file, report, output

**Skip**:
Leaving a frame alone because its sidecar already carries a rating of 1–5. No
request is made, nothing is written, and the rating stays whatever it was.
_Avoid_: cache hit, reuse, ignore, duplicate

**Dedupe**:
Reusing the judgement of a frame whose preview is byte-identical to another
frame's within one run. A cache, not a relation: frames of one group are not
duplicates, identical bytes alone do not make a group, and the cache dies with
the run.
_Avoid_: cache hit, skip

### What a run prints

**Frame line**:
The one-line record a run prints as each frame is judged: where the frame sits
in the run, its answer, what the request cost, and when the run will finish.
One per frame and ended by a newline, so a redirected log reads the same as a
terminal. The model-free pass prints one too, as each frame is measured: where
it sits, its name, and its status, in the pass's own order, because the frame's
group is not known until the last frame has been read.
_Avoid_: progress line, status line, log line, spinner
