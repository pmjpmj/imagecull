# 06: Failure taxonomy, retries, preflight and exit codes

**What to build:** A run that fails in the right way at the right scale. Every failure lands in the documented taxonomy and drives a decision the photographer would make: a corrupt or preview-less file is recorded and the run continues; a rejected rubric or a malformed request body aborts immediately, because a wrong rubric failing identically 400 times is worse than stopping; a too-long context is recorded with a hint to lower the preview size; a hiccup is retried three times with a short backoff, but only for the transient cases, never for a 4xx. Before the first file, a preflight confirms the server is answering and the alias exists, so a broken server costs seconds rather than an hour. The run classifies its own outcome through distinct exit codes — all ok, completed with failed rows, fatal, interrupted — and cooperative cancellation between files leaves the ledger consistent.

**Blocked by:** 04

**Status:** ready-for-agent

- [ ] Each status in the taxonomy is produced by its documented cause and follows its documented record-or-abort action
- [ ] Connection failures, timeouts and 5xx are retried three times with 0.5/1/2 s backoff; 4xx is never retried
- [ ] A rejected rubric and a malformed request body abort the run instead of grinding through the remaining files
- [ ] Preflight runs before the first file and fails fast with an actionable message when the server is unreachable or the alias is missing
- [ ] Exit codes distinguish all-ok, completed-with-failed-rows, fatal, and interrupted
- [ ] Cancellation is checked between files, not mid-request, and leaves an intact ledger
- [ ] Every failed row carries a short error message that explains itself without reading logs
- [ ] Acceptance: a synthetic 400-file run over fixtures completes with a status on every row
- [ ] Acceptance: with no XMP requested, an mtime scan of the shoot folder shows zero writes
