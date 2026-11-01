//! The rubric is a file a photographer edits, so these tests treat it as one:
//! what the embedded default asks, and what the hash does when the text
//! changes. The hash is the identity of a rubric — it decides whether two CSVs
//! may be compared and whether a resume may reuse a cached score — so its
//! stability rules are asserted here rather than discovered later.

use std::fs;

mod support;

use imagecull_core::omlx;
use imagecull_core::rubric::{Criterion, QuestionKind, Rubric};
use support::{rubric, rubric_toml};

#[test]
fn the_embedded_rubric_asks_the_six_questions_in_declaration_order() {
    let embedded = Rubric::embedded().expect("the shipped rubric must parse");

    let ids: Vec<&str> = embedded.questions().iter().map(|q| q.id()).collect();
    assert_eq!(
        ids,
        [
            "overall",
            "technical",
            "composition",
            "impact",
            "stars",
            "verdict"
        ]
    );
    assert_eq!(embedded.model(), omlx::DEFAULT_MODEL);
}

#[test]
fn the_four_scores_are_independent_questions_with_a_zero_to_ten_legend() {
    let embedded = Rubric::embedded().expect("parse");

    for id in ["overall", "technical", "composition", "impact"] {
        let question = embedded.question(id).expect("a required score question");
        assert_eq!(question.kind(), QuestionKind::Score, "{id} must be a score");
        let keys: Vec<&str> = question.criteria().iter().map(Criterion::key).collect();
        assert_eq!(
            keys,
            ["0", "1", "2", "3", "4", "5", "6", "7", "8", "9", "10"],
            "{id} needs an ordered 0-10 legend"
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

#[test]
fn the_other_axes_are_told_to_ignore_each_other() {
    let embedded = Rubric::embedded().expect("parse");

    // Without this, four "independent" scores collapse into one taste: the
    // model has to be told, per question, which axes are somebody else's.
    for (id, others) in [
        ("technical", ["composition", "impact"]),
        ("composition", ["technical", "impact"]),
        ("impact", ["technical", "composition"]),
    ] {
        let instructions = embedded
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

    let overall = embedded.question("overall").expect("a score question");
    assert!(
        overall.instructions().to_lowercase().contains("keeper")
            && overall
                .instructions()
                .to_lowercase()
                .contains("distinctive moment"),
        "overall is framed around photojournalistic keepers: {}",
        overall.instructions()
    );
}

#[test]
fn stars_is_five_ranked_options_and_verdict_is_the_three_way_choice() {
    let embedded = Rubric::embedded().expect("parse");

    let stars = embedded.question("stars").expect("a stars question");
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

    let verdict = embedded.question("verdict").expect("a verdict question");
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
fn a_rubric_file_overrides_the_embedded_one() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let path = dir.path().join("mine.toml");
    fs::write(&path, rubric_toml("clef-27b-oQ4e", "Rate for print.")).expect("write");

    let loaded = Rubric::load(Some(&path)).expect("load a rubric file");

    assert_eq!(loaded.model(), "clef-27b-oQ4e");
    assert_eq!(
        loaded.question("overall").expect("overall").instructions(),
        "Rate for print."
    );
    assert_ne!(
        loaded.hash("clef-27b-oQ4e"),
        Rubric::embedded().expect("parse").hash("clef-27b-oQ4e"),
        "a file override is a different rubric"
    );

    let embedded = Rubric::load(None).expect("no path loads the embedded rubric");
    assert_eq!(
        embedded.hash(omlx::DEFAULT_MODEL),
        Rubric::embedded().expect("parse").hash(omlx::DEFAULT_MODEL)
    );
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
