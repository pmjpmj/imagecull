//! The rubric is a file a photographer edits, so these tests treat it as one:
//! which rubrics ship inside the binary, how one is selected, and what the hash
//! does when the text changes. The hash is the identity of a rubric — it decides
//! whether two CSVs may be compared and whether a resume may reuse a cached
//! score — so its stability rules are asserted here rather than discovered later.

use std::fs;
use std::path::{Path, PathBuf};

mod support;

use imagecull_core::omlx;
use imagecull_core::rubric::{Builtin, Criterion, QuestionKind, Rubric, RubricSource};
use support::{rubric, rubric_toml};

/// The six ids, in the order every report has a column for.
const SIX_IDS: [&str; 6] = [
    "overall",
    "technical",
    "composition",
    "impact",
    "stars",
    "verdict",
];

fn ids(rubric: &Rubric) -> Vec<&str> {
    rubric.questions().iter().map(|q| q.id()).collect()
}

#[test]
fn the_general_rubric_is_what_a_run_uses_when_nothing_selects_another() {
    let general = Rubric::select(None).expect("the default built-in must parse");

    assert_eq!(general.builtin(), Some(Builtin::General));
    assert_eq!(ids(&general), SIX_IDS);
    assert_eq!(general.model(), omlx::DEFAULT_MODEL);

    // General is the one rubric that runs before anyone said what the folder
    // holds, so it may not assume a genre: that is the whole of its difference
    // from `press` and `holiday`, whose names are their genres.
    let overall = general
        .question("overall")
        .expect("a score question")
        .instructions()
        .to_lowercase();
    assert!(
        overall.contains("genre"),
        "general must say it has no genre to go on: {overall}"
    );
    for assumed in ["photojournal", "album", "client"] {
        assert!(
            !overall.contains(assumed),
            "general must not assume `{assumed}`: {overall}"
        );
    }

    // Four scores with one shared legend is what ticket 08 measured as keeping
    // the axes collapsed. General is new, so nothing is invalidated by writing
    // each axis its own.
    let legends: Vec<Vec<&str>> = ["overall", "technical", "composition", "impact"]
        .iter()
        .map(|id| {
            general
                .question(id)
                .expect("a required score question")
                .criteria()
                .iter()
                .map(Criterion::description)
                .collect()
        })
        .collect();
    for (index, legend) in legends.iter().enumerate() {
        for other in legends.iter().skip(index + 1) {
            assert_ne!(
                legend, other,
                "the legend is the question: these two axes must not share one"
            );
        }
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
            SIX_IDS,
            "a second rubric is a second set of words, not a second schema"
        );
        assert_eq!(
            shipped.model(),
            omlx::DEFAULT_MODEL,
            "all three were written for the installed alias"
        );
        assert_the_four_axes_are_independent(&shipped);
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
        loaded.question("overall").expect("overall").instructions(),
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
fn the_four_scores_are_independent_questions_with_a_zero_to_ten_legend() {
    for builtin in Builtin::ALL {
        let shipped = builtin.rubric().expect("parse");
        for id in ["overall", "technical", "composition", "impact"] {
            let question = shipped.question(id).expect("a required score question");
            assert_eq!(question.kind(), QuestionKind::Score, "{id} must be a score");
            let keys: Vec<&str> = question.criteria().iter().map(Criterion::key).collect();
            assert_eq!(
                keys,
                ["0", "1", "2", "3", "4", "5", "6", "7", "8", "9", "10"],
                "{id} of {} needs an ordered 0-10 legend",
                builtin.name()
            );
            assert!(
                question
                    .criteria()
                    .iter()
                    .all(|criterion| !criterion.description().is_empty()),
                "{id} must describe every level"
            );
        }
    }
}

/// Without this, four "independent" scores collapse into one taste: the model
/// has to be told, per question, which axes are somebody else's. Every rubric
/// shipped with the tool owes this, so every one is held to it here.
fn assert_the_four_axes_are_independent(rubric: &Rubric) {
    for (id, others) in [
        ("technical", ["composition", "impact"]),
        ("composition", ["technical", "impact"]),
        ("impact", ["technical", "composition"]),
    ] {
        let instructions = rubric
            .question(id)
            .expect("a required score question")
            .instructions()
            .to_lowercase();
        assert!(
            instructions.contains("ignore"),
            "{id} must tell the model what to ignore: {instructions}"
        );
        for other in others {
            assert!(
                instructions.contains(other),
                "{id} must name `{other}` as another question's job: {instructions}"
            );
        }
    }
}

#[test]
fn the_press_rubric_judges_a_shoot_for_a_photo_editor() {
    let press = Builtin::Press.rubric().expect("parse");
    let overall = press
        .question("overall")
        .expect("a score question")
        .instructions()
        .to_lowercase();

    for expected in ["photo editor", "keeper", "distinctive moment"] {
        assert!(
            overall.contains(expected),
            "press is the photojournalistic cull and must still say so (`{expected}`): {overall}"
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
    assert_the_four_axes_are_independent(&holiday);

    let overall = holiday
        .question("overall")
        .expect("a score question")
        .instructions()
        .to_lowercase();
    for expected in ["family", "album", "landscape"] {
        assert!(
            overall.contains(expected),
            "overall must judge a holiday frame as a family-album keeper, naming `{expected}`: \
             {overall}"
        );
    }

    // Each score carries its own legend here, so a technical 3 and a
    // composition 3 diagnose different things. The press rubric shares one
    // legend across all four; this one must not be that copy.
    let legends: Vec<Vec<&str>> = ["overall", "technical", "composition", "impact"]
        .iter()
        .map(|id| {
            let question = holiday.question(id).expect("a required score question");
            question
                .criteria()
                .iter()
                .map(Criterion::description)
                .collect()
        })
        .collect();
    for (index, legend) in legends.iter().enumerate() {
        for other in legends.iter().skip(index + 1) {
            assert_ne!(
                legend, other,
                "the legend is the question: these two axes must not share one"
            );
        }
    }
}

#[test]
fn stars_is_five_ranked_options_and_verdict_is_the_three_way_choice() {
    for builtin in Builtin::ALL {
        let shipped = builtin.rubric().expect("parse");

        let stars = shipped.question("stars").expect("a stars question");
        assert_eq!(stars.kind(), QuestionKind::Choice);
        assert_eq!(
            stars
                .criteria()
                .iter()
                .map(Criterion::key)
                .collect::<Vec<_>>(),
            ["1", "2", "3", "4", "5"],
            "the star keys are what the CSV's integer column parses"
        );

        let verdict = shipped.question("verdict").expect("a verdict question");
        assert_eq!(verdict.kind(), QuestionKind::Choice);
        assert_eq!(
            verdict
                .criteria()
                .iter()
                .map(Criterion::key)
                .collect::<Vec<_>>(),
            ["dislike", "neutral", "like"]
        );
    }
}

#[test]
fn the_same_schema_in_different_toml_formatting_hashes_the_same() {
    let plain = rubric("clef-flash-oQ4e", "Rate the frame.");
    let reformatted = Rubric::from_toml(&format!(
        "# a comment, different whitespace, and the keys in another order\n\
         model    =   'clef-flash-oQ4e'\n\
         question = [\n{questions}\n]\n",
        questions = [
            ("overall", "Rate the frame."),
            ("technical", "Judge technical quality only."),
            ("composition", "Judge composition only."),
            ("impact", "Judge impact only."),
            ("stars", "How many stars?"),
            ("verdict", "Would you keep it?"),
        ]
        .iter()
        .map(|(id, instructions)| {
            let legend = (0..=10)
                .map(|level| format!("{{ key = \"{level}\", description = \"level {level}\" }}"))
                .collect::<Vec<_>>()
                .join(", ");
            match *id {
                "overall" | "technical" | "composition" | "impact" => format!(
                    "  {{ type = 'score', id = '{id}', criteria = [{legend}], \
                     instructions = '{instructions}' }}"
                ),
                "stars" => "  { type = 'choice', id = 'stars', instructions = 'How many stars?', \
                     criteria = [{ key = '1', description = 'reject' }, \
                     { key = '2', description = 'poor' }, \
                     { key = '3', description = 'ok' }, \
                     { key = '4', description = 'strong' }, \
                     { key = '5', description = 'excellent' }] }"
                    .to_string(),
                _ => "  { type = 'choice', id = 'verdict', \
                     instructions = 'Would you keep it?', \
                     criteria = [{ key = 'dislike', description = 'no' }, \
                     { key = 'neutral', description = 'maybe' }, \
                     { key = 'like', description = 'yes' }] }"
                    .to_string(),
            }
        })
        .collect::<Vec<_>>()
        .join(",\n")
    ))
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

    let reordered = Rubric::from_toml(&rubric_toml("clef-flash-oQ4e", "Rate the frame.").replace(
        "{ key = \"3\", description = \"level 3\" }",
        "{ key = \"3\", description = \"level three\" }",
    ))
    .expect("a valid rubric");
    assert_ne!(reordered.hash("clef-flash-oQ4e"), base_hash);

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
    let cases: [(&str, &str, String); 8] = [
        (
            "missing stars",
            "stars",
            valid.replace("id = \"stars\"", "id = \"rating\""),
        ),
        (
            "no model",
            "model",
            valid.replace("model = \"clef-flash-oQ4e\"", ""),
        ),
        (
            "overall as a choice",
            "overall",
            valid.replace(
                "id = \"overall\"\ntype = \"score\"",
                "id = \"overall\"\ntype = \"choice\"",
            ),
        ),
        (
            "stars with four options",
            "stars",
            valid.replace("{ key = \"5\", description = \"excellent\" },", ""),
        ),
        (
            "verdict with the wrong keys",
            "verdict",
            valid.replace("{ key = \"like\", description = \"yes\" }", "{ key = \"love\", description = \"yes\" }"),
        ),
        (
            "a legend that starts at one",
            "overall",
            valid.replace(
                "{ key = \"0\", description = \"level 0\" }, ",
                "{ key = \"1\", description = \"level 1\" }, { key = \"0\", description = \"level 0\" }, ",
            ),
        ),
        (
            "a misspelled field",
            "instrcutions",
            valid.replace("instructions = \"Rate the frame.\"", "instrcutions = \"Rate the frame.\""),
        ),
        (
            "an empty instruction",
            "overall",
            valid.replace("instructions = \"Rate the frame.\"", "instructions = \"  \""),
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
