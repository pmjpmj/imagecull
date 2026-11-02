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
//! and refused. So the request is built to fit: previews at [`EDGE`] px by
//! default and at most [`MAX`] of them (4,154 tokens, a quarter of the budget,
//! measured). Both numbers in that shape are defaults a run may move: `[group]
//! max_edge` or `groups --max-edge` for the picture's detail, and `[group]
//! max_images` for how many images the server takes at once — a host that refuses
//! more than four is a fact about the host, and [`chunk`] splits a group into
//! batches of the cap either way. The confidence [`FLOOR`] is the one number that
//! stays measured.
//!
//! # A group larger than one request is compared in rounds
//!
//! Nothing narrows a group before the model has seen it. A group the budget holds
//! is compared whole; a larger one is split by [`chunk`] into consecutive batches
//! of at most the run's cap — [`MAX`] by default, `[group] max_images` when a
//! server takes fewer — split evenly so no batch is left with one frame to
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
//! confidence is recorded beside the verdict on every member, a group below
//! [`FLOOR`] is counted on the summary line, and its non-keepers are marked
//! yellow instead of red: the photographer is told which groups the model was
//! unsure about, rather than being handed a filter that quietly omits them or a
//! red that claims a certainty the run does not have.

use serde_json::Value;

use crate::preprocess::{Encoded, Params};
use crate::scoring::{Response, State};

/// The longest edge of a comparison preview, in pixels: measured decisive on a
/// real difference (0.98 sharp against soft) and 3× cheaper than 1024.
///
/// This is the default, not a constant of the algorithm: a run sends its
/// comparison previews at [`Params::max_edge`](crate::preprocess::Params::max_edge),
/// which [`cull::Config::preview`](crate::cull::Config::preview) carries. A
/// larger edge sees more detail and costs more tokens per frame, so [`MAX`]
/// frames at 1568 px no longer fit one request and the group's `413` says which
/// flag would make it fit.
pub const EDGE: u32 = 512;

/// The payload shape of one image in a comparison, when the run said nothing:
/// [`EDGE`] px, at the quality a rating would encode at.
///
/// The quality is not a second measurement — it only moves bytes on the wire —
/// so the two commands share it and the edge is the number a cull can differ on.
pub const fn default_preview() -> Params {
    Params {
        max_edge: EDGE,
        jpeg_quality: Params::DEFAULT_JPEG_QUALITY,
    }
}

/// The most frames one comparative request carries by default: 4,154 tokens of
/// the 16,384-token image budget, measured.
///
/// It is the reference server's measured batch, not a constant of the algorithm:
/// a host that takes fewer images per request sets `[group] max_images`, and
/// [`chunk`] batches a group by that cap instead. A smaller cap costs requests
/// and changes no verdict, because every round's winners are compared again.
pub const MAX: usize = 20;

/// The confidence under which a group's verdict is reported as a low-confidence
/// one — and still written, on non-keepers as Yellow rather than Red. Measured
/// near-ties sit at 0.55 against 0.43, so a gate here would silence the feature
/// rather than improve it.
pub const FLOOR: f64 = 0.60;

/// The id of the question a comparison asks.
pub const QUESTION: &str = "keeper";

/// What chose a keeper, recorded beside it.
pub const BY_COMPARISON: &str = "comparison";

/// What the model is asked.
///
/// One stance, not a ranked list: the frame is judged whole — what it is trying
/// to be, whether it caught anything worth looking at, and whether it is made
/// well enough to carry that — which is the stance `general` already sends per
/// frame (ADR 0015). The comparison asks the same stance *relative* to the other
/// frames of the group; it does not add a rule the ratings never had.
///
/// That is a replacement, not a refinement. The wording here used to rank five
/// gates with technical crispness first and "rule 1 decides by itself": a sharp
/// frame beat a softer frame with the better expression, and a whole soft burst
/// was answered by "keep the sharpest and stillest". `groups` therefore named a
/// different frame of a moment than `score` called the best of it, and the
/// frames it passed over were the ones the general rubric deliberately keeps —
/// slightly soft, holding the moment. The two commands describe one judgement;
/// when they disagree the wording is the thing that is wrong.
///
/// Four clauses are load-bearing and easy to lose in an edit:
///
/// - **"Read each frame for what it is trying to be … without holding it to
///   something it never aimed at"** — no genre is assumed. The frames of one
///   group are one moment, usually one subject, but not always one intent: a
///   turn, a zoom or a re-frame inside a burst changes what the frame is aiming
///   at, and a comparison that assumes a portrait punishes the landscape beside
///   it.
/// - **"is there something worth looking at, is it caught, and is the frame made
///   well enough to carry it"** — the three things judged, and the order in which
///   they decide. Perfection is last on purpose, exactly as `general` has it, so
///   a soft frame that holds something can win.
/// - **"cannot carry the picture … is not the one to keep, however good its
///   moment"** — the floor that keeps the stance from becoming a preference for
///   anything interesting over anything usable. Without it a smeared frame with
///   the best expression wins, and the mark sends the photographer to a frame
///   they cannot deliver.
/// - **"Do not prefer a frame because it is first"** — the pick was measured
///   swapping between orderings at 0.55 against 0.43
///   (`.scratch/burst-grouping-v1/evidence/comparison-budget.md`), so position is
///   doing work the comparison should not be doing.
///
/// **"Facing the camera" retired with the ranking.** It was the one gate that was
/// a shoot-type preference rather than a universal one — a candid or reportage
/// burst is often *better* with the subject ignoring the lens — and `general`
/// names no such target, so the comparison names none either. Direct address is
/// now one cue among several, not a gate that outranks light and composition.
///
/// The vocabulary is the glossary's: *moment* for the group, *frame* for what is
/// shown. "Burst" stayed in the old sentence because it was the register the
/// reference server was measured in; that measurement belongs to wording this
/// file no longer sends, so it does not bind the new text.
///
/// **Not yet measured.** The evidence above measured an earlier, shorter
/// sentence, and the gates it justified are now retired; this wording is a
/// candidate until the same three frames are re-run at every edge and in every
/// ordering, with the swap rate and the count of groups under [`FLOOR`] recorded
/// beside it. If a real shoot shows the comparison keeping frames the
/// photographer threw away for focus — or preferring a sharp frame in which
/// nothing happens — this const is where to look, not the thresholds and not the
/// preview size. Until then, do not describe a run as sending a measured
/// question.
pub const INSTRUCTIONS: &str = "Several frames of one moment are shown, in order. They are close enough that only one is the one to keep.\n\
     Read each frame for what it is trying to be — a portrait, a landscape, an event, a street frame, a snapshot — and judge it as that, without holding it to something it never aimed at.\n\
     The standard does not move with the genre: is there something worth looking at, is it caught, and is the frame made well enough to carry it? A distinctive moment, gesture, expression or quality of light counts for more than clinical perfection: a frame that is slightly soft but holds something beats a sharp frame in which nothing happens.\n\
     A frame that cannot carry the picture — the subject smeared by motion, the focus on the background, the highlights blown to white — is not the one to keep, however good its moment.\n\
     Do not prefer a frame because it is first, brighter or warmer; keep the one that would be delivered as it is. Answer with the key of that frame.";

/// How much of a group one comparative request may carry, and how sure its
/// answer has to be before it is not called shaky.
///
/// `max` is a capacity rather than a judgement: [`MAX`] is the measured batch on
/// the reference server, and `[group] max_images` moves it for a host that takes
/// fewer images per request. [`chunk`] splits any group into batches of it, so a
/// smaller cap costs requests and changes no verdict. `floor` stays measured,
/// like [`Thresholds`](crate::group::Thresholds): how sure is sure is a fact
/// about the model, not a preference.
///
/// The *size* of each preview is not here: it is the run's own [`Params`],
/// defaulting to [`EDGE`], because a
/// photographer weighing token budget against detail is asking about the picture
/// and not about the comparison's shape.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Budget {
    /// The most previews one request carries.
    pub max: usize,
    /// The confidence below which a group is counted as low-confidence and its
    /// non-keepers are marked yellow rather than red.
    pub floor: f64,
}

impl Default for Budget {
    fn default() -> Budget {
        Budget {
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
/// because the previews are rendered for this request alone, at the run's
/// comparison [`Params`], and die with it.
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
