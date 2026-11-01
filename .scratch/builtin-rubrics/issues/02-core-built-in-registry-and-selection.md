# 02: Built-in rubrics in the core, and one setting that holds a name or a path

**What to build:** `Builtin` (three variants, `name`, `from_name`, `ALL`,
`DEFAULT`) and `RubricSource` (`Builtin(name)` or `File(path)`, with `parse` and
`anchored`) in `rubric.rs`; `Builtin::rubric()` parses the compiled-in file;
`Rubric::select(Option<&RubricSource>)` is the one entry point, `None` being
`general`. `Rubric` records `Option<Builtin>` as provenance.

Name matching is exact and lowercase-only and never consults the filesystem, so
a bare name is a built-in however the working directory is named and `./holiday`
is unambiguously the file. A blank value is fatal and lists the three names; a
bare word that cannot be opened is told what the three names are.

**Status:** resolved
**Blocked by:** 01

- [x] `Builtin`/`RubricSource`/`Rubric::select`, `Rubric::load(path)`
- [x] the name is not part of the hash: a name and a path to the same words hash
      equal, and `builtin()` still tells them apart
- [x] blank and unreadable-selection errors name the three built-ins
- [x] `Rubric::embedded()` replaced by `Builtin::DEFAULT.rubric()` everywhere

## Answer

`crates/imagecull-core/src/rubric.rs`, exported from `lib.rs`. `tests/rubric.rs`
covers selection, the exact-match rule, the blank and mistyped-name messages,
and identity-vs-provenance. `Rubric::builtin` was split: the constructor lives on
`Builtin` (`Builtin::Holiday.rubric()`), because `Rubric` needs `builtin()` as
the accessor.
