# 02: From shoot folder to sent previews

**What to build:** `imagecull score <DIR> --dry-run` takes a shoot folder and produces, for every accepted RAW, exactly the bytes that a scoring run would send to the model — written into a previews directory so the photographer can eyeball orientation, crop and detail loss before burning an hour. Along the way it scans the folder (accepted extensions case-insensitively, non-recursive unless asked, hidden directories never descended, Finder noise ignored and counted, natural NFC-normalised filename order, `--limit` for a smoke test), takes the camera's largest embedded preview measured by pixel area, applies EXIF orientation before any resizing, and downscales to the configured longest edge without ever upscaling. `doctor <FILE>` gains a preview-extraction self-test so the pipeline can be proven on the photographer's own camera output before a long run. Nothing is written into the shoot folder.

**Blocked by:** 01

**Status:** ready-for-agent

- [x] `imagecull score <DIR> --dry-run` writes one preview per accepted file and performs no HTTP
- [x] Extensions `.nef`, `.nrw`, `.dng` accepted case-insensitively; anything else counted as skipped, never an error
- [x] `._*`, `.DS_Store` and anything under `__MACOSX` ignored with a reported count; hidden directories never descended; `--recursive` opt-in
- [x] Files sorted by natural, Unicode-normalised filename so preview order and later CSV row order are stable
- [x] The preview chosen in a file that embeds several is the one with the largest pixel area, not the largest byte count
- [x] Orientation 1–8 applied before downscaling; a portrait fixture exports upright
- [x] Longest edge capped by `--max-edge` (default 1568), `--jpeg-quality` (default 88) honoured, and a preview smaller than the cap is never upscaled
- [x] Encoded JPEG carries no EXIF/APP1
- [x] A file with no embedded preview and a truncated/garbage file each yield a classified failure without a panic, and the run continues
- [x] `doctor <FILE>` self-tests preview extraction on a real file
- [x] A mtime scan of the shoot folder after a run shows zero writes

## Comments

Implemented. M1 is in: `imagecull score <DIR> --dry-run` takes a shoot folder to
a previews directory with no server anywhere in the path, and `doctor <FILE>`
proves the pipeline on one of the photographer's own files.

- **Ingest** — `core::ingest::scan`. `.nef`/`.nrw`/`.dng` accepted
  case-insensitively, everything else counted into `skipped_extension` and never
  an error. `._*` and `.DS_Store` files and every file under a `__MACOSX`
  directory (counted recursively) go into `ignored`; directories whose name
  starts with a dot are never descended and are counted in
  `hidden_directories`; `--recursive` is opt-in; `--limit N` truncates after
  ordering, so a smoke test sees the first N frames and the counts still
  describe the whole folder. Order is natural (digit runs compare as numbers,
  so `DSC_9` < `DSC_10` < `DSC_100`) over NFC-normalised names, ties broken by
  the relative path.
- **Preview** — `core::preview::extract` takes the largest embedded preview by
  **pixel area** (spelled out rather than trusting the upstream sort, because it
  is acceptance criterion 5) and carries its EXIF orientation tag. Files with no
  preview come back `no_preview`, unparseable ones `unreadable`, both as `Error`
  values with the file named in the message.
- **Preprocess** — `core::preprocess::render`: decode, apply orientation 1–8,
  downscale in one Lanczos3 pass (`fast_image_resize` 6.1.0) to
  `--max-edge` (default 1568) only when the preview is larger, encode JPEG at
  `--jpeg-quality` (default 88). The output carries no APP1/EXIF (asserted by
  walking the segment chain). Defaults live in `Params`, and the CLI's
  `default_value_t` reads them, so there is one source of truth.
- **Dry run** — `core::dry_run::render_previews` writes one JPEG per accepted
  file into `--previews-dir` (default `./previews`), mirroring subfolders and
  replacing the RAW extension with `.jpg`. Nothing goes into the shoot folder:
  a previews directory inside it is refused as a fatal error before anything is
  written, and a test snapshots (size, mtime) of every entry in the shoot folder
  before and after. No HTTP client is constructed on this path at all.
- **CLI** — `score` gains `DIR`, `--dry-run`, `--previews-dir`, `--max-edge`,
  `--jpeg-quality`, `--recursive`, `--limit`; `doctor` gains an optional `FILE`
  and runs the preview self-test even when the server is unreachable or the
  endpoint will not parse. Argument parsing moved to `cli.rs` (with tests for
  defaults, flags and rejected values) so the CLI surface is testable without a
  terminal; `score`'s report and exit classification are pure functions over the
  export. Exit codes: 0 all previewed, 1 some frames failed, 2 fatal.
  `score` without `--dry-run` is a fatal, explicit error until ticket 03 lands
  the wire adapter.

Tests: 65 green (`cargo test`), clippy `-D warnings` clean. Coverage is at the
seams the spec names: `ingest::scan` over temp directories, `preview::extract`
and `preprocess::render` over synthetic NEF fixtures, `dry_run::render_previews`
for the whole `--dry-run` contract, `doctor::check_file` for the self-test, and
the CLI's parser and renderer in isolation.

Two things worth knowing:

- **Fixtures are synthesised, not captured.** `tests/support/mod.rs` builds the
  smallest TIFF `raw-files` will read as a Nikon NEF — IFD0 naming Nikon, an
  orientation tag, and embedded JPEG previews with their real frame sizes. That
  is what lets orientation be tested for all eight tags without a committed NEF:
  a 32x16 frame with four quadrant greys, sampled after the encode, pins where
  each tag puts each corner (the expected table comes from EXIF 2.32 §4.6.4, not
  from the implementation). The fetch-script fixtures (a real landscape, a real
  portrait, a high-ISO frame, a burst pair) are still owed by M3/M4's acceptance
  runs; nothing here needs them.
- **`.dng` is accepted but not readable by this stack.** `raw-files 0.6.0`
  sniffs NEF, ARW and JPEG only — `Format::all()` has no DNG variant — so a Z
  body shooting DNG gets a row per file marked `unreadable` rather than scores.
  The extension is accepted as this ticket asks, and the taxonomy makes the gap
  visible instead of hiding it, but a photographer on `.dng` cannot be served by
  v1's extraction as pinned. Either `raw-files` grows DNG previews (upstream is
  adding formats) or v1 documents DNG as unsupported; the spec's accepted-extension
  list and the pinned extractor disagree, and that is a decision for ticket 08
  (real-shoot validation) or a follow-up.

Also observed while running the suite under load: the existing
`tests/doctor_http.rs` stub server is timing-sensitive — under a heavily loaded
machine, 3–4 of its 9 tests intermittently fail with `Connection reset` /
`Peer disconnected` on connect, and pass on every rerun (11/11 single-file runs
green). Pre-existing, untouched by this ticket, but the stub wants a longer
accept window or a per-test single-connection design.


### Review round (two axes: standards, spec)

Both axes ran green on the substance — the .dng gap and the seam question below
were the only findings above P2 — and the cheap ones were fixed in the same
ticket:

- `Export` and `Scan` carried the same ordered file list twice (`Scan.files` and
  `frames[i].source`). The counts now travel as `ingest::ScanCounts`
  (`Scan { files, counts }`), so `Export { previews, counts, frames }` has one
  copy of the order and the counts question has one home.
- The duplicated "cannot create the previews directory" message in `dry_run` is
  one helper, and the file-name-for-a-message code lives in `preview::file_name`
  and is reused by `doctor` instead of being written twice.
- **`.dng` behaviour is now pinned**: `a_dng_gets_a_row_that_says_this_build_cannot_read_it`
  builds a DNG-shaped TIFF (`DNGVersion` set, as a camera-native or
  Adobe-converted DNG has) and asserts the row is accepted and classified
  `unreadable`. The day `raw-files` grows DNG previews, that test is the thing
  that must change.
- **`doctor <FILE>` takes `--max-edge` and `--jpeg-quality`**, so the self-test
  proves the pipeline the photographer is about to actually run rather than one
  with hard-coded defaults.
- `apply_orientation` now documents why the turn happens exactly once
  (`raw-files` reports the file's tag for each preview; `image`'s decoders never
  apply one themselves), since the fixture set cannot carry a preview with its
  own APP1 orientation.
- `tests/fixtures/README.md` documents the real captures the spec's Testing
  Decisions ask for (landscape, portrait 6/8, high-ISO, blurred, a burst pair
  with identical previews, a truncated file), how to place them, and the
  skip-loudly convention for tests that need them. None are committed.

Two findings are recorded and deliberately not fixed here:

- **The spec's "one seam" rule is only partly honoured in M1.** "No other seam
  is introduced… preview extraction, orientation, downscaling are observable
  through `Decider`" — but `Decider` arrives in ticket 03, so M1's tests enter at
  `preview::extract`, `preprocess::render` and `dry_run::render_previews`. That
  is a deliberate ordering compromise, not an oversight: those are public module
  interfaces, not internals. Ticket 03 must fold `dry_run`'s per-file step under
  the one scoring entry point, so `--dry-run` and a real run share it, and
  re-point the orientation tests at the bytes the `Decider` sees.
- **The shoot-folder guard is stricter than the spec's literal wording**, and
  deliberately so. Story 43 says v1 writes nothing into the photo folder except
  the CSV, and story 14 says previews go into a directory in the *working*
  directory — so `cd ~/shoots/wedding && imagecull score . --dry-run` is now a
  fatal error naming the fix rather than quietly writing `./previews` into the
  shot. If that friction turns out to matter, the change is one `if` in
  `dry_run::guard_outside_shoot`, but it should be a decision about the safety
  rule, not a convenience patch.
