# 02: From shoot folder to sent previews

**What to build:** `imagecull score <DIR> --dry-run` takes a shoot folder and produces, for every accepted RAW, exactly the bytes that a scoring run would send to the model — written into a previews directory so the photographer can eyeball orientation, crop and detail loss before burning an hour. Along the way it scans the folder (accepted extensions case-insensitively, non-recursive unless asked, hidden directories never descended, Finder noise ignored and counted, natural NFC-normalised filename order, `--limit` for a smoke test), takes the camera's largest embedded preview measured by pixel area, applies EXIF orientation before any resizing, and downscales to the configured longest edge without ever upscaling. `doctor <FILE>` gains a preview-extraction self-test so the pipeline can be proven on the photographer's own camera output before a long run. Nothing is written into the shoot folder.

**Blocked by:** 01

**Status:** ready-for-agent

- [ ] `imagecull score <DIR> --dry-run` writes one preview per accepted file and performs no HTTP
- [ ] Extensions `.nef`, `.nrw`, `.dng` accepted case-insensitively; anything else counted as skipped, never an error
- [ ] `._*`, `.DS_Store` and anything under `__MACOSX` ignored with a reported count; hidden directories never descended; `--recursive` opt-in
- [ ] Files sorted by natural, Unicode-normalised filename so preview order and later CSV row order are stable
- [ ] The preview chosen in a file that embeds several is the one with the largest pixel area, not the largest byte count
- [ ] Orientation 1–8 applied before downscaling; a portrait fixture exports upright
- [ ] Longest edge capped by `--max-edge` (default 1568), `--jpeg-quality` (default 88) honoured, and a preview smaller than the cap is never upscaled
- [ ] Encoded JPEG carries no EXIF/APP1
- [ ] A file with no embedded preview and a truncated/garbage file each yield a classified failure without a panic, and the run continues
- [ ] `doctor <FILE>` self-tests preview extraction on a real file
- [ ] A mtime scan of the shoot folder after a run shows zero writes
