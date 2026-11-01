//! The rubric is a file a photographer edits, so these tests treat it as one:
//! which rubrics ship inside the binary, how one is selected, what the one
//! question may say, and what the hash does when the text changes. The hash is
//! the identity of a rubric — it decides whether two CSVs may be compared and
//! whether a resume may reuse a cached answer — so its stability rules are
//! asserted here rather than discovered later.

use std::fs;
use std::path::{Path, PathBuf};

mod support;

use imagecull_core::omlx;
use imagecull_core::rubric::{Builtin, Rubric, RubricSource};
use support::{rubric, rubric_toml};

/// The one id, which is also the one column a report keeps values in.
const ONE_ID: [&str; 1] = ["stars"];

fn ids(rubric: &Rubric) -> Vec<&str> {
    rubric.questions().iter().map(|q| q.id()).collect()
}

/// The legend a shipped rubric offers, in key order.
fn legend(rubric: &Rubric) -> Vec<(String, String)> {
    rubric
        .question("stars")
        .expect("the one question")
        .criteria()
        .iter()
        .map(|criterion| {
            (
                criterion.key().to_string(),
                criterion.description().to_string(),
            )
        })
        .collect()
}

#[test]
fn the_general_rubric_is_what_a_run_uses_when_nothing_selects_another() {
    let general = Rubric::select(None).expect("the default built-in must parse");

    assert_eq!(general.builtin(), Some(Builtin::General));
    assert_eq!(ids(&general), ONE_ID);
    assert_eq!(general.model(), omlx::DEFAULT_MODEL);

    // General is the one rubric that runs before anyone said what the folder
    // holds, so it may not assume a genre: that is the whole of its difference
    // from `press` and `holiday`, whose names are their genres.
    let instructions = general
        .question("stars")
        .expect("the one question")
        .instructions()
        .to_lowercase();
    assert!(
        instructions.contains("genre"),
        "general must say it has no genre to go on: {instructions}"
    );
    for assumed in ["photojournal", "album", "client"] {
        assert!(
            !instructions.contains(assumed),
            "general must not assume `{assumed}`: {instructions}"
        );
    }
}

#[test]
fn all_three_built_ins_are_compiled_in_and_each_is_its_own_judgement() {
    assert_eq!(
        Builtin::ALL.map(Builtin::name),
        ["general", "press", "holiday"]
    );

    let mut hashes = Vec::new();
    for builtin in Builtin::ALL {
        assert_eq!(
            Builtin::from_name(builtin.name()),
            Some(builtin),
            "a built-in's name must name it back"
        );

        let shipped = builtin
            .rubric()
            .unwrap_or_else(|error| panic!("{} must parse: {}", builtin.name(), error.message()));
        assert_eq!(shipped.builtin(), Some(builtin));
        assert_eq!(
            ids(&shipped),
            ONE_ID,
            "a second rubric is a second set of words, not a second schema"
        );
        assert_eq!(
            shipped.model(),
            omlx::DEFAULT_MODEL,
            "all three were written for the installed alias"
        );
        hashes.push((builtin.name(), shipped.hash(shipped.model())));
    }

    for (index, (name, hash)) in hashes.iter().enumerate() {
        for (other, other_hash) in &hashes[index + 1..] {
            assert_ne!(
                hash, other_hash,
                "`{name}` and `{other}` are different judgements; their reports must never be comparable"
            );
        }
    }
}

#[test]
fn the_three_names_are_matched_exactly_and_a_path_is_a_path() {
    assert_eq!(
        RubricSource::parse("holiday").expect("a built-in name"),
        RubricSource::Builtin(Builtin::Holiday)
    );
    assert_eq!(Builtin::from_name("PRESS"), None, "case is not fuzzy");
    assert_eq!(
        RubricSource::parse("/Volumes/Shoot A/mine.toml").expect("a path"),
        RubricSource::File(PathBuf::from("/Volumes/Shoot A/mine.toml"))
    );
    // No separator and no extension: still a path, and the failure a run
    // reports for it names the three built-ins (see the next test).
    assert_eq!(
        RubricSource::parse("presss").expect("a path that is not a name"),
        RubricSource::File(PathBuf::from("presss"))
    );
}

#[test]
fn a_name_beats_a_file_that_happens_to_share_it() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let file = dir.path().join("holiday");
    fs::write(&file, "not a rubric").expect("write a file called holiday");

    // Selection never consults the filesystem: three known words are names, and
    // everything else is a path. The file on disk is reached by saying so.
    assert_eq!(
        RubricSource::parse("holiday").expect("a name"),
        RubricSource::Builtin(Builtin::Holiday),
        "a bare name is a built-in however the working directory is named"
    );
    assert_eq!(
        RubricSource::parse(&file.display().to_string()).expect("a path"),
        RubricSource::File(file)
    );
}

#[test]
fn a_blank_selection_is_fatal_and_lists_the_built_ins() {
    for text in ["", "   "] {
        let message = RubricSource::parse(text).expect_err("a blank selection names nothing");
        for builtin in Builtin::ALL {
            assert!(
                message.contains(builtin.name()),
                "a blank selection must say what could have been selected: {message}"
            );
        }
    }
}

#[test]
fn a_word_that_is_not_a_built_in_fails_naming_the_three() {
    let error = Rubric::select(Some(&RubricSource::File(PathBuf::from("presss"))))
        .expect_err("`presss` is not a rubric");

    assert!(
        error.is_fatal(),
        "an unreadable rubric stops the run, it does not become a row"
    );
    let message = error.message();
    for expected in ["presss".to_string()].into_iter().chain(
        Builtin::ALL
            .iter()
            .map(|builtin| builtin.name().to_string()),
    ) {
        assert!(
            message.contains(&expected),
            "the message must mention `{expected}`: {message}"
        );
    }
    assert!(
        message.to_lowercase().contains("path"),
        "and say the other way to name a rubric: {message}"
    );
}

#[test]
fn the_built_in_a_rubric_came_from_is_not_part_of_its_identity() {
    let by_name = Builtin::Holiday.rubric().expect("parse");
    let by_path = Rubric::load(&holiday_rubric_path()).expect("the file loads");

    assert_eq!(by_name.builtin(), Some(Builtin::Holiday));
    assert_eq!(
        by_path.builtin(),
        None,
        "a file was selected by path, not by name"
    );
    assert_eq!(
        by_name.hash(by_name.model()),
        by_path.hash(by_path.model()),
        "the same words are the same judgement: a name is how it was asked for, not what was judged"
    );
}

#[test]
fn a_rubric_from_a_file_remembers_no_built_in() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let path = dir.path().join("mine.toml");
    fs::write(&path, rubric_toml("clef-27b-oQ4e", "Rate for print.")).expect("write");

    let loaded = Rubric::select(Some(&RubricSource::File(path))).expect("load a rubric file");

    assert_eq!(loaded.builtin(), None);
    assert_eq!(loaded.model(), "clef-27b-oQ4e");
    assert_eq!(
        loaded.question("stars").expect("stars").instructions(),
        "Rate for print."
    );
    assert_ne!(
        loaded.hash("clef-27b-oQ4e"),
        Builtin::General
            .rubric()
            .expect("parse")
            .hash("clef-27b-oQ4e"),
        "a file override is a different rubric"
    );
}

#[test]
fn every_rubric_asks_one_question_with_five_ordered_levels() {
    for builtin in Builtin::ALL {
        let shipped = builtin.rubric().expect("parse");
        assert_eq!(
            ids(&shipped),
            ONE_ID,
            "one answer is one column and one question: {}",
            builtin.name()
        );

        let levels = legend(&shipped);
        assert_eq!(
            levels
                .iter()
                .map(|(key, _)| key.as_str())
                .collect::<Vec<_>>(),
            ["1", "2", "3", "4", "5"],
            "the keys are what the CSV's integer column parses"
        );
        assert!(
            levels
                .iter()
                .all(|(_, description)| !description.is_empty()),
            "{} must describe every level",
            builtin.name()
        );
    }
}

#[test]
fn the_legend_says_delete_not_great_okay_good_and_great() {
    for builtin in Builtin::ALL {
        let shipped = builtin.rubric().expect("parse");
        for (key, description) in legend(&shipped) {
            let anchor = match key.as_str() {
                "1" => "delete",
                "2" => "not great",
                "3" => "okay to keep",
                "4" => "good",
                "5" => "great",
                other => panic!("unexpected key `{other}`"),
            };
            assert!(
                description.starts_with(anchor),
                "{}'s level {key} must start with `{anchor}`: {description}",
                builtin.name()
            );
        }
    }
}

#[test]
fn the_press_rubric_judges_a_shoot_for_a_photo_editor() {
    let press = Builtin::Press.rubric().expect("parse");
    let instructions = press
        .question("stars")
        .expect("the one question")
        .instructions()
        .to_lowercase();

    for expected in ["photo editor", "keeper", "distinctive moment"] {
        assert!(
            instructions.contains(expected),
            "press is the photojournalistic cull and must still say so (`{expected}`): {instructions}"
        );
    }
}

/// The holiday rubric, loaded by name and by path: the two ways a run can reach
/// the same words, and the reason the hash may not cover the name.
fn holiday_rubric() -> Rubric {
    Builtin::Holiday
        .rubric()
        .expect("the built-in holiday rubric parses")
}

fn holiday_rubric_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("rubrics/holiday.toml")
}

#[test]
fn the_holiday_rubric_judges_a_frame_as_a_family_album_keeper() {
    let holiday = holiday_rubric();

    let instructions = holiday
        .question("stars")
        .expect("the one question")
        .instructions()
        .to_lowercase();
    for expected in ["family", "album", "landscape"] {
        assert!(
            instructions.contains(expected),
            "the question must judge a holiday frame as a family-album keeper, naming \
             `{expected}`: {instructions}"
        );
    }
}

#[test]
fn the_same_schema_in_different_toml_formatting_hashes_the_same() {
    let plain = rubric("clef-flash-oQ4e", "Rate the frame.");
    let reformatted = Rubric::from_toml(
        "# a comment, different whitespace, and the keys in another order\n\
         model    =   'clef-flash-oQ4e'\n\
         question = [\n\
         \x20 { type = 'choice', id = 'stars', \
         criteria = [{ key = '1', description = 'delete' }, \
         { key = '2', description = 'not great' }, \
         { key = '3', description = 'okay to keep' }, \
         { key = '4', description = 'good' }, \
         { key = '5', description = 'great' }], \
         instructions = 'Rate the frame.' }\n\
         ]\n",
    )
    .expect("the reformatted rubric is the same schema");

    assert_eq!(
        plain.hash("clef-flash-oQ4e").as_str(),
        reformatted.hash("clef-flash-oQ4e").as_str(),
        "whitespace, comments, quotes and key order are not part of the rubric"
    );
}

#[test]
fn changing_an_instruction_a_criterion_or_the_model_changes_the_hash() {
    let base = rubric("clef-flash-oQ4e", "Rate the frame.");
    let base_hash = base.hash("clef-flash-oQ4e");

    let reworded = rubric("clef-flash-oQ4e", "Rate this photograph.");
    assert_ne!(reworded.hash("clef-flash-oQ4e"), base_hash);

    let reworded_level =
        Rubric::from_toml(&rubric_toml("clef-flash-oQ4e", "Rate the frame.").replace(
            "description = \"okay to keep\"",
            "description = \"keep it\"",
        ))
        .expect("a valid rubric");
    assert_ne!(reworded_level.hash("clef-flash-oQ4e"), base_hash);

    assert_ne!(
        base.hash("clef-27b-oQ4e"),
        base_hash,
        "the alias is part of the effective rubric"
    );
}

#[test]
fn the_hash_is_sha256_hex_and_its_short_form_is_the_first_eight_characters() {
    let hash = rubric("clef-flash-oQ4e", "Rate the frame.").hash("clef-flash-oQ4e");

    assert_eq!(hash.as_str().len(), 64, "sha256 is 32 bytes of hex");
    assert!(
        hash.as_str()
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
    );
    assert_eq!(hash.short(), &hash.as_str()[..8]);
}

#[test]
fn a_broken_rubric_is_a_fatal_error_that_says_what_is_wrong() {
    let valid = rubric_toml("clef-flash-oQ4e", "Rate the frame.");
    let cases: [(&str, &str, String); 9] = [
        (
            "an id a cull does not report",
            "rating",
            valid.replace("id = \"stars\"", "id = \"rating\""),
        ),
        (
            "no model",
            "model",
            valid.replace("model = \"clef-flash-oQ4e\"", ""),
        ),
        (
            "the question as a score",
            "score",
            valid.replace("type = \"choice\"", "type = \"score\""),
        ),
        (
            "four options",
            "must offer",
            valid.replace("{ key = \"5\", description = \"great\" },", ""),
        ),
        (
            "a key off the legend",
            "must offer",
            valid.replace(
                "{ key = \"5\", description = \"great\" }",
                "{ key = \"0\", description = \"great\" }",
            ),
        ),
        (
            "an undescribed level",
            "describe",
            valid.replace("description = \"delete\"", "description = \"\""),
        ),
        (
            "a misspelled field",
            "instrcutions",
            valid.replace(
                "instructions = \"Rate the frame.\"",
                "instrcutions = \"Rate the frame.\"",
            ),
        ),
        (
            "an empty instruction",
            "instructions",
            valid.replace(
                "instructions = \"Rate the frame.\"",
                "instructions = \"  \"",
            ),
        ),
        (
            "a second question",
            "exactly one",
            format!(
                "{valid}\n[[question]]\nid = \"stars\"\ntype = \"choice\"\ninstructions = \"Again.\"\ncriteria = [\n    {{ key = \"1\", description = \"delete\" }},\n    {{ key = \"2\", description = \"not great\" }},\n    {{ key = \"3\", description = \"okay to keep\" }},\n    {{ key = \"4\", description = \"good\" }},\n    {{ key = \"5\", description = \"great\" }},\n]\n"
            ),
        ),
    ];

    for (case, expected, text) in cases {
        let error = Rubric::from_toml(&text).expect_err(&format!("{case} must be rejected"));
        assert!(
            error.is_fatal(),
            "{case}: a bad rubric stops the run, it does not become a row"
        );
        let message = error.message().to_lowercase();
        assert!(
            message.contains(expected),
            "{case}: the message must mention '{expected}', got: {message}"
        );
    }
}
