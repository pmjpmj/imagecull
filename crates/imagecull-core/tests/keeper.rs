//! The comparative request's own rules: what the budget is, who gets compared
//! when a group is larger than it, and what an answer may say.
//!
//! Everything here is a pure function of data the run already holds, so none of
//! it needs a server. The one request that leaves the process is asserted in
//! `systemone_http.rs`, where the body the adapter actually sends is recorded.

use imagecull_core::keeper::{self, Budget, Candidate, Choice};
use imagecull_core::scoring::Response;
use serde_json::json;

fn candidate(rating: u8, acutance: Option<f64>) -> Candidate {
    Candidate { rating, acutance }
}

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
        budget.edge, 512,
        "512 px is decisive and a third of the cost"
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
fn a_group_inside_the_budget_is_compared_whole_and_in_capture_order() {
    let group = [
        candidate(4, Some(0.5)),
        candidate(5, Some(0.4)),
        candidate(3, Some(0.9)),
    ];

    assert_eq!(
        keeper::compared(&group, keeper::MAX),
        [0, 1, 2],
        "every frame of a group the budget holds is compared, in the order it was taken"
    );
}

#[test]
fn a_group_over_the_budget_keeps_the_best_rated_frames_and_leaves_the_rest_out() {
    let group = [
        candidate(2, Some(0.9)),
        candidate(5, Some(0.1)),
        candidate(3, Some(0.5)),
        candidate(2, Some(0.2)),
    ];

    assert_eq!(
        keeper::compared(&group, 2),
        [1, 2],
        "the two best-rated frames are compared; the rating decides, and only the rating"
    );
}

#[test]
fn an_equal_rating_is_broken_by_acutance_and_an_unmeasured_frame_loses_to_a_number() {
    let group = [
        candidate(4, Some(0.30)),
        candidate(4, None),
        candidate(4, Some(0.80)),
        candidate(4, Some(0.55)),
    ];

    assert_eq!(
        keeper::compared(&group, 2),
        [2, 3],
        "the two sharpest of four equal ratings: the frame nothing could be measured from sorts \
         below every frame that has a number, however low"
    );
}

#[test]
fn a_shortlist_is_a_function_of_the_group_and_not_of_the_order_it_arrived_in() {
    let first = [
        candidate(4, Some(0.30)),
        candidate(4, Some(0.30)),
        candidate(4, Some(0.10)),
    ];
    let second = [
        candidate(4, Some(0.10)),
        candidate(4, Some(0.30)),
        candidate(4, Some(0.30)),
    ];

    let left = keeper::compared(&first, 2);
    let right = keeper::compared(&second, 2);

    assert_eq!(left, [0, 1], "equal frames keep their capture order");
    assert_eq!(
        right,
        [1, 2],
        "and the same two are chosen whatever the order"
    );
}

#[test]
fn a_group_of_twenty_five_is_narrowed_to_the_measured_twenty_frames() {
    let group: Vec<Candidate> = (0..25)
        .map(|index| {
            candidate(
                u8::try_from(index % 5).unwrap_or(0) + 1,
                Some(f64::from(index)),
            )
        })
        .collect();

    let compared = keeper::compared(&group, keeper::MAX);

    assert_eq!(
        compared.len(),
        20,
        "20 previews at 512 px is a quarter of the 16,384-token budget: the cap is the rule, not a \
         limit discovered from a 413"
    );
    assert!(
        compared.iter().all(|&index| group[index].rating > 1),
        "and what is left out is chosen by the rating: only the two weakest frames are: \
         {compared:?}"
    );
    assert_eq!(
        compared.len(),
        group.iter().filter(|frame| frame.rating > 1).count(),
        "nothing rated above the weakest five is left out"
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
