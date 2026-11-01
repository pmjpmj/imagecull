//! Which frame of a group to keep: one comparative request, and the verdict.
//!
//! The question a group asks is *relative* — "of these twelve, which one" — and
//! the only thing that can answer it is a request that shows the model all of
//! them at once. So a group of two or more gets one
//! [`Comparison`](crate::keeper::Comparison): several previews of one moment, a
//! `choice` question keyed by position, and one answer naming the frame to keep.
//!
//! # The budget is enforced here, not discovered from a 413
//!
//! The reference server holds **16,384 image tokens**, and tokens scale with
//! pixel area: 1,639 at 1568 px, 696 at 1024, 201 at 512. Over the cap it answers
//! `413` and drops nothing, so a whole-group answer or a reported failure are the
//! only two outcomes — but a request that 413s has already been built, paid for
//! and refused. So the request is built to fit: previews at [`EDGE`] px, at most
//! [`MAX`] of them (4,154 tokens, a quarter of the budget, measured), and a group
//! that holds more is narrowed to the [`MAX`] frames most worth comparing —
//! highest rating first, ties broken by [`acutance`](crate::measure) because the
//! rating is the model's judgement and the measure is the only other thing that
//! can order two equally-rated frames. That narrowing chooses **who is compared**,
//! never a rating, and the frames it leaves out are still marked as non-keepers.
//!
//! # What this module does not do
//!
//! Nothing here prints, writes a file, or knows about a sidecar. The verdict is
//! returned as data — who won, how sure the answer was — and
//! [`scoring`](crate::scoring) records it. The one model call goes through
//! [`Decider::compare`](crate::scoring::Decider::compare), the same seam every
//! per-frame request goes through, so the whole feature is testable with no
//! server running.
//!
//! # The answer is a coin flip on a true near-tie, and it still decides
//!
//! Measured on a real three-frame group: the pick swapped between orderings at
//! 0.55 against 0.43, and between preview sizes. A confidence floor would
//! therefore go silent on exactly the groups this exists for. Instead the
//! confidence is recorded beside the verdict on every member, and a group below
//! [`FLOOR`] is counted on the summary line: the photographer is told which
//! groups the model was unsure about, rather than being handed a filter that
//! quietly omits them.

use serde_json::Value;

use crate::preprocess::Encoded;
use crate::scoring::{Response, State};

/// The longest edge of a comparison preview, in pixels: measured decisive on a
/// real difference (0.98 sharp against soft) and 3× cheaper than 1024.
pub const EDGE: u32 = 512;

/// The most frames one comparative request carries: 4,154 tokens of the
/// 16,384-token image budget, measured.
pub const MAX: usize = 20;

/// The confidence under which a group's verdict is reported as a low-confidence
/// one — and still written. Measured near-ties sit at 0.55 against 0.43, so a
/// gate here would silence the feature rather than improve it.
pub const FLOOR: f64 = 0.60;

/// The id of the question a comparison asks.
pub const QUESTION: &str = "keeper";

/// What chose a keeper, recorded beside it.
pub const BY_COMPARISON: &str = "comparison";

/// What the model is asked, in the evidence's own words.
///
/// "Burst" stays in this sentence on purpose. It is the wording that was measured
/// against the reference server (`.scratch/burst-grouping-v1/evidence/`), and the
/// glossary's preference for *group* governs this codebase's prose, not the
/// question the model answered best: change it and the measurement no longer
/// describes what a run sends.
pub const INSTRUCTIONS: &str = "Several frames of one burst are shown, in order. Decide which single frame is the one to \
     keep: the sharpest and the one whose subject, expression and light hold together best. \
     Answer with the key of that frame.";

/// How much of a group one comparative request may carry, and how sure its
/// answer has to be before it is not called shaky.
///
/// The defaults are the measured ones and are part of the algorithm, like
/// [`Thresholds`](crate::group::Thresholds): a run does not take them from the
/// command line, because a photographer choosing them would be choosing how much
/// picture the model gets to see, which is a fact about the model.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Budget {
    /// The longest edge of each preview in the request, in pixels.
    pub edge: u32,
    /// The most previews one request carries.
    pub max: usize,
    /// The confidence below which a group is counted as low-confidence.
    pub floor: f64,
}

impl Default for Budget {
    fn default() -> Budget {
        Budget {
            edge: EDGE,
            max: MAX,
            floor: FLOOR,
        }
    }
}

/// One frame of a group, as the choice among them is made: the judgement it
/// already carries, and the one measure that can order two equal judgements.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Candidate {
    /// The rating the frame was given, 1–5.
    pub rating: u8,
    /// How sharp it is, when it could be measured.
    pub acutance: Option<f64>,
}

/// Which of a group's frames enter its comparison, as indices into
/// `candidates`, in capture order.
///
/// A group no larger than the budget is compared whole. A larger one keeps the
/// best-rated frames, breaks a rating tie on acutance, and leaves the rest out:
/// the rating is the judgement the model is good at, and acutance is the only
/// other fact in hand. A frame with no acutance sorts below one that has a
/// number — a measure that failed is not a sharp frame.
pub fn compared(candidates: &[Candidate], max: usize) -> Vec<usize> {
    let mut order: Vec<usize> = (0..candidates.len()).collect();
    order.sort_by(|&left, &right| {
        let (first, second) = (&candidates[left], &candidates[right]);
        second
            .rating
            .cmp(&first.rating)
            .then_with(|| sharpness(second.acutance).total_cmp(&sharpness(first.acutance)))
            // Equal judgements and equal measures: the earlier frame first, so
            // the shortlist is a function of the group and not of a sort's mood.
            .then_with(|| left.cmp(&right))
    });
    order.truncate(max.max(1));
    // Back into capture order: the criteria the model reads are positions in a
    // group, not a ranking, and the answer has to be a frame rather than a
    // order the model was silently handed.
    order.sort_unstable();
    order
}

/// A frame's acutance as a sort key, with "could not be measured" below every
/// number rather than at zero.
fn sharpness(acutance: Option<f64>) -> f64 {
    acutance.unwrap_or(f64::NEG_INFINITY)
}

/// One group's comparative request: several frames of one moment, and the
/// question which of them to keep.
///
/// Owned rather than borrowed, unlike [`Request`](crate::scoring::Request),
/// because the previews are rendered for this request alone at [`Budget::edge`]
/// and die with it.
#[derive(Debug, Clone, PartialEq)]
pub struct Comparison {
    /// The alias the request asks.
    pub model: String,
    /// The framing the group is judged in.
    pub state: State,
    /// The previews to compare, in the order the criteria name them.
    pub previews: Vec<Encoded>,
}

/// What a comparison answered: which of the frames it was shown to keep, and how
/// sure it was.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Choice {
    /// The frame it chose, counting from 1 in the order the request put them.
    pub position: usize,
    /// The top-option probability of that choice.
    pub confidence: f64,
}

impl Choice {
    /// The index of the chosen frame in the request's own order.
    pub fn index(&self) -> usize {
        self.position.saturating_sub(1)
    }
}

/// Read a comparative answer document, at the strictness [`reading`] applies to a
/// rating: a choice that is not one of the frames offered, or a document without
/// one, is a classified failure rather than a guess.
///
/// `offered` is how many frames the request carried. It is passed in because the
/// document does not know: an answer naming position 7 of a four-frame request is
/// a broken answer, and marking the wrong frame red is worse than reporting the
/// group as undecided.
///
/// [`reading`]: crate::reading
pub fn read(response: &Response, offered: usize) -> crate::Result<Choice> {
    let answer = response
        .document
        .get("answers")
        .and_then(|answers| answers.get(QUESTION))
        .ok_or_else(|| bad("the comparative answer has no `keeper` answer"))?;
    let key = answer
        .get("choice")
        .and_then(Value::as_str)
        .ok_or_else(|| bad("the `keeper` answer has no `choice` string"))?;
    let position: usize = key
        .trim()
        .parse()
        .map_err(|_| bad(format!("the `keeper` answer `{key}` is not a position")))?;
    if position == 0 || position > offered {
        return Err(bad(format!(
            "the `keeper` answer names frame {position}, and the request showed {offered}"
        )));
    }
    let confidence = answer
        .get("confidence")
        .and_then(Value::as_f64)
        .ok_or_else(|| bad("the `keeper` answer has no numeric `confidence`"))?;
    Ok(Choice {
        position,
        confidence,
    })
}

/// A comparative answer that cannot be read: `bad_response`, never a pick.
fn bad(message: impl Into<String>) -> crate::Error {
    crate::Error::new(crate::ErrorKind::BadResponse, message)
}
