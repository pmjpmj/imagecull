//! Which frame of a group to keep: one comparative request per round, and the
//! verdict.
//!
//! The question a group asks is *relative* — "of these twelve, which one" — and
//! the only thing that can answer it is a request that shows the model all of
//! them at once. So a group of two or more gets a
//! [`Comparison`](crate::keeper::Comparison): several previews of one moment, a
//! `choice` question keyed by position, and one answer naming the frame to keep.
//!
//! # The budget is enforced here, not discovered from a 413
//!
//! The reference server holds **16,384 image tokens**, and tokens scale with
//! pixel area: 1,639 at 1568 px, 696 at 1024, 201 at 512. Over the cap it answers
//! `413` and drops nothing, so a whole-group answer or a reported failure are the
//! only two outcomes — but a request that 413s has already been built, paid for
//! and refused. So the request is built to fit: previews at [`EDGE`] px and at
//! most [`MAX`] of them (4,154 tokens, a quarter of the budget, measured).
//!
//! # A group larger than one request is compared in rounds
//!
//! Nothing narrows a group before the model has seen it. A group the budget holds
//! is compared whole; a larger one is split by [`chunk`] into consecutive batches
//! of at most [`MAX`] frames, split evenly so no batch is left with one frame to
//! "compare", each batch answered by one request, and the batch winners compared
//! again, until one survives. Every frame of the group enters a
//! request, which is the point: the per-frame rating cannot see the alternatives,
//! so it is not allowed to decide who is compared — and neither is a measure.
//!
//! Rounds cost requests: a 60-frame burst is three batches and one final, where a
//! shortlist would have paid one. That is the price of the model answering about
//! every frame, and [`shakiest`] is how the run keeps the price visible.
//!
//! # What this module does not do
//!
//! Nothing here prints, writes a file, or knows about a sidecar. The verdict is
//! returned as data — who won, how sure the answer was — and
//! [`cull`](crate::cull) records it. The one model call goes through
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

/// What the model is asked.
///
/// A ranked list of gates, not a conjunction of superlatives: sharpness on the
/// subject decides on its own, the remaining gates only separate frames that are
/// already sharp, and general pleasantness is the last tiebreak — so the trade-off
/// between them is made here rather than left to a `choice` head that has no way to
/// say which one it weighted.
///
/// Four clauses are load-bearing and easy to lose in an edit:
///
/// - **"Rule 1 decides alone … however sharp the rest"** — technical crispness is
///   asked to outrank the moment, the gaze and the composition, which is a real
///   loss as well as a real gain: the frame with the peak expression that is a
///   little soft now loses to the sharp frame with the duller one, and a
///   photographer's own picks routinely trade a few percent of sharpness for the
///   expression. If `calibrate` (spec §11) shows the model disagreeing with the
///   ratings already on past shoots, this sentence is the first place to look — not
///   the thresholds, and not the preview size.
/// - **"the main subject is in sharp focus … or its principal detail"** — at
///   [`EDGE`] px a face in a full-body frame is a few pixels wide, so asking for
///   the eyes alone would ask for something the preview cannot show.
/// - **"if every frame fails rule 1, keep the sharpest and stillest"** — a whole
///   burst can be soft (the focus missed every frame, or the light was too low to
///   hold), and a comparison still has to name one frame: the fallback keeps the
///   answer a frame of the group rather than a refusal, and keeps `read`'s
///   position check meaningful.
/// - **"Do not prefer a frame because it is first"** — the pick was measured
///   swapping between orderings at 0.55 against 0.43
///   (`.scratch/burst-grouping-v1/evidence/comparison-budget.md`), so position is
///   doing work the comparison should not be doing.
///
/// **"Facing the camera"** is the one gate that is a shoot-type preference rather
/// than a universal one: a candid or reportage burst is often *better* with the
/// subject ignoring the lens, and SPEC §11 lists the aesthetic target as exactly
/// the open question that changes a rubric's instructions. It is written as a
/// preference among already-sharp frames, and `shoot_context` is the only way a
/// photographer can override it today — a per-shoot target setting is the honest
/// fix, and that gate is where the bias would be undone.
///
/// **Light is last of the gates.** That is a real trade and not an oversight:
/// clipped highlights and blocked shadows cannot be recovered, while a frame's
/// composition can still be cropped and its expression was never fixable anyway,
/// so a photographer who would reject a blown-out frame outright is not described
/// by this wording. It sits where it does because the folder decides — and if
/// `calibrate` shows the model keeping frames the photographer threw away for the
/// light, this is the rule to move, ahead of composition rather than after it.
///
/// The tiebreak names what *pleasant* means (expression, background, pose,
/// sharpness) instead of leaving one floating adjective for the model to fill in:
/// an unoperationalised "most pleasing" is how a choice head ends up deciding on
/// tone or brightness.
///
/// "Burst" stays in this sentence on purpose: it is the register the reference
/// server was measured in, and the glossary's preference for *group* governs this
/// codebase's prose, not the question the model answers.
///
/// **Not yet measured.** The evidence above measured an earlier, shorter
/// sentence; this wording is a candidate until the same three frames are re-run at
/// every edge and in every ordering, and the swap rate and the count of groups
/// under [`FLOOR`] are recorded beside it. Until then, do not describe a run as
/// sending a measured question.
pub const INSTRUCTIONS: &str = "Several frames of one burst are shown, in order. They are near-identical, and only one is the one to keep.\n\
     The rules are ranked: rule 1 decides by itself, and rules 2 to 5 only separate frames that are already sharp.\n\
     1. Sharp and still, above all else: the main subject in sharp focus — its eyes or face, or its principal detail, crisp — with no motion blur and no camera shake. No frame that fails this is the one to keep, however good its light, moment, gaze or composition, so a sharp frame beats a softer frame with the better expression. If every frame fails this rule, keep the sharpest and stillest of them.\n\
     2. The moment: eyes open, expression and gesture at their peak — not a blink, a step in transit, or a mouth mid-word.\n\
     3. Facing the camera: prefer the frame in which the main subject is looking towards the camera with its face turned to it; a subject that has turned away, or is looking down or off frame, loses to one that has not.\n\
     4. Composition: level horizon, room around the subject, no bystander, hand or flare in this frame alone.\n\
     5. Light kept, last of the gates: detail in white clothing, skin and the sky, no blown highlights and no blocked-up shadows. Prefer the frame that kept the detail over the brighter one.\n\
     Where two frames pass all five equally, keep the more pleasant picture: the better expression, the calmer background, the more natural pose, and the sharper of the two. Do not prefer a frame because it is first, brighter or warmer; keep the one that would be delivered as it is. Answer with the key of that frame.";

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

/// How a group larger than one request is compared: consecutive batches of at
/// most `max` places, in capture order, split as evenly as the count allows.
///
/// The batches of one round hold every place exactly once, and no batch is a
/// single frame unless the whole input is one: a remainder that would stand
/// alone is spread across the batches instead, because asking the model which of
/// one frame to keep is not a comparison and its answer is not evidence. The
/// caller compares each batch and passes the winners back through the same
/// function, so a group of any size is reduced to one frame without a frame ever
/// leaving the comparison.
///
/// `max` is a request's capacity, and a comparative request carries two frames
/// or it is not one: a cap below two is read as two here, and [`crate::cull`]
/// refuses a group outright rather than send it.
pub fn chunk<T: Clone>(places: &[T], max: usize) -> Vec<Vec<T>> {
    let width = max.max(2);
    if places.is_empty() {
        return Vec::new();
    }
    if places.len() <= width {
        return vec![places.to_vec()];
    }
    let batches = places.len().div_ceil(width);
    let (base, extra) = (places.len() / batches, places.len() % batches);
    let mut out = Vec::with_capacity(batches);
    let mut start = 0;
    for index in 0..batches {
        let size = base + usize::from(index < extra);
        out.push(places[start..start + size].to_vec());
        start += size;
    }
    out
}

/// The shakiest call a verdict depended on: the lowest top-option probability on
/// the winner's path through the rounds.
///
/// A tournament's final call can be a near-certainty about two frames whose
/// membership in the final was itself a coin flip. Recording the final round's
/// confidence alone would report a group as sure that was not, so every round the
/// winner survived contributes, and the weakest one is what the sidecar carries.
/// An empty path is `1.0`: a verdict is never reached without a call.
pub fn shakiest(confidences: impl IntoIterator<Item = f64>) -> f64 {
    confidences
        .into_iter()
        .fold(1.0_f64, |lowest, value| lowest.min(value))
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
