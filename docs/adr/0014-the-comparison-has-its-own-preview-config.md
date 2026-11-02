# 0014 — The comparison has its own preview config

Status: accepted · 2026-11-02 · Feature `preview-per-command` ·
supersedes [ADR 0010](0010-the-group-section-holds-the-grouping-thresholds.md)'s
"`[preview] max_edge` still does not reach a cull" and its out-of-scope bullet ·
amends `SPEC.md` §6.4 and §11.1

## Context

ADR 0010 put the grouping thresholds in `[group]` and drew a line around the
comparison's request shape: `[preview] jpeg_quality` reached a cull, `[preview]
max_edge` did not, and the edge stayed at the measured `keeper::EDGE` (512 px)
inside `keeper::Budget` — a struct whose doc said a run does not take its numbers
from the command line "because a photographer choosing them would be choosing how
much picture the model gets to see, which is a fact about the model".

Three things make that line wrong now.

The first is that it left one preview half-configurable. A photographer running a
dim shoot at `--jpeg-quality 95` was configuring the comparison's compression and
not its size, which is the half that matters: quality moves bytes on the loopback
wire and nothing else, while the edge moves tokens, detail and how many frames fit
one request.

The second is the failure a fixed edge produces. A group of 20 at 512 px is 4,154
of the reference server's 16,384 image tokens. At 1568 px it does not fit, and the
server answers `413`, so a group is reported undecided and nothing is marked. That
is the right outcome for a run that asked for 1568 — but `--max-edge` could not
ask, and the `413`'s message could name no remedy, because the group's only
remedy is the number that was fixed.

The third is what the recorded reason actually claimed. "A fact about the model"
is true of the token cap (20 frames) and of the confidence floor (0.60): those are
measurements of the model's answer quality. It is not true of the edge in the same
way — the measurement (`.scratch/burst-grouping-v1/evidence/comparison-budget.md`)
says 512 px was *decisive* on a real difference and 3× cheaper than 1024, which is
a good default and not a law. A photographer who has one undecidable group, or a
machine with tokens to spare, is asking a question the tool can answer with a key.

## Decision

**Two preview pairs, one per kind of request.** `[preview] max_edge` /
`jpeg_quality` stay the single-frame pair `score` and `doctor` read. `[group]`
gains `max_edge` and `jpeg_quality` — the payload shape of **one image** in a
comparison — read by `groups`. Neither reaches the other command: a rating's
1568 px and a comparison's 512 px are different payloads, and one key cannot be
both (ADR 0010's own argument against misfiled keys, applied to itself).

**The edge leaves `keeper::Budget`.** `Budget` keeps what is measured about the
comparison's answer — `max` frames per request and the confidence `floor` —
because those are facts about the model. The payload shape becomes
`cull::Config::preview: preprocess::Params`, defaulting to
`keeper::default_preview()` (512 px at the shared quality of 88). `cull` builds
each request's `Params` from it, exactly as `scoring` builds a rating's.

**The flags are named per command, not per section.** `groups --max-edge` and
`groups --jpeg-quality` mean the comparison's pair, the way `score --max-edge`
means a rating's. The alternative — `--group-max-edge`, matching the `--group-*`
thresholds — was rejected because it invents a second vocabulary for two knobs
that already have names, and because a flag is scoped by the command it follows.

**The keys land on the `group_` side of `Resolved`.** `group_max_edge` and
`group_jpeg_quality` resolve flag-over-file-over-default through the one merge
everything else uses, so no key is reachable from a file without passing
`Resolved`, and `[group]`'s defaults stay core constants
(`keeper::default_preview()`), not a second copy in the CLI.

**A `413` on a comparison now names its remedy.** With the edge configurable the
undecided group's message says a lower `--max-edge` (or `[group] max_edge`) is how
to ask for less picture. It still refuses the request rather than quietly
re-rendering it smaller: a run that silently changed its own numbers would report
a verdict for a question nobody asked.

## Considered options

- **`[group.preview]` / `[preview.group]` nested tables.** Rejected by the owner in
  favour of flat keys in `[group]`: the section already reads as "what a cull
  does", and four flat keys beside three thresholds is one table to explain, not
  two.
- **Moving both pairs under `[preview.score]` and `[preview.group]`.** Symmetric,
  but it would break every config in the wild that sets `[preview] max_edge`, or
  need alias support for a shape with no other user. A new key beside the old one
  changes nothing for a file that does not use it.
- **Keeping `[preview] jpeg_quality` reaching a cull.** Then a cull's two numbers
  would come from two sections, and `--jpeg-quality` on `groups` would have to
  write a `[preview]` key — the exact coupling the split is for.
- **A quality pair only, leaving the edge measured.** Cheapest, but it makes the
  half that moves tokens unconfigurable, which is the request this ADR answers.
- **Leaving `Budget` as-is and filling `edge` from config.** `Budget` would then
  hold one measured value and two run-chosen ones while documenting itself as the
  measured shape. Splitting the struct is the honest form of the same change.
- **A per-group override (`--group-max-edge` applying to one group).** Nothing in
  the run knows which group will overflow before the requests are built, and
  retrying a refused group smaller is what the `413` refusal exists to prevent.

## Consequences

- `[preview] max_edge` and `[preview] jpeg_quality` no longer reach a cull; a
  shoot config that set a non-default `jpeg_quality` for a cull now needs it under
  `[group]`. The default is the same 88, so only a file that moved the value sees
  a difference.
- `groups --max-edge 1024` is a way to make a group undecidable on purpose, and
  the `413` says so. The comparison's `MAX` and `FLOOR` remain measured, so
  "how many frames" and "how sure is sure" stay facts about the model.
- `cull::Config` carries `preview: Params` and no longer a bare `jpeg_quality`, so
  every caller names one thing (the payload shape) instead of two.
- `keeper::Budget` loses `edge`, and `keeper::EDGE` becomes the default rather
  than the only value; the module's budget docs now say which number is which.
- `SPEC.md` §11.1 records `groups` as a reader of all of `[group]` and no longer
  as a reader of `[preview]`; `imagecull.toml.example`, the README table and the
  glossary's Preview entry say which pair belongs to which command.
- ADR 0003's "a section mirrors the core type it fills" now has one section
  filling two (`[group]` → `Thresholds` and `Params`). The principle it was
  reaching for survives — a section owns what one command needs — but the
  one-for-one phrasing no longer describes the file.

**ADR 0016 took the option this ADR rejected.** "Moving both pairs under a
per-command subsection" was declined here because it "would break every config in
the wild that sets `[preview] max_edge`, or need alias support". The move has now
happened, and neither cost is paid silently: `[preview]`, `[outputs]` and `[run]`
are tombstones that refuse any key by name and point at its `[score]` home, so the
migration is one error message per file and there is still one spelling per key.
The preview split itself — two pairs, one per kind of request, `[group]`'s pair
defaulting to the measured 512 px — stands unchanged; it is now `[score] max_edge`
beside `[group] max_edge`.

**ADR 0018 moved `max` as well, and kept `floor`.** `Budget` held `max` and
`floor` as "facts about the model" a run could not move. Half of that survived:
the confidence floor is a measurement of the answer's quality, and it is still a
constant, but twenty frames per request was always the *reference server's*
measured batch. Workers AI takes four, so `[group] max_images` sets it and
`Budget::max` is a default a run may lower — the same argument this ADR made for
the edge, arriving at the other number. `chunk` splits a group by the cap, so a
smaller batch costs requests and changes no verdict.
