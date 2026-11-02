//! The comparative request's own rules: what the budget is, who gets compared
//! when a group is larger than it, and what an answer may say.
//!
//! Everything here is a pure function of data the run already holds, so none of
//! it needs a server. The one request that leaves the process is asserted in
//! `systemone_http.rs`, where the body the adapter actually sends is recorded.

use imagecull_core::keeper::{self, Budget, Choice};
use imagecull_core::scoring::Response;
use serde_json::json;

/// An answer document as the server sends one, with the keeper it chose.
fn answered(position: &str, confidence: f64) -> Response {
    Response {
        document: json!({
            "model": "clef-flash-oQ4e",
            "answers": {
                "keeper": {
                    "type": "choice",
                    "choice": position,
                    "confidence": confidence,
                    "probabilities": {},
                }
            },
            "usage": {"input_tokens": 4154, "output_tokens": 0},
        }),
        client_latency_ms: 11_500,
    }
}

#[test]
fn the_budget_is_the_measured_configuration_and_not_a_guess() {
    let budget = Budget::default();

    assert_eq!(
        keeper::EDGE,
        512,
        "512 px is the default comparison edge: decisive, and a third of the cost of 1024"
    );
    assert_eq!(
        budget.max, 20,
        "20 previews at 512 px is 25% of the token cap"
    );
    assert_eq!(
        budget.floor, 0.60,
        "below 0.60 the pick is a coin flip, and is reported rather than suppressed"
    );
}

#[test]
fn the_instructions_are_ranked_gates_and_never_carry_the_source_indentation() {
    let instructions = keeper::INSTRUCTIONS;

    for gate in [
        "1. Sharp and still, above all else",
        "2. The moment",
        "3. Facing the camera",
        "4. Composition",
        "5. Light kept, last of the gates",
    ] {
        assert!(
            instructions.contains(gate),
            "the gates are ranked, and {gate} is one of them: {instructions}"
        );
    }
    let ranked: Vec<usize> = [
        "1. Sharp and still",
        "2. The moment",
        "3. Facing the camera",
        "4. Composition",
        "5. Light kept",
    ]
    .iter()
    .map(|gate| {
        instructions
            .find(gate)
            .unwrap_or_else(|| panic!("{gate} is missing from the ranking: {instructions}"))
    })
    .collect();
    assert!(
        ranked.windows(2).all(|pair| pair[0] < pair[1]),
        "the gates appear in rank order, sharpness first and light last: {instructions}"
    );
    assert!(
        instructions.contains("rule 1 decides by itself, and rules 2 to 5 only separate frames"),
        "crispness outranks every other gate, and says so: {instructions}"
    );
    assert!(
        instructions.contains("a sharp frame beats a softer frame with the better expression"),
        "the trade-off is named, not left to the choice head: {instructions}"
    );
    assert!(
        instructions.contains("If every frame fails this rule, keep the sharpest and stillest"),
        "a soft burst still names a frame: {instructions}"
    );
    assert!(
        instructions.contains("its eyes or face, or its principal detail, crisp"),
        "a face at 512 px cannot be asked about eye by eye: {instructions}"
    );
    assert!(
        instructions.contains("looking towards the camera with its face turned to it"),
        "the shoot's preference for direct address is stated, not implied: {instructions}"
    );
    assert!(
        instructions.contains("keep the more pleasant picture")
            && instructions.contains("the calmer background")
            && instructions.contains("the more natural pose"),
        "`pleasant` is operationalised rather than left to the choice head: {instructions}"
    );
    assert!(
        instructions.contains("Do not prefer a frame because it is first"),
        "position was measured deciding a pick the comparison should decide: {instructions}"
    );
    assert!(
        instructions.ends_with("Answer with the key of that frame."),
        "the answer is a key and nothing else: {instructions}"
    );
    for line in instructions.lines() {
        assert_eq!(
            line.trim_start(),
            line,
            "the continuation is joined without the source's indentation: {line:?}"
        );
    }
}

#[test]
fn a_group_inside_one_request_is_one_chunk_in_capture_order() {
    let places = [0, 1, 2];

    assert_eq!(
        keeper::chunk(&places, keeper::MAX),
        [vec![0, 1, 2]],
        "every frame of a group the budget holds is compared in one request, in the order it was \
         taken"
    );
}

#[test]
fn a_group_over_one_request_is_split_into_even_batches_in_capture_order() {
    let places: Vec<usize> = (0..45).collect();

    let chunks = keeper::chunk(&places, keeper::MAX);

    assert_eq!(
        chunks,
        [
            (0..15).collect::<Vec<_>>(),
            (15..30).collect::<Vec<_>>(),
            (30..45).collect::<Vec<_>>()
        ],
        "consecutive frames of the moment, at most one request's worth each, split evenly so the \
         remainder is spread rather than left as a request of its own"
    );
    assert_eq!(
        chunks.iter().flatten().count(),
        places.len(),
        "every frame of the group is compared by clef: no measure and no rating narrows who is \
         looked at"
    );
}

#[test]
fn a_remainder_that_would_be_one_frame_is_spread_across_the_batches() {
    let places: Vec<usize> = (0..21).collect();

    let chunks = keeper::chunk(&places, keeper::MAX);

    assert_eq!(
        chunks,
        [(0..11).collect::<Vec<_>>(), (11..21).collect::<Vec<_>>()],
        "twenty and one would ask clef which of one frame to keep, which is not a comparison at \
         all: the odd frame joins a batch instead"
    );
}

#[test]
fn chunks_never_exceed_the_request_and_never_leave_a_lone_frame() {
    for (frames, max) in [
        (1usize, 20usize),
        (2, 20),
        (20, 20),
        (21, 20),
        (40, 20),
        (41, 20),
        (0, 20),
    ] {
        let places: Vec<usize> = (0..frames).collect();
        let chunks = keeper::chunk(&places, max);
        assert!(
            chunks
                .iter()
                .all(|chunk| !chunk.is_empty() && chunk.len() <= max),
            "{frames} frames at {max} per request: {chunks:?}"
        );
        assert!(
            chunks.iter().all(|chunk| chunk.len() > 1) || frames < 2,
            "no request carries a single frame, which has nothing to compare: {frames} at \
             {max}: {chunks:?}"
        );
        assert_eq!(
            chunks.iter().flatten().copied().collect::<Vec<_>>(),
            places,
            "{frames} frames at {max} per request: every place survives, in order"
        );
    }
}

#[test]
fn the_lowest_confidence_on_a_winner_path_is_the_one_recorded() {
    assert_eq!(
        keeper::shakiest([0.83, 0.55, 0.91]),
        0.55,
        "a tournament's last call can be certain about two frames whose membership in the final \
         was itself a coin flip"
    );
    assert_eq!(
        keeper::shakiest([0.9]),
        0.9,
        "a group compared in one request records that request's confidence"
    );
}

#[test]
fn the_answer_is_read_as_the_frame_it_names_with_its_confidence() {
    let choice = keeper::read(&answered("2", 0.71), 3).expect("a readable answer");

    assert_eq!(
        choice,
        Choice {
            position: 2,
            confidence: 0.71
        }
    );
    assert_eq!(
        choice.index(),
        1,
        "the second of the frames the request showed"
    );
}

#[test]
fn an_answer_naming_a_frame_the_request_never_showed_is_refused() {
    let error = keeper::read(&answered("7", 0.9), 4).expect_err("a position out of range");

    assert_eq!(error.kind(), Some(imagecull_core::ErrorKind::BadResponse));
    assert!(
        error.message().contains('7') && error.message().contains('4'),
        "the refusal names the answer and the frames that were offered: {}",
        error.message()
    );
}

#[test]
fn a_document_that_answers_nothing_is_undecided_and_never_a_pick() {
    for document in [
        json!({"model": "clef-flash-oQ4e"}),
        json!({"answers": {}}),
        json!({"answers": {"keeper": {"type": "choice"}}}),
        json!({"answers": {"keeper": {"choice": "1"}}}),
        json!({"answers": {"keeper": {"choice": "first", "confidence": 0.7}}}),
    ] {
        let response = Response {
            document,
            client_latency_ms: 3,
        };
        let error = keeper::read(&response, 2).expect_err("no answer to read");
        assert_eq!(
            error.kind(),
            Some(imagecull_core::ErrorKind::BadResponse),
            "{}",
            error.message()
        );
    }
}

#[test]
fn the_comparison_carries_its_own_previews_and_the_state_the_group_is_judged_in() {
    let comparison = keeper::Comparison {
        model: "clef-flash-oQ4e".to_string(),
        state: imagecull_core::scoring::State::for_frame("DSC_7956.NEF", None),
        previews: Vec::new(),
    };

    assert_eq!(comparison.model, "clef-flash-oQ4e");
    assert_eq!(
        comparison.state.filename, "DSC_7956.NEF",
        "the group is named by the frame it opens with"
    );
}
