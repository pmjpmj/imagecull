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
use imagecull_core::error::{ErrorKind, Result};
use imagecull_core::ingest;
use imagecull_core::omlx::{Client, Endpoint, Timeouts};
use imagecull_core::preprocess::{Encoded, Params};
use imagecull_core::reading::{self, Verdict};
use imagecull_core::rubric::Rubric;
use imagecull_core::scoring::{self, Config, Decider, Request, Response, State};
use imagecull_core::systemone::Adapter;
use serde_json::{Value, json};
use support::stub::Stub;
use tempfile::TempDir;

/// A complete, well-formed answer: six answers for the six questions.
fn answer_document() -> String {
    json!({
        "model": "clef-flash-oQ4e",
        "answers": {
            "overall": {"type": "score", "score": 6.4237, "confidence": 0.41,
                        "legend": {"0": "delete"}, "probabilities": {"6": 0.41}},
            "technical": {"type": "score", "score": 7.8125, "confidence": 0.55,
                          "legend": {"0": "delete"}, "probabilities": [0.45, 0.55]},
            "composition": {"type": "score", "score": 5.3398, "confidence": 0.33,
                            "legend": {"0": "delete"}, "probabilities": {"5": 0.33}},
            "impact": {"type": "score", "score": 4.9601, "confidence": 0.62,
                       "legend": {"0": "delete"}, "probabilities": {"4": 0.62}},
            "stars": {"type": "choice", "choice": "4", "confidence": 0.63,
                      "probabilities": {"4": 0.63}},
            "verdict": {"type": "choice", "choice": "like", "confidence": 0.88,
                        "probabilities": {"like": 0.88}}
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

/// The real adapter, pointed at the stub, with the timeouts a run would use.
fn adapter(stub: &Stub) -> Adapter {
    Adapter::new(Client::new(stub.endpoint(), Timeouts::with_read_seconds(5)))
}

/// One request for one frame, as the pipeline would build it.
fn decide(stub: &Stub, state: State, preview: &Encoded, model: &str) -> Result<Response> {
    let rubric = Rubric::embedded().expect("parse");
    adapter(stub).decide(&request(&rubric, preview, state, model))
}

#[test]
fn a_canned_answer_comes_back_as_a_reading_with_all_six_answers() {
    let stub = Stub::scripted(vec![(200, answer_document())]);
    let preview = preview(1568, 1045);

    let response = decide(
        &stub,
        State::for_frame("DSC_0001.NEF", Some("street at dusk".into())),
        &preview,
        "clef-flash-oQ4e",
    )
    .expect("a 200 is an answer");

    let reading = reading::read(&Rubric::embedded().expect("parse"), response);
    let answers = reading.answers.expect("six readable answers");
    assert_eq!(answers.overall, 6.4237);
    assert_eq!(answers.stars, 4);
    assert_eq!(answers.verdict, Verdict::Like);
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
        [
            "overall",
            "technical",
            "composition",
            "impact",
            "stars",
            "verdict"
        ],
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
fn a_score_question_sends_its_legend_and_a_choice_question_its_options() {
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
    let rubric = Rubric::embedded().expect("parse");

    let overall = &questions["overall"];
    assert_eq!(overall["type"], "score");
    assert_eq!(
        overall["instructions"],
        rubric.question("overall").expect("overall").instructions()
    );
    let legend = overall["criteria"].as_array().expect("an ordered legend");
    assert_eq!(legend.len(), 11, "keys 0-10, one description each");
    assert_eq!(legend[0], "delete — unusable, the frame does not work");
    assert_eq!(legend[10], "portfolio-grade — the best the shoot can offer");

    let stars = &questions["stars"];
    assert_eq!(stars["type"], "choice");
    let options = stars["criteria"].as_object().expect("options keyed by key");
    assert_eq!(
        options.keys().map(String::as_str).collect::<Vec<_>>(),
        ["1", "2", "3", "4", "5"],
        "option order survives, because it is part of the legend"
    );
    assert_eq!(
        options["1"],
        "reject — unusable, out of focus, or a duplicate"
    );

    let verdict = &questions["verdict"];
    assert_eq!(
        verdict["criteria"]
            .as_object()
            .expect("options keyed by key")
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        ["dislike", "neutral", "like"]
    );
}

#[test]
fn an_adversarial_document_becomes_a_bad_response_not_a_guess() {
    let cases: [(&str, Value); 4] = [
        (
            "an answer key missing",
            json!({"answers": {"overall": {"score": 6.0, "confidence": 0.4}}}),
        ),
        (
            "only probabilities",
            json!({"answers": {
                "overall": {"probabilities": {"6": 0.9}, "confidence": 0.4},
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
        let reading = reading::read(&Rubric::embedded().expect("parse"), response);
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
    let reading = reading::read(&Rubric::embedded().expect("parse"), response);
    let error = reading.answers.expect_err("nothing to read");
    assert_eq!(error.kind(), Some(ErrorKind::BadResponse));
}

#[test]
fn the_status_taxonomy_names_what_the_server_rejected() {
    let cases: [(u16, &str, ErrorKind); 3] = [
        (
            400,
            r#"{"detail":"invalid criteria for question 'overall'"}"#,
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
fn a_5xx_is_classified_as_a_failure_with_its_status_kept() {
    // Retrying and the 5xx action are ticket 06's; here the point is that a
    // 500 is never mistaken for an answer.
    let stub = Stub::scripted(vec![(500, r#"{"detail":"boom"}"#.to_string())]);
    let preview = preview(100, 100);

    let error = decide(
        &stub,
        State::for_frame("DSC_0001.NEF", None),
        &preview,
        "clef-flash-oQ4e",
    )
    .expect_err("a 500 is not an answer");

    assert_eq!(error.http_status(), Some(500));
    assert!(error.message().contains("boom"));
}

#[test]
fn nothing_listening_is_server_unreachable() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("addr");
    drop(listener);

    let adapter = Adapter::new(Client::new(
        Endpoint::parse(&format!("http://{addr}")).expect("valid endpoint"),
        Timeouts::with_read_seconds(5),
    ));
    let rubric = Rubric::embedded().expect("parse");
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
    let adapter = Adapter::new(Client::new(
        stub.endpoint(),
        Timeouts {
            connect: Duration::from_secs(5),
            read: Duration::from_millis(200),
        },
    ));
    let rubric = Rubric::embedded().expect("parse");
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
    let stub = Stub::scripted(script).taking(Duration::from_millis(30));

    let config = Config::new(
        Rubric::embedded().expect("parse"),
        None,
        Params::default(),
        ingest::ScanOptions::default(),
        Some("street at dusk".into()),
    );
    let run = scoring::run(shoot.path(), &config, &adapter(&stub)).expect("a run");

    assert_eq!(run.frames.len(), 3);
    assert_eq!(run.scored(), 3);
    assert_eq!(run.failed(), 0);
    assert_eq!(run.model, "clef-flash-oQ4e");
    assert_eq!(run.rubric_hash, config.rubric_hash());

    let requests = stub.requests();
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
    let stub = Stub::scripted(vec![(200, answer_document())]);
    let config = Config::new(
        Rubric::embedded().expect("parse"),
        Some("clef-27b-oQ4e".into()),
        Params::default(),
        ingest::ScanOptions::default(),
        None,
    );

    let run = scoring::run(shoot.path(), &config, &adapter(&stub)).expect("a run");

    let frame = &run.frames[0];
    let reading = frame.outcome.as_ref().expect("the frame was prepared");
    let answers = reading.answers.as_ref().expect("six answers");
    assert_eq!(answers.overall, 6.4237);
    assert_eq!(answers.stars, 4);
    assert_eq!(stub.requests()[0].body["model"], "clef-27b-oQ4e");
    assert_eq!(
        run.rubric_hash,
        config.rubric.hash("clef-27b-oQ4e"),
        "--model feeds the rubric hash"
    );
}
