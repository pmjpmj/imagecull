# 01: The general rubric, and one file per built-in

**What to build:** `crates/imagecull-core/rubrics/general.toml` — the rubric a
run uses when nothing selects another. Six questions, the same ids, the 0-10
legends and the star/verdict keys as every other rubric, written so that no
genre is assumed: a portrait, a landscape, an event, a street frame and a
snapshot all arrive in the same folder. The standard does not move with the
genre (focus, exposure and framing are judged as themselves); what counts as the
subject and the moment is read off the frame. Four per-axis legends, not the
press rubric's shared one, because ticket 08 measured the shared legend as
keeping the four scores collapsed.

`default.toml` becomes `press.toml`: it is no longer what a run uses by default,
and a file named for a role it no longer has is a trap. `holiday.toml` stays as
it is but is now named by `--rubric holiday` rather than by path.

**Status:** resolved
**Blocked by:** none

- [x] `general.toml` authored, six questions, per-axis legends, no genre assumed
- [x] `default.toml` renamed to `press.toml`, its header saying it is the press
      cull and not the default
- [x] `holiday.toml`'s header points at `--rubric holiday`
- [x] every rubric's four axes name what they must ignore

## Answer

`general.toml`, `press.toml`, `holiday.toml` under
`crates/imagecull-core/rubrics/`. `general.toml`'s `overall` says it has no
genre to go on, reads the frame for what it is trying to be, and keeps the
standard fixed; its four legends differ from each other and from both other
rubrics. `tests/rubric.rs` holds every shipped rubric to the "name what you
ignore" invariant and asserts the three hashes are pairwise distinct.
