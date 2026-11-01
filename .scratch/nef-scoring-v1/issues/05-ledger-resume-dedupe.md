# 05: Ledger, resume and burst dedupe

**What to build:** A 400-file run that can be interrupted and picked up again without losing work or producing a lying CSV. Every scored frame is appended to a JSONL ledger beside the output — a versioned header recording rubric hash, model, endpoint and start time, then one record per frame carrying the file's identity, the hash of the bytes that were actually sent, the status and the full outcome. Each record is flushed as it is written and a corrupt tail from a hard kill is repaired on recovery. `--resume` skips what is already recorded, matching on file plus size plus modification time so that a re-exported file is re-scored rather than silently recycled; `--fresh` discards the ledger deliberately; a ledger built under a different rubric or model is a hard, actionable error rather than a silent mix. A frame whose sent preview bytes match an earlier one reuses that outcome — the model is deterministic for identical bytes — while still getting its own CSV row with `duplicate_of` naming the first frame, and `--no-dedupe` turns that off. The CSV is rewritten from the ledger periodically during the run and at exit, so a killed run still leaves something readable.

**Blocked by:** 04

**Status:** ready-for-agent

- [ ] The ledger is written beside the output CSV with a versioned header and one record per scored frame, appended and flushed immediately
- [ ] A deliberately truncated/garbage final ledger line is repaired on recovery and does not poison the run
- [ ] `--resume` completes a cancelled run with no duplicate rows and no re-scored files
- [ ] A file whose size or modification time changed since it was scored is re-scored
- [ ] `--fresh` discards the ledger; a rubric-hash or model mismatch against an existing ledger without `--fresh` is a fatal, actionable error
- [ ] Identical sent preview bytes reuse the earlier outcome and set `duplicate_of` to the first frame's filename; every NEF still gets its own row
- [ ] `--no-dedupe` scores every frame independently
- [ ] The CSV is rewritten from the ledger every 25 rows and at exit
- [ ] Acceptance: cancelling mid-run then resuming produces the same CSV as one uninterrupted run
- [ ] Acceptance: two full runs over the same folder with the same rubric and model produce byte-identical CSV
