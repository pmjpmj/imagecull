# 04: The CSV — one row per frame, self-explaining

**What to build:** The v1 deliverable: a `scores.csv` the photographer can open in a spreadsheet and act on. One row per input file in ingest order, RFC 4180 and UTF-8, carrying the four raw scores at full precision, the star rating, the verdict, the three confidences, latency and input tokens, the status, a short error message on failed rows, the empty `duplicate_of` column that dedupe will fill later, the rubric hash and the model alias. Paths are relative to the input folder unless `--absolute`. `--json-scores <FILE>` writes the full raw model answer per file alongside, so a surprising score loses nothing. Golden snapshots pin column order and number formatting, and filenames containing commas, quotes and non-ASCII characters are escaped correctly.

**Blocked by:** 03

**Status:** ready-for-agent

- [ ] `imagecull score <DIR> --out <FILE>` writes a CSV covering every accepted input file, one row each, in ingest order
- [ ] Column set is exactly the spec's: file, the four scores, stars, verdict, the three confidences, latency, input tokens, status, error, duplicate_of, rubric hash, model
- [ ] Scores are the model's full-precision expected values; failed rows leave score cells empty rather than zero or a placeholder
- [ ] RFC 4180 escaping verified for filenames containing commas, double quotes and non-ASCII characters
- [ ] `--absolute` emits absolute paths; default is relative to the input folder
- [ ] `--json-scores <FILE>` writes the raw answer document per file
- [ ] `--limit <N>` scores only the first N files, so a run can be smoke-tested first
- [ ] Golden snapshot pins header, column order and formatting for the fixture set against a scripted decider
