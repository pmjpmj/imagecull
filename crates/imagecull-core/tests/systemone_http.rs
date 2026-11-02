//! The adapter against a real socket: canned and adversarial `/v1/systemone`
//! answers, the exact request the client puts on the wire, the status taxonomy,
//! a timeout, and a three-frame run that proves requests go out one at a time.
//!
//! The stub is `tests/support/stub.rs`, a hand-rolled `TcpListener` shared with
//! `doctor_http.rs`: the real `ureq` client and the real JSON paths are
//! exercised, and every request is recorded, so the body assertions here are
//! about what the client actually sent. No network, no model in CI.

mod support;

use std::fs;
use std::net::TcpListener;
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use imagecull_core::error::{ErrorKind, Recovery, Result};
use imagecull_core::ingest;
use imagecull_core::keeper::{self, Comparison};
use imagecull_core::omlx::{Client, Endpoint, Timeouts};
use imagecull_core::preprocess::{Encoded, Params};
use imagecull_core::progress::Silent;
use imagecull_core::reading;
use imagecull_core::rubric::{Builtin, Rubric};
use imagecull_core::scoring::{self, Cancel, Config, Decider, Request, Response, State};
use imagecull_core::systemone::Adapter;
use serde_json::{Value, json};
use support::ExifFacts;
use support::stub::Stub;
use tempfile::TempDir;

/// A complete, well-formed answer: one choice for the one question.
fn answer_document() -> String {
    json!({
        "model": "clef-flash-oQ4e",
        "answers": {
            "stars": {"type": "choice", "choice": "4", "confidence": 0.63,
                      "probabilities": {"1": 0.01, "2": 0.03, "3": 0.12,
                                        "4": 0.63, "5": 0.21}}
        },
        "usage": {"input_tokens": 1327, "output_tokens": 0}
    })
    .to_string()
}

fn preview(width: u32, height: u32) -> Encoded {
    Encoded {
        jpeg: vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10, b'J', b'F', b'I', b'F'],
        width,
        height,
    }
}

fn request<'a>(
    rubric: &'a Rubric,
    preview: &'a Encoded,
    state: State,
    model: &'a str,
) -> Request<'a> {
    Request {
        model,
        rubric,
        state,
        preview,
    }
}

/// The decision requests the stub received, in order: a run's preflight GETs
/// come first and are not frames.
fn posts(stub: &Stub) -> Vec<support::stub::Received> {
    stub.requests()
        .into_iter()
        .filter(|request| request.path == "/v1/systemone")
        .collect()
}

/// The real adapter, pointed at the stub, with the timeouts a run would use.
fn adapter(stub: &Stub, model: &str) -> Adapter {
    Adapter::new(
        Client::new(stub.endpoint(), Timeouts::with_read_seconds(5)),
        model,
    )
}

/// One request for one frame, as the pipeline would build it.
fn decide(stub: &Stub, state: State, preview: &Encoded, model: &str) -> Result<Response> {
    let rubric = Builtin::General.rubric().expect("parse");
    adapter(stub, model).decide(&request(&rubric, preview, state, model))
}

#[test]
fn a_canned_answer_comes_back_as_a_reading_with_the_one_answer() {
    let stub = Stub::scripted(vec![(200, answer_document())]);
    let preview = preview(1568, 1045);

    let response = decide(
        &stub,
        State::for_frame("DSC_0001.NEF", Some("street at dusk".into())),
        &preview,
        "clef-flash-oQ4e",
    )
    .expect("a 200 is an answer");

    let reading = reading::read(&Builtin::General.rubric().expect("parse"), response);
    let answers = reading.answers.expect("a readable answer");
    assert_eq!(answers.stars, 4);
    assert_eq!(answers.confidence, 0.63);
    assert_eq!(reading.usage.input_tokens, Some(1327));

    let requests = stub.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].path, "/v1/systemone");
    assert!(
        requests[0]
            .content_type
            .as_deref()
            .is_some_and(|value| value.starts_with("application/json")),
        "the body is JSON, announced: {:?}",
        requests[0].content_type
    );
}

#[test]
fn the_request_carries_one_image_one_state_and_the_questions_in_order() {
    let stub = Stub::scripted(vec![(200, answer_document())]);
    let preview = preview(1568, 1045);

    decide(
        &stub,
        State::for_frame("DSC_0421.NEF", Some("street at dusk, high ISO".into())),
        &preview,
        "clef-flash-oQ4e",
    )
    .expect("a 200 is an answer");

    let body = stub.requests()[0].body.clone();
    assert_eq!(body["model"], "clef-flash-oQ4e");
    assert_eq!(body["truncate"], true);
    assert_eq!(
        body["state"],
        json!({
            "task": "photo_cull",
            "filename": "DSC_0421.NEF",
            "shoot_context": "street at dusk, high ISO",
        })
    );

    let images = body["images"].as_array().expect("one images array");
    assert_eq!(images.len(), 1, "one frame per request, never several");
    let uri = images[0].as_str().expect("a data URI");
    let encoded = uri
        .strip_prefix("data:image/jpeg;base64,")
        .expect("a full JPEG data URI");
    assert_eq!(
        BASE64.decode(encoded).expect("valid base64"),
        preview.jpeg,
        "the image is the exact bytes the pipeline encoded"
    );

    let questions = body["questions"].as_object().expect("named questions");
    assert_eq!(
        questions.keys().map(String::as_str).collect::<Vec<_>>(),
        ["stars"],
        "declaration order is part of the legend the model reads"
    );
}

#[test]
fn the_shoot_context_is_omitted_when_the_photographer_gave_none() {
    let stub = Stub::scripted(vec![(200, answer_document()), (200, answer_document())]);
    let preview = preview(640, 480);

    decide(
        &stub,
        State::for_frame("DSC_0001.NEF", None),
        &preview,
        "clef-flash-oQ4e",
    )
    .expect("a 200 is an answer");
    decide(
        &stub,
        State::for_frame("DSC_0002.NEF", Some("high ISO".into())),
        &preview,
        "clef-flash-oQ4e",
    )
    .expect("a 200 is an answer");

    let requests = stub.requests();
    let without = requests[0].body["state"]
        .as_object()
        .expect("a state object");
    assert_eq!(
        without.get("shoot_context"),
        None,
        "a missing --shoot-context is absent, not null"
    );
    assert_eq!(without["task"], "photo_cull");
    assert_eq!(without["filename"], "DSC_0001.NEF");
    assert_eq!(requests[1].body["state"]["shoot_context"], "high ISO");
}

#[test]
fn the_question_sends_its_options_keyed_by_key_in_order() {
    let stub = Stub::scripted(vec![(200, answer_document())]);
    let preview = preview(100, 100);

    decide(
        &stub,
        State::for_frame("DSC_0001.NEF", None),
        &preview,
        "clef-flash-oQ4e",
    )
    .expect("a 200 is an answer");

    let questions = stub.requests()[0].body["questions"].clone();
    let rubric = Builtin::General.rubric().expect("parse");

    let stars = &questions["stars"];
    assert_eq!(stars["type"], "choice");
    assert_eq!(
        stars["instructions"],
        rubric.question("stars").expect("stars").instructions()
    );
    let options = stars["criteria"].as_object().expect("options keyed by key");
    assert_eq!(
        options.keys().map(String::as_str).collect::<Vec<_>>(),
        ["1", "2", "3", "4", "5"],
        "option order survives, because it is part of the legend"
    );
    let described: Vec<&str> = rubric
        .question("stars")
        .expect("stars")
        .criteria()
        .iter()
        .map(|criterion| criterion.description())
        .collect();
    assert_eq!(
        options
            .values()
            .map(|option| option.as_str().expect("a description"))
            .collect::<Vec<_>>(),
        described,
        "each option carries the wording the rubric gives it, in key order"
    );
}

#[test]
fn an_adversarial_document_becomes_a_bad_response_not_a_guess() {
    let cases: [(&str, Value); 4] = [
        (
            "an answer key missing",
            json!({"answers": {"technical": {"score": 6.0, "confidence": 0.4}}}),
        ),
        (
            "only probabilities",
            json!({"answers": {
                "stars": {"probabilities": {"4": 0.9}, "confidence": 0.4},
            }}),
        ),
        (
            "a star key the rubric does not offer",
            json!({"answers": {
                "stars": {"choice": "9", "confidence": 0.5},
            }}),
        ),
        (
            "no answers object at all",
            json!({"model": "clef-flash-oQ4e"}),
        ),
    ];

    for (case, document) in cases {
        let stub = Stub::scripted(vec![(200, document.to_string())]);
        let preview = preview(100, 100);
        let response = decide(
            &stub,
            State::for_frame("DSC_0001.NEF", None),
            &preview,
            "clef-flash-oQ4e",
        )
        .unwrap_or_else(|error| panic!("{case}: HTTP itself succeeded: {error}"));
        let reading = reading::read(&Builtin::General.rubric().expect("parse"), response);
        let error = reading.answers.expect_err(&format!("{case} must not read"));
        assert_eq!(error.kind(), Some(ErrorKind::BadResponse), "{case}");
    }
}

#[test]
fn a_body_that_is_not_json_is_kept_as_the_document_it_was() {
    let stub = Stub::scripted(vec![(200, "not json at all".to_string())]);
    let preview = preview(100, 100);

    let response = decide(
        &stub,
        State::for_frame("DSC_0001.NEF", None),
        &preview,
        "clef-flash-oQ4e",
    )
    .expect("the status was a 200");

    assert_eq!(response.document, json!("not json at all"));
    let reading = reading::read(&Builtin::General.rubric().expect("parse"), response);
    let error = reading.answers.expect_err("nothing to read");
    assert_eq!(error.kind(), Some(ErrorKind::BadResponse));
}

#[test]
fn the_status_taxonomy_names_what_the_server_rejected() {
    let cases: [(u16, &str, ErrorKind); 3] = [
        (
            400,
            r#"{"detail":"invalid criteria for question 'stars'"}"#,
            ErrorKind::Http400,
        ),
        (413, r#"{"detail":"context too long"}"#, ErrorKind::Http413),
        (
            422,
            r#"{"detail":"state must not be null"}"#,
            ErrorKind::Http422,
        ),
    ];

    for (status, body, kind) in cases {
        let stub = Stub::scripted(vec![(status, body.to_string())]);
        let preview = preview(100, 100);
        let error = decide(
            &stub,
            State::for_frame("DSC_0001.NEF", None),
            &preview,
            "clef-flash-oQ4e",
        )
        .unwrap_err();

        assert_eq!(error.kind(), Some(kind), "HTTP {status}");
        assert_eq!(error.http_status(), Some(status));
        assert_eq!(error.recovery(), kind.recovery());
        let message = error.message();
        assert!(message.contains(&status.to_string()), "{message}");
        let detail = body
            .trim_start_matches(r#"{"detail":""#)
            .trim_end_matches(r#""}"#);
        assert!(
            message.contains(detail),
            "the server's own detail must ride along: {message}"
        );
    }
}

#[test]
fn a_5xx_names_the_server_side_failure_and_keeps_its_status() {
    let stub = Stub::scripted(vec![(500, r#"{"detail":"boom"}"#.to_string())]);
    let preview = preview(100, 100);

    let error = decide(
        &stub,
        State::for_frame("DSC_0001.NEF", None),
        &preview,
        "clef-flash-oQ4e",
    )
    .expect_err("a 500 is not an answer");

    assert_eq!(error.kind(), Some(ErrorKind::Http5xx));
    assert_eq!(error.http_status(), Some(500));
    assert_eq!(
        error.recovery(),
        Recovery::RetryThenRecord,
        "a server-side hiccup is retried, not an abort and not a 4xx"
    );
    assert!(error.message().contains("boom"));
}

#[test]
fn a_context_that_is_too_long_says_which_knob_to_turn_down() {
    let stub = Stub::scripted(vec![(413, r#"{"detail":"context too long"}"#.to_string())]);
    let preview = preview(100, 100);

    let error = decide(
        &stub,
        State::for_frame("DSC_0001.NEF", None),
        &preview,
        "clef-flash-oQ4e",
    )
    .expect_err("a 413 is not an answer");

    assert_eq!(error.kind(), Some(ErrorKind::Http413));
    assert!(
        error.message().contains("--max-edge"),
        "the row has to say what to change: {}",
        error.message()
    );
}

#[test]
fn nothing_listening_is_server_unreachable() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("addr");
    drop(listener);

    let adapter = Adapter::new(
        Client::new(
            Endpoint::parse(&format!("http://{addr}")).expect("valid endpoint"),
            Timeouts::with_read_seconds(5),
        ),
        "clef-flash-oQ4e",
    );
    let rubric = Builtin::General.rubric().expect("parse");
    let preview = preview(100, 100);
    let error = adapter
        .decide(&request(
            &rubric,
            &preview,
            State::for_frame("DSC_0001.NEF", None),
            "clef-flash-oQ4e",
        ))
        .expect_err("nobody is listening");

    assert_eq!(error.kind(), Some(ErrorKind::ServerUnreachable));
}

#[test]
fn a_server_that_never_answers_times_out() {
    let stub = Stub::scripted(vec![(200, answer_document())]).taking(Duration::from_secs(2));
    let adapter = Adapter::new(
        Client::new(
            stub.endpoint(),
            Timeouts {
                connect: Duration::from_secs(5),
                read: Duration::from_millis(200),
            },
        ),
        "clef-flash-oQ4e",
    );
    let rubric = Builtin::General.rubric().expect("parse");
    let preview = preview(100, 100);

    let error = adapter
        .decide(&request(
            &rubric,
            &preview,
            State::for_frame("DSC_0001.NEF", None),
            "clef-flash-oQ4e",
        ))
        .expect_err("the read timeout expires");

    assert_eq!(error.kind(), Some(ErrorKind::Timeout));
}

/// A shoot folder with `frames` synthetic NEFs, each with its own preview.
fn shoot(frames: usize) -> TempDir {
    let dir = tempfile::tempdir().expect("a temp dir");
    for index in 1..=frames {
        let bytes = support::nef(1, &[support::jpeg(48 + index as u32, 32, 80)]);
        fs::write(dir.path().join(format!("DSC_{index:04}.NEF")), bytes).expect("write");
    }
    dir
}

#[test]
fn a_run_sends_one_request_at_a_time_in_ingest_order() {
    let shoot = shoot(3);
    let script = std::iter::repeat_n((200, answer_document()), 3).collect();
    // A handler that takes a moment to answer would let a concurrent client
    // overlap requests; the stub records the deepest overlap it ever saw.
    let stub = Stub::serving(script, &["clef-flash-oQ4e"]).taking(Duration::from_millis(30));

    let config = Config::new(
        Builtin::General.rubric().expect("parse"),
        None,
        Params::default(),
        ingest::ScanOptions::default(),
        Some("street at dusk".into()),
        support::unwatched(),
    );
    let run = scoring::run(
        shoot.path(),
        &config,
        &adapter(&stub, &config.model),
        &imagecull_core::sidecar::Filesystem,
        &Silent,
        &Cancel::new(),
    )
    .expect("a run");

    assert_eq!(run.frames.len(), 3);
    assert_eq!(run.scored(), 3);
    assert_eq!(run.failed(), 0);
    assert_eq!(run.model, "clef-flash-oQ4e");
    assert_eq!(run.rubric_hash, config.rubric_hash());

    let requests = posts(&stub);
    assert_eq!(requests.len(), 3, "one request per frame");
    let names: Vec<&str> = requests
        .iter()
        .map(|request| {
            request.body["state"]["filename"]
                .as_str()
                .expect("filename")
        })
        .collect();
    assert_eq!(names, ["DSC_0001.NEF", "DSC_0002.NEF", "DSC_0003.NEF"]);
    for request in &requests {
        assert_eq!(
            request.body["images"].as_array().expect("images").len(),
            1,
            "exactly one image per request"
        );
        assert_eq!(request.body["state"]["shoot_context"], "street at dusk");
        assert_eq!(request.body["truncate"], true);
    }
    assert_eq!(
        stub.max_in_flight(),
        1,
        "requests are strictly sequential; the server never saw two at once"
    );
}

#[test]
fn a_run_over_http_reports_the_answers_it_read() {
    let shoot = shoot(1);
    let stub = Stub::serving(vec![(200, answer_document())], &["clef-27b-oQ4e"]);
    let config = Config::new(
        Builtin::General.rubric().expect("parse"),
        Some("clef-27b-oQ4e".into()),
        Params::default(),
        ingest::ScanOptions::default(),
        None,
        support::unwatched(),
    );

    let run = scoring::run(
        shoot.path(),
        &config,
        &adapter(&stub, &config.model),
        &imagecull_core::sidecar::Filesystem,
        &Silent,
        &Cancel::new(),
    )
    .expect("a run");

    let frame = &run.frames[0];
    let reading = frame.reading().expect("the frame was prepared");
    let answers = reading.answers.as_ref().expect("a readable answer");
    assert_eq!(answers.stars, 4);
    assert_eq!(posts(&stub)[0].body["model"], "clef-27b-oQ4e");
    assert_eq!(
        run.rubric_hash,
        config.rubric.hash("clef-27b-oQ4e"),
        "--model feeds the rubric hash"
    );
}

#[test]
fn a_server_that_keys_its_type_report_still_passes_preflight() {
    // The real-world shape: /health and /v1/models open, /v1/models/status behind
    // an API key, /v1/systemone answering. Refusing to score here would be the
    // tool calling a working server broken.
    let stub = Stub::routed(vec![
        support::stub::Route::new(
            "/health",
            200,
            json!({"status": "healthy", "default_model": "clef-flash-oQ4e"}).to_string(),
        ),
        support::stub::Route::new(
            "/v1/models",
            200,
            json!({"data": [{"id": "clef-flash-oQ4e"}]}).to_string(),
        ),
        support::stub::Route::new(
            "/v1/models/status",
            401,
            json!({"error": {"message": "API key required"}}).to_string(),
        ),
    ]);
    let decide = &adapter(&stub, "clef-flash-oQ4e");

    assert!(
        decide.preflight().is_ok(),
        "a keyed type report is a caveat, not a failure: {:?}",
        decide.preflight().map_err(|error| error.to_string())
    );
}

#[test]
fn a_server_that_does_not_report_model_types_can_still_be_scored() {
    // Spec §51: preflight confirms the alias exists "and if the server exposes
    // per-model status, confirm the model is a decision model". A server without
    // that endpoint has not said it is unfit — refusing every run against it
    // would refuse a working server on the strength of a missing nicety.
    let shoot = shoot(1);
    let stub = Stub::untyped(vec![(200, answer_document())], &["clef-flash-oQ4e"]);
    let config = Config::new(
        Builtin::General.rubric().expect("parse"),
        None,
        Params::default(),
        ingest::ScanOptions::default(),
        None,
        support::unwatched(),
    );
    let adapter = adapter(&stub, &config.model);
    let preflight = adapter.preflight();
    assert!(
        preflight.is_ok(),
        "a server that cannot confirm types may still score: {:?}",
        preflight.map_err(|error| error.to_string())
    );
    let run = scoring::run(
        shoot.path(),
        &config,
        &adapter,
        &imagecull_core::sidecar::Filesystem,
        &Silent,
        &Cancel::new(),
    )
    .expect("a run over a server with no type report");

    assert_eq!(run.scored(), 1, "the frame was scored, not refused");
    assert_eq!(posts(&stub).len(), 1, "one frame, one request");
}

#[test]
fn a_server_that_says_the_alias_is_not_a_decision_model_fails_preflight() {
    let stub = Stub::routed(vec![
        support::stub::Route::new(
            "/health",
            200,
            json!({"status": "ok", "version": "0.7.1.dev1"}).to_string(),
        ),
        support::stub::Route::new(
            "/v1/models",
            200,
            json!({"data": [{"id": "clef-flash-oQ4e"}]}).to_string(),
        ),
        support::stub::Route::new(
            "/v1/models/status",
            200,
            json!({"models": [{"id": "clef-flash-oQ4e", "model_type": "vlm"}]}).to_string(),
        ),
    ]);

    let error = adapter(&stub, "clef-flash-oQ4e")
        .preflight()
        .expect_err("a chat model cannot answer /v1/systemone");

    assert!(error.is_fatal());
    assert!(
        error.message().contains("not a decision model") && error.message().contains("vlm"),
        "the message says what the alias is and what it needs to be: {}",
        error.message()
    );
    assert!(
        posts(&stub).is_empty(),
        "nothing is sent to a server that was told it cannot answer"
    );
}

#[test]
fn a_server_that_predates_decision_models_fails_preflight() {
    // The relaxed rule still refuses the one server that has said, in so many
    // words, that it cannot serve the endpoint: 0.7.0 predates decision models.
    let stub = Stub::routed(vec![
        support::stub::Route::new(
            "/health",
            200,
            json!({"status": "ok", "version": "0.7.0"}).to_string(),
        ),
        support::stub::Route::new(
            "/v1/models",
            200,
            json!({"data": [{"id": "clef-flash-oQ4e"}]}).to_string(),
        ),
        // No /v1/models/status at all: the stub 404s, as 0.7.0 does.
    ]);

    let error = adapter(&stub, "clef-flash-oQ4e")
        .preflight()
        .expect_err("a server older than decision models cannot score");

    assert!(error.is_fatal());
    assert!(
        error.message().contains("predates decision models")
            && error.message().contains("0.7.1.dev1"),
        "the message names the release that is needed: {}",
        error.message()
    );
}

#[test]
fn a_run_against_a_server_that_does_not_serve_the_alias_never_asks_a_question() {
    let shoot = shoot(2);
    let stub = Stub::serving(Vec::new(), &["clef-27b-oQ4e"]);
    let config = Config::new(
        Builtin::General.rubric().expect("parse"),
        Some("clef-flash-oQ4e".into()),
        Params::default(),
        ingest::ScanOptions::default(),
        None,
        support::unwatched(),
    );

    let error = scoring::run(
        shoot.path(),
        &config,
        &adapter(&stub, "clef-flash-oQ4e"),
        &imagecull_core::sidecar::Filesystem,
        &Silent,
        &Cancel::new(),
    )
    .expect_err("a missing alias is a broken server, not a broken file");

    assert!(error.is_fatal());
    assert!(
        error.message().contains("clef-flash-oQ4e") && error.message().contains("clef-27b-oQ4e"),
        "the message names the alias asked for and the ones served: {}",
        error.message()
    );
    assert!(
        posts(&stub).is_empty(),
        "no frame is sent to a server that cannot answer"
    );
}

/// The answer a comparative request gets: one choice, by position.
fn keeper_document(position: &str, confidence: f64) -> String {
    json!({
        "model": "clef-flash-oQ4e",
        "answers": {"keeper": {"type": "choice", "choice": position, "confidence": confidence}},
        "usage": {"input_tokens": 557, "output_tokens": 0}
    })
    .to_string()
}

/// A comparative request over `count` previews of one moment.
fn comparison(count: usize) -> Comparison {
    Comparison {
        model: "clef-flash-oQ4e".to_string(),
        state: State::for_frame("DSC_7956.NEF", Some("street at dusk".into())),
        previews: (0..count).map(|_| preview(512, 341)).collect(),
    }
}

#[test]
fn the_comparative_request_is_one_body_of_several_images_and_a_choice_between_them() {
    let stub = Stub::scripted(vec![(200, keeper_document("2", 0.71))]);

    let response = adapter(&stub, "clef-flash-oQ4e")
        .compare(&comparison(3))
        .expect("a 200 is an answer");
    let choice = keeper::read(&response, 3).expect("a readable answer");

    assert_eq!(choice.position, 2);
    assert_eq!(choice.confidence, 0.71);

    let requested = posts(&stub);
    assert_eq!(requested.len(), 1, "one request, whatever the group holds");
    let body = &requested[0].body;
    assert_eq!(body["model"], json!("clef-flash-oQ4e"));
    assert_eq!(body["truncate"], json!(true), "the state is truncatable");
    assert_eq!(body["state"]["task"], json!("photo_cull"));
    assert_eq!(
        body["state"]["filename"],
        json!("DSC_7956.NEF"),
        "the group is named by the frame it opens with"
    );
    assert_eq!(body["state"]["shoot_context"], json!("street at dusk"));

    let images = body["images"].as_array().expect("an images array");
    assert_eq!(images.len(), 3, "one image per compared frame");
    assert!(
        images.iter().all(|image| image
            .as_str()
            .is_some_and(|uri| uri.starts_with("data:image/jpeg;base64,"))),
        "every image is a data URI, which is the only form the server takes: {images:?}"
    );

    let questions = body["questions"]
        .as_object()
        .expect("questions is a map keyed by question id");
    let keeper = &questions["keeper"];
    assert_eq!(keeper["type"], json!("choice"));
    assert!(
        keeper["instructions"]
            .as_str()
            .is_some_and(|text| text.contains("Answer with the key of that frame")),
        "the question says what the answer is: {keeper}"
    );
    let criteria = keeper["criteria"]
        .as_object()
        .expect("criteria is a map of option keys to descriptions");
    assert_eq!(criteria.len(), 3);
    assert_eq!(criteria["1"], json!("image at position 1"));
    assert_eq!(criteria["3"], json!("image at position 3"));
}

#[test]
fn a_group_the_model_cannot_hold_is_a_413_a_run_can_report_and_never_a_smaller_request() {
    let stub = Stub::scripted(vec![(
        413,
        json!({"message": "the schema needs 19810 tokens before the state; the limit is 16384"})
            .to_string(),
    )]);

    let error = adapter(&stub, "clef-flash-oQ4e")
        .compare(&comparison(21))
        .expect_err("a 413 is a failure, not an answer");

    assert_eq!(error.kind(), Some(ErrorKind::Http413));
    assert!(
        !error.recovery().aborts_run(),
        "one undecided group must not cost the run its other frames"
    );
    assert!(
        error.message().contains("nothing was marked"),
        "the remedy is a reported group, not a quietly smaller request: {}",
        error.message()
    );
    assert_eq!(
        posts(&stub).len(),
        1,
        "the request is refused once, and never retried smaller"
    );
}

/// A shoot folder holding two frames of one moment: one body, one exposure, a
/// tenth of a second apart, and two different pictures of one scene.
fn moment() -> TempDir {
    let dir = tempfile::tempdir().expect("a temp dir");
    let scene = support::scenes::scene(320, 214);
    for (name, stops, subsecond) in [("DSC_0001.NEF", 0.0, "10"), ("DSC_0002.NEF", 0.2, "20")] {
        let image = if stops == 0.0 {
            scene.clone()
        } else {
            support::scenes::brighter(&scene, stops)
        };
        let facts = ExifFacts {
            captured: Some("2026:11:02 10:00:00".to_string()),
            subsecond: Some(subsecond.to_string()),
            serial: Some("3001".to_string()),
            iso: Some(200),
            exposure_time: Some((1, 500)),
            f_number: Some((28, 10)),
            focal_length: Some((50, 1)),
        };
        fs::write(
            dir.path().join(name),
            support::nef_with(1, &[support::jpeg_of(&image, 92)], &facts),
        )
        .expect("write a NEF");
    }
    dir
}

#[test]
fn a_group_that_413s_is_undecided_and_marks_nothing() {
    let shoot = moment();
    let stub = Stub::serving(
        vec![(
            413,
            json!({"message": "the schema needs 19810 tokens before the state; the limit is 16384"})
                .to_string(),
        )],
        &["clef-flash-oQ4e"],
    );
    let plan = imagecull_core::group::run(
        shoot.path(),
        &imagecull_core::group::Options {
            scan: ingest::ScanOptions::default(),
            thresholds: imagecull_core::group::Thresholds::default(),
            jobs: Some(1),
        },
    )
    .expect("a plan");
    let config = imagecull_core::cull::Config {
        model: "clef-flash-oQ4e",
        shoot_context: None,
        budget: keeper::Budget::default(),
        preview: Params::default(),
        grouping_hash: &plan.hash,
        allow_foreign: false,
        retry: scoring::RetryPolicy::without_waiting(),
    };

    let culled = imagecull_core::cull::run(
        &plan,
        &config,
        &adapter(&stub, config.model),
        &imagecull_core::sidecar::Filesystem,
        &imagecull_core::cull::Silent,
    )
    .expect("a rejected group is not a rejected run");

    assert_eq!(posts(&stub).len(), 1, "one group, one comparison");
    assert_eq!(culled.undecided, 1);
    assert_eq!(
        culled.non_keepers, 0,
        "a group with no answer marks nothing"
    );
    let failure = culled.verdicts[0]
        .failure
        .as_ref()
        .expect("the group says why it has no keeper");
    assert_eq!(
        failure.kind,
        ErrorKind::Http413,
        "the group names the status the frame path's rows carry, so a photographer and a script \
         read the same word"
    );
    assert!(
        failure.message.contains("413") && failure.message.contains("nothing was marked"),
        "the message carries the status and says nothing was filtered: {}",
        failure.message
    );
    for name in ["DSC_0001.NEF", "DSC_0002.NEF"] {
        assert!(
            !imagecull_core::sidecar::path_for(&shoot.path().join(name)).exists(),
            "{name} was never rated and a refused group writes no verdict"
        );
    }
}
