//! Reading the model's answer document: canned and adversarial bodies, with no
//! socket and no model.
//!
//! The request half of the same contract — the body a frame becomes — is
//! asserted in `systemone_http.rs`, on what the real client put on the wire.

use imagecull_core::error::ErrorKind;
use imagecull_core::reading::{self, Verdict};
use imagecull_core::rubric::Rubric;
use imagecull_core::scoring::Response;
use serde_json::{Value, json};

fn rubric() -> Rubric {
    Rubric::embedded().expect("the embedded rubric parses")
}

/// A canned answer document, with all six answers as the reference server
/// sends them: object-keyed probabilities, float scores, string choices.
fn document() -> Value {
    json!({
        "model": "clef-flash-oQ4e",
        "answers": {
            "overall": {
                "type": "score",
                "score": 6.4237,
                "confidence": 0.41,
                "legend": {"0": "delete", "10": "portfolio-grade"},
                "probabilities": {"0": 0.01, "6": 0.41, "7": 0.33}
            },
            "technical": {
                "type": "score",
                "score": 7.8125,
                "confidence": 0.55,
                "legend": {"0": "delete", "10": "portfolio-grade"},
                "probabilities": [0.0, 0.05, 0.4, 0.55]
            },
            "composition": {
                "type": "score",
                "score": 5.3398,
                "confidence": 0.33,
                "legend": {"0": "delete", "10": "portfolio-grade"},
                "probabilities": {"0": 0.02, "5": 0.33}
            },
            "impact": {
                "type": "score",
                "score": 4.9601,
                "confidence": 0.62,
                "legend": {"0": "delete", "10": "portfolio-grade"},
                "probabilities": {"4": 0.62, "5": 0.38}
            },
            "stars": {
                "type": "choice",
                "choice": "4",
                "confidence": 0.63,
                "probabilities": {"3": 0.12, "4": 0.63, "5": 0.25}
            },
            "verdict": {
                "type": "choice",
                "choice": "like",
                "confidence": 0.88,
                "probabilities": {"dislike": 0.02, "neutral": 0.10, "like": 0.88}
            }
        },
        "usage": {"input_tokens": 1327, "output_tokens": 0}
    })
}

fn read(document: Value) -> reading::Reading {
    reading::read(
        &rubric(),
        Response {
            document,
            client_latency_ms: 900,
        },
    )
}

fn failure(document: Value) -> imagecull_core::Error {
    read(document)
        .answers
        .expect_err("this document cannot be read")
}

#[test]
fn a_canned_answer_reads_back_all_six_at_full_precision() {
    let reading = read(document());
    let answers = reading.answers.expect("six readable answers");

    assert_eq!(answers.overall, 6.4237);
    assert_eq!(answers.technical, 7.8125);
    assert_eq!(answers.composition, 5.3398);
    assert_eq!(answers.impact, 4.9601);
    assert_eq!(
        (answers.composition, answers.impact),
        (5.3398, 4.9601),
        "the four are recorded apart and never rescaled to 100"
    );
    assert_eq!(answers.stars, 4);
    assert_eq!(answers.verdict, Verdict::Like);

    assert_eq!(answers.confidence.overall, 0.41);
    assert_eq!(answers.confidence.stars, 0.63);
    assert_eq!(answers.confidence.verdict, 0.88);

    assert_eq!(reading.usage.input_tokens, Some(1327));
    assert_eq!(
        reading.document,
        document(),
        "the raw document is preserved for --json-scores"
    );
}

#[test]
fn probabilities_are_display_only_whether_they_arrive_as_an_array_or_an_object() {
    // The canned document already mixes both shapes: `technical` sends an
    // array, the others objects. Both read.
    assert!(read(document()).answers.is_ok());

    // And neither is ever a source for the recorded answer: a document that
    // only has probabilities cannot be scored, however clear they look.
    let mut without_score = document();
    without_score["answers"]["overall"]
        .as_object_mut()
        .expect("an answer object")
        .remove("score");
    let error = failure(without_score);
    assert_eq!(error.kind(), Some(ErrorKind::BadResponse));
    assert!(error.message().contains("`overall`"));
    assert!(error.message().contains("score"));
}

#[test]
fn a_score_may_be_a_whole_number_and_a_choice_is_always_the_rubric_key() {
    let mut whole = document();
    whole["answers"]["overall"]["score"] = json!(6);
    assert_eq!(read(whole).answers.expect("readable").overall, 6.0);

    let mut string_score = document();
    string_score["answers"]["overall"]["score"] = json!("6.4");
    let error = failure(string_score);
    assert_eq!(error.kind(), Some(ErrorKind::BadResponse));
    assert!(error.message().contains("numeric `score`"));
}

#[test]
fn an_answer_key_that_is_missing_or_off_the_rubric_is_a_bad_response() {
    let mut missing = document();
    missing["answers"]
        .as_object_mut()
        .expect("answers object")
        .remove("composition");
    let error = failure(missing);
    assert_eq!(error.kind(), Some(ErrorKind::BadResponse));
    assert!(error.message().contains("no answer for `composition`"));

    let mut off_legend = document();
    off_legend["answers"]["stars"]["choice"] = json!("6");
    let error = failure(off_legend);
    assert_eq!(error.kind(), Some(ErrorKind::BadResponse));
    assert!(error.message().contains("`6`"));

    let mut off_verdict = document();
    off_verdict["answers"]["verdict"]["choice"] = json!("love");
    let error = failure(off_verdict);
    assert_eq!(error.kind(), Some(ErrorKind::BadResponse));
    assert!(error.message().contains("`love`"));
}

#[test]
fn a_missing_confidence_or_a_wrongly_shaped_answer_is_a_bad_response() {
    let mut no_confidence = document();
    no_confidence["answers"]["stars"]
        .as_object_mut()
        .expect("an answer")
        .remove("confidence");
    let error = failure(no_confidence);
    assert_eq!(error.kind(), Some(ErrorKind::BadResponse));
    assert!(error.message().contains("`stars`"));
    assert!(error.message().contains("confidence"));

    let mut no_answers = json!({"model": "clef-flash-oQ4e", "usage": {"input_tokens": 10}});
    let error = failure(no_answers.clone());
    assert_eq!(error.kind(), Some(ErrorKind::BadResponse));
    assert!(error.message().contains("`answers`"));

    no_answers["answers"] = json!([{"id": "overall"}]);
    let error = failure(no_answers);
    assert_eq!(error.kind(), Some(ErrorKind::BadResponse));
}

#[test]
fn a_failed_reading_keeps_the_document_it_could_not_read() {
    let reading = read(json!({"answers": {"overall": {"score": 6.0}}}));
    assert!(reading.answers.is_err());
    assert_eq!(
        reading.document,
        json!({"answers": {"overall": {"score": 6.0}}}),
        "a bad response still owes --json-scores its raw document"
    );
}

#[test]
fn latency_is_the_servers_when_it_reports_one_and_ours_otherwise() {
    let ours = read(document());
    assert_eq!(ours.usage.server_latency_ms, None);
    assert_eq!(ours.usage.latency_ms(), 900, "client-measured");

    let mut served = document();
    served["usage"]["latency_ms"] = json!(812);
    let reading = read(served);
    assert_eq!(reading.usage.server_latency_ms, Some(812));
    assert_eq!(reading.usage.latency_ms(), 812);

    let mut unmeasured = document();
    unmeasured["usage"] = json!({});
    let reading = read(unmeasured);
    assert_eq!(reading.usage.input_tokens, None);
    assert!(reading.answers.is_ok(), "no usage is not a bad answer");
}
