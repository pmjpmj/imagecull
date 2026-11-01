# 04: Say it in the spec, the glossary and the ADR

**What to build:** the documents that make the change legible a year later.
`SPEC.md` §8 (three built-ins, `general` the default, a name is provenance),
§11 (the flag's shape, `--version`), §11.1 (`[run] rubric` takes a name or a
path; a name is not anchored). `GLOSSARY.md` gains *built-in rubric* and
*general rubric*, and retires "default rubric". ADR 0004 records the family, the
one-setting choice and the identity rule. Ticket 08's freeze target becomes "the
press rubric", since the press wording is no longer the default.

**Status:** resolved
**Blocked by:** 01, 02, 03

- [x] `SPEC.md` §8, §11, §11.1 and the source-tree sketch
- [x] `GLOSSARY.md`: *built-in rubric*, *general rubric*, the hash note, the
      effective-config list
- [x] `docs/adr/0004-built-in-rubric-family.md`
- [x] `.scratch/nef-scoring-v1/issues/08-…` reworded to freeze `press`
