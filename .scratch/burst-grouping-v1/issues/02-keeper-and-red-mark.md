# 02: The group's keeper, and the red mark on everyone else

**What to build:** `score` stops judging frames one at a time and judging groups
not at all. After a group's frames have been rated, one comparative request asks
clef which frame of the group to keep; the keeper gets no label at all, and every
other frame of the group gets `xmp:Label="Red"` — the only field a cull can filter
on that is portable between applications. A red frame is not a bad frame: it is a
frame that lost its group, and a 4★ red is correct when the group's keeper is a
5★. No rating is ever changed by this pass, and no measure changes a rating.
[ADR 0007](../../../docs/adr/0007-the-red-label-marks-the-non-keepers.md) holds
the decision; `.scratch/xmp-sidecar/spec.md` still governs everything else about
the sidecar, including its rule that a label we did not write is never overwritten
and never removed.

The verdict is recorded beside the rating in our own namespace, so a red can
always be explained without the folder:
`imagecull:groupKeeper` (`true`/`false`), `imagecull:groupStart`,
`imagecull:groupSize`, `imagecull:groupingHash`, `imagecull:pickBy`,
`imagecull:pickConf`, `imagecull:pickedAt`, and `imagecull:labeledBy="imagecull"`
on the frames whose label this tool wrote. `groupKeeper` is not a copy of the
label: the label can be missing (not ours to write) or foreign (the photographer's
Yellow), and the verdict still exists.

**Blocked by:** 01

**Status:** needs-info — two inputs are missing, and writing either down before
measuring would be the invented weighting `SPEC.md` §8.4 refuses:

- **The context budget of the comparative call.** Several previews go into one
  request. How many frames, at what preview size, before the request is too large
  for the model to answer — and what happens to a 20-frame burst. The spec's
  guess was ~512 px per frame for eight frames; that has to be measured, not
  assumed. If the cap binds, the measures supply the shortlist (see Notes), which
  is why issue 01's blind review still matters.
- **Where `do not compare` is drawn.** §Out of Scope of the burst spec forbade
  gating model requests on measures. Choosing which frames enter the *comparison*
  is not gating a *rating*, but it is the same machine, so the line needs to be
  written deliberately.

## Acceptance

- [ ] A group of two or more gets exactly one comparative request; a group of one gets none
- [ ] The keeper carries no `xmp:Label` write; every other frame of the group carries `Red`
- [ ] A frame whose label is the photographer's is left alone, its verdict still recorded, and counted on a summary line (`unmarked: 3` — label not ours)
- [ ] A frame whose label was written by a previous run is rewritten only if that run recorded `imagecull:labeledBy="imagecull"`
- [ ] No label is ever removed: a frame promoted to keeper by a later grouping keeps its red, and the summary warns that the folder disagrees with the current grouping
- [ ] A comparative call that fails, times out or answers nothing marks nothing and reports the group as undecided
- [ ] `xmp:Rating` is identical before and after the group pass for every frame
- [ ] A frame's rating survives a group pass that excludes it; excluded frames are red and still rated
- [ ] `--print-sidecars` shows the label line per frame — `xmp:Label  Red  (was: none)` — and writes nothing
- [ ] `imagecull groups` remains read-only and makes no request; its test suite still contains no HTTP
- [ ] Marks are written after the frame's rating, so the terminal never reports a mark whose rating is not on disk

## Notes

- **Why the losers and not the keeper.** The keeper is already the frame with the
  best stars; the non-keepers are the frames the photographer has to get past, and
  Red on them turns the cull into a filter rather than a search.
- **Why `groupKeeper` is not a second copy of the label.** The label is one
  rendering of a verdict that may be missing or foreign; recording the verdict
  separately is what lets the summary say "12 non-keepers, 9 red, 3 unmarked".
  The ADR 0006 objection ("two properties holding one number") does not apply:
  these are a judgement and its rendering, and the run warns when they diverge.
- **The stale-red warning is the only remedy.** Removing a label means guessing
  whether we wrote it, and a wrong guess deletes the photographer's own mark. So
  the run reports `12 red, 2 now the keeper` and leaves the files alone.
- **Cost.** One request per group on top of the per-frame pass: ~430 requests for
  400 frames in 30 groups. A `--force` re-run re-pays both.
- **First-pass artefact.** The skip rule leaves already-rated frames alone, so
  labels are written by the run that rated the folder and never refreshed by a
  cheap re-run. That is accepted in ADR 0007 and has to appear in the docs as a
  fact, not a surprise.
- **The ranking rule survives as a shortlist.** If the comparative call's context
  cap binds, its inputs become the measures' exclusions (camera focus report,
  acutance floor, roll) plus the per-frame rating — the same ordered criteria,
  now choosing candidates rather than a winner. That is why issue 01's blind
  review is still on the critical path.

## Comments
