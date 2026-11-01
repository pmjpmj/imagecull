# 07: A long run you can watch

**What to build:** The legibility layer for a five-to-twenty-five minute run. Progress reaches the CLI as plain events through the scoring core's reporter seam, so the core still never touches stdout and a window can replace the terminal later without touching scoring logic. While running, a live line shows index over total, filename, the scores and star rating, per-frame latency and an estimate of time remaining. At the end, a summary tells the photographer whether to trust the run: counts by status, mean and median for each of the four scores, the star histogram, the rate at which `stars` and `overall` disagree — reported, never reconciled — and total wall time. `--quiet` and `--verbose` control how much of this appears, so scripted use stays clean and debugging gets detail.

**Blocked by:** 05, 06

**Status:** ready-for-agent

- [ ] Progress, warnings and the summary reach the CLI as data through the reporter seam; the core still writes nothing to stdout, checks no TTY and calls no process exit
- [ ] A test can collect progress events into a collection and assert on them without a terminal
- [ ] The live progress line shows index/total, filename, scores and stars, per-frame latency and ETA
- [ ] The final summary reports counts by status, mean and median per score, the star histogram, the `stars`/`overall` disagreement rate, and total wall time
- [ ] The disagreement rate is reported as a number and nothing is reconciled or fudged
- [ ] `--quiet` suppresses per-frame output but keeps the outcome usable by a script; `--verbose` adds diagnostic detail
- [ ] Progress output appears only when the output is a terminal, so redirected output stays clean
