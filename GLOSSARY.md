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
orientation. A comparison renders its own, at its own edge: one request carries
many frames of one moment, so a rating's 1568 px and a comparison's 512 px are
separate numbers with separate keys — `[score] max_edge` and `[group] max_edge`.
_Avoid_: thumbnail, resized image, payload

**Run**:
One invocation over a shoot folder, from the scan to the summary: a `score` run
rates frames, a `groups` run groups them and marks the ones it would not keep.
_Avoid_: session, job, batch, process

**Cull**:
The act of deciding which frames to keep. The tool rates every frame and marks
the ones that lost their group; the photographer decides what to do about them.

### The server

**Server**:
The HTTP service a run asks for one rating per request, speaking the SystemOne
decision API — a local oMLX by default, and any host speaking that API when
`[server]` names one. A server is described by its wire, never by its vendor.
_Avoid_: backend, provider, model host, cloud

**Token**:
The bearer credential a configured server may demand, read from the environment
variable `[server] token_env` names and never written anywhere a run writes.
_Avoid_: API key, secret, password, credential

### How frames relate

**Group**:
The frames of one shoot that are the same moment: close in capture time, the
same body and the same picture — the same frame and optics, at an exposure
within a tolerance — and the same scene by signature. An exposure that drifted
inside that tolerance is still one moment; a zoom, a turn or a bracket past it
is not.
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

**Comparison**:
The one relative question a group asks the model — which of these frames to
keep — carrying a preview of every frame it covers. Never a rating, and never
narrowed by one.
_Avoid_: ranking, vote, tournament

**Round**:
One comparison per batch of a group too large for a single request. The batches'
winners are compared again until one frame survives, so every frame of the group
enters a request.
_Avoid_: pass, stage, heat

**Red mark**:
The `xmp:Label` value `Red`, written to a group's non-keepers when the comparison
was at or above the confidence floor, so the cull can filter for them. A
statement about the group, not a rating and not a reject: written only where the
label is ours to write, and rewritten only when our own later verdict says
something else — a red this tool did not write is never touched.
_Avoid_: flag, reject, colour label, label (unqualified)

**Yellow mark**:
The `xmp:Label` value `Yellow`, written to a group's non-keepers when the
comparison was below the confidence floor. It says what Red says — not this
group's keeper — and adds that the comparison was a near-tie, so the photographer
should look before deleting. Written and rewritten on the same terms as a red
mark; a photographer's Yellow is never touched.
_Avoid_: warn flag, soft red, tentative reject

**Green mark**:
The `xmp:Label` value `Green`, written to a group's keeper. It is the
comparison's answer made visible in a grid, not a second opinion on the picture:
written and rewritten on the same terms as a red mark, and never over a
photographer's own colour.
_Avoid_: keep flag, pick, best, hero

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
alias, the rating's preview parameters, scan breadth, the comparison's preview
parameters, grouping thresholds, output paths — after every source has been
resolved.
_Avoid_: settings, options, arguments, parameters

**Config file**:
A file that supplies values for an effective config, so they need not be typed
as flags on every run. A source of config, never the effective config itself.
Four sections: `[score]` and `[group]` own what one command needs, `[server]`
and `[scan]` hold what every command shares.
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
mark that follows from that. The run's only lasting record of a frame.
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
terminal. A `groups` run prints one as each frame is measured, and `--verbose`
adds one per group as each group is compared: `group 3 · 12 frames · keeper
DSC_0104.NEF · 11 red` — or `11 yellow` when the comparison was a near-tie.
_Avoid_: progress line, status line, log line, spinner
