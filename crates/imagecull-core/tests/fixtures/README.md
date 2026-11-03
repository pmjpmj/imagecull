# Real-capture fixtures

Tests in this directory build their RAW files synthetically
(`tests/support/mod.rs`): the smallest TIFF `raw-files` will read as a Nikon
NEF, with embedded JPEG previews at real frame sizes. That covers everything the
pipeline does to a preview without committing tens of megabytes.

The burst-grouping tests generate their own answer key the same way
(`tests/support/burst.rs`): one scene — a bright sky over a textured ground,
across a level horizon, with one deliberately soft patch — and the transforms a
burst goes wrong with (a Gaussian blur, a 5 px horizontal motion blur, a known
tilt, +2 EV), each with a known expected reading. No JPEG is committed, because a
generator is deterministic, reviewable as code, and can draw the same scene at two
sensor sizes.

What synthetic files cannot cover is the camera's own quirks. These are the real
captures the spec's Testing Decisions ask for (`.scratch/nef-scoring-v1/spec.md`,
"Fixtures"), and they are **not committed**:

| file | what it exercises |
|---|---|
| `landscape-1.nef` | EXIF orientation 1, the common case |
| `portrait-6.nef` | orientation 6, a portrait frame stored sideways (the regression case) |
| `portrait-8.nef` | orientation 8, the other quarter turn |
| `high-iso.nef` | a dim, noisy frame at ISO 6400 or above |
| `blurred.nef` | motion or focus blur, for M4's rubric tuning |
| `burst-a.nef`, `burst-b.nef` | two frames from one burst with byte-identical embedded previews, for dedupe |
| `truncated.nef` | a `burst-a.nef` cut off partway, for the `unreadable` / `no_preview` paths |

Place them here, from your own shoot (a card copy is enough — nothing is
written back):

```sh
# from a card or an existing library, naming them as the table above
mkdir -p crates/imagecull-core/tests/fixtures
cp /Volumes/NIKON\\ Z\\ 8/DCIM/100NZ8E/DSC_0001.NEF \
   crates/imagecull-core/tests/fixtures/landscape-1.nef
# a truncated copy, for the unreadable path
head -c 2000000 crates/imagecull-core/tests/fixtures/landscape-1.nef \
  > crates/imagecull-core/tests/fixtures/truncated.nef
```

A test that needs one of these must **skip loudly** — print one line naming the
missing fixture and this file, and return — never pass silently:

```rust
let path = Path::new("tests/fixtures/portrait-6.nef");
if !path.is_file() {
    eprintln!(
        "skipping: {} is not present — see tests/fixtures/README.md",
        path.display()
    );
    return;
}
```

Before a long run on a new camera, `imagecull score --dry-run` renders the same
previews by hand for one of these files.
