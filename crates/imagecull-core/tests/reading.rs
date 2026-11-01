//! Reading the model's answer document: canned and adversarial bodies, with no
//! socket and no model.
//!
//! The request half of the same contract — the body a frame becomes — is
//! asserted in `systemone_http.rs`, on what the real client put on the wire.

use imagecull_core::error::ErrorKind;
use imagecull_core::reading;
use imagecull_core::rubric::{Builtin, Rubric};
use imagecull_core::scoring::Response;
use serde_json::{Value, json};

fn rubric() -> Rubric {
    Builtin::General
        .rubric()
        .expect("the default built-in parses")
}

/// A canned answer document, with the answer as the reference server sends it:
/// an object-keyed probability map and a string choice.
fn document() -> Value {
    json!({
        "model": "clef-flash-oQ4e",
        "answers": {
            "stars": {
                "type": "choice",
                "choice": "4",
                "confidence": 0.63,
                "probabilities": {"1": 0.01, "2": 0.03, "3": 0.12, "4": 0.63, "5": 0.21}
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
fn a_canned_answer_reads_back_at_full_precision() {
    let reading = read(document());
    let answers = reading.answers.expect("a readable answer");

    assert_eq!(answers.stars, 4, "the choice key, as a number");
    assert_eq!(
        answers.confidence, 0.63,
        "the top-option probability of the choice"
    );

    assert_eq!(reading.usage.input_tokens, Some(1327));
    assert_eq!(
        reading.document,
        document(),
        "the raw document is preserved for --json-scores"
    );
}

#[test]
fn probabilities_are_display_only_whether_they_arrive_as_an_array_or_an_object() {
    // Neither shape is ever a source for the recorded answer: a document that
    // only has probabilities cannot be read, however clear they look.
    let mut without_choice = document();
    without_choice["answers"]["stars"]
        .as_object_mut()
        .expect("an answer object")
        .remove("choice");
    let error = failure(without_choice);
    assert_eq!(error.kind(), Some(ErrorKind::BadResponse));
    assert!(error.message().contains("`stars`"));
    assert!(error.message().contains("choice"));

    let mut array_probabilities = document();
    array_probabilities["answers"]["stars"]["probabilities"] = json!([0.0, 0.01, 0.03, 0.12, 0.63]);
    assert!(
        read(array_probabilities).answers.is_ok(),
        "a probability array is display-only, not a reason to reject"
    );
}

#[test]
fn a_choice_must_be_the_rubric_key_and_a_string() {
    let mut whole_number = document();
    whole_number["answers"]["stars"]["choice"] = json!(4);
    let error = failure(whole_number);
    assert_eq!(error.kind(), Some(ErrorKind::BadResponse));
    assert!(error.message().contains("no `choice` string"));
}

#[test]
fn an_answer_that_is_missing_or_off_the_rubric_is_a_bad_response() {
    let mut missing = document();
    missing["answers"]
        .as_object_mut()
        .expect("answers object")
        .remove("stars");
    let error = failure(missing);
    assert_eq!(error.kind(), Some(ErrorKind::BadResponse));
    assert!(error.message().contains("no answer for `stars`"));

    let mut off_legend = document();
    off_legend["answers"]["stars"]["choice"] = json!("6");
    let error = failure(off_legend);
    assert_eq!(error.kind(), Some(ErrorKind::BadResponse));
    assert!(error.message().contains("`6`"));
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

    no_answers["answers"] = json!([{"id": "stars"}]);
    let error = failure(no_answers);
    assert_eq!(error.kind(), Some(ErrorKind::BadResponse));
}

#[test]
fn a_failed_reading_keeps_the_document_it_could_not_read() {
    let reading = read(json!({"answers": {"stars": {"choice": "4"}}}));
    assert!(reading.answers.is_err());
    assert_eq!(
        reading.document,
        json!({"answers": {"stars": {"choice": "4"}}}),
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
