# 02: The writer port, the filesystem writer, and the scan's own output

**Type:** task
**Status:** ready-for-agent
**Blocked by:** none
**Spec:** [../spec.md](../spec.md) §"Modules and interfaces" (the `Writer` port),
§"The sidecar document" (filename, atomicity), §"Statuses and exit behaviour"
(backups), §"Implementation Decisions" (ingest)

## What to build

Three small things, all of them about the sidecar's contact with the filesystem:

1. **The `Writer` port** in the `sidecar` module: `write(target, document,
   existing) -> Result<Written>`, where `Written` distinguishes created from
   replaced. Production and tests both need this shape — `scoring::run` will take
   it as a port (ticket 03).
2. **The filesystem writer**: the sidecar path for a frame (extension replaced, not
   appended), a temp file in the same directory, `rename` over the target,
   permission bits preserved when the target existed, and the `.bak` rule — a
   `.bak` only when the target already existed, never over an existing `.bak`.
3. **The scan stops counting the run's own output**: `.xmp` and `.bak` are no
   longer `skipped_extension`, and no new counter replaces them.

## Acceptance

- Writing a new sidecar leaves exactly one file, with no temp file left behind.
- Replacing an existing sidecar preserves its permission bits and its content is
  exactly the document passed in.
- A `.bak` appears only when the target already existed; a second write does not
  overwrite the first `.bak`.
- `DSC_0001.NEF`, `dsc_0001.nef`, `DSC_0001.NRW` and `DSC_0001.dng` all produce
  `DSC_0001.xmp` beside them.
- A write into a directory that cannot be written fails with an error the run can
  classify; nothing is left half-written.
- `ingest::scan` over a folder holding RAW files plus their `.xmp` and `.bak`
  reports the same counts as the folder before the sidecars existed.
- Tests for the writer use a real temp directory; the failure path uses the port
  with a failing implementation, not `chmod`.

## Notes

- `crates/imagecull-core/tests/ingest.rs` is where the counting assertions live.
- Error taxonomy: the port returns `Result`; ticket 03 decides which `ErrorKind`
  a write failure becomes. Keep this ticket's errors classifiable rather than
  printing.
- Prior art for a port in this codebase: `scoring::Decider` — a trait with a
  production implementation and a scripted one under `tests/support/`.
