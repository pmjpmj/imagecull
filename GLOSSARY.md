# imagecull

Culling a shoot of mirrorless RAW files with a local decision model: a shoot
folder goes in, one row of scores per frame comes out, alongside a record of
what the run did.

## Language

### The run

**Shoot folder**:
The directory of raw files one run covers.
_Avoid_: input dir, source directory, library

**Frame**:
One photograph in the shoot folder, the unit a score is given to.
_Avoid_: photo, image, file, shot

**Preview**:
The downscaled JPEG rendered from a frame and sent to the model, after
orientation.
_Avoid_: thumbnail, resized image, payload

**Run**:
One invocation of scoring over a shoot folder, from the scan to the summary.
_Avoid_: session, job, batch, process

**Cull**:
The act of deciding which frames to keep. The tool scores; the photographer
culls.

### What a run reads

**Rubric**:
The questions a frame is judged by, with the scoring legend each answer is
chosen from.
_Avoid_: prompt, template, config file

**Rubric hash**:
The identity of a rubric, alias included. Two runs with different hashes did not
judge the same way, and their scores are not comparable.
_Avoid_: checksum, fingerprint, version

**Effective config**:
The values a run actually uses — rubric, alias, preview parameters, scan
breadth, output paths — after every source has been resolved.
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

**Ledger**:
The record beside the report of what each frame was judged, so an interrupted
run can pick up where it stopped.
_Avoid_: journal, state file, cache, checkpoint

**Report**:
The table of scores one row per frame, read in a spreadsheet.
_Avoid_: output, CSV, results

**Resume**:
Picking up an interrupted run, reusing every frame the ledger already holds.
_Avoid_: continue, restart

**Dedupe**:
Reusing the judgement of a frame whose preview is byte-identical to another.
_Avoid_: cache hit, skip

### What a run prints

**Frame line**:
The one-line record a run prints as each frame is judged: where the frame sits
in the run, its answers, what the request cost, and when the run will finish.
One per frame and ended by a newline, so a redirected log reads the same as a
terminal.
_Avoid_: progress line, status line, log line
