//! Reading the model's answer document into the one typed answer a cull keeps.
//!
//! This is where the wire contract is made strict (spec §7.2): a `choice` is a
//! string key from the rubric and a confidence is the top-option probability —
//! `probabilities` is display-only, never a source for the recorded answer.
//! Anything missing or mistyped becomes a classified `bad_response` rather than
//! a guess, and the raw document is carried along either way so nothing is lost.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::error::{Error, ErrorKind, Result};
use crate::rubric::{RATING_ID, Rubric};
use crate::scoring::{Response, Usage};

/// One frame's answers as read, plus the document they came from.
#[derive(Debug)]
pub struct Reading {
    /// The one typed answer, or the classified reason the document could not be
    /// read. A failure here is `bad_response`, not a missing row.
    pub answers: Result<Answers>,
    /// The tokens and latency the request cost.
    pub usage: Usage,
    /// The raw document, exactly as the server sent it.
    pub document: Value,
}

impl Reading {
    /// Whether the answer came out.
    pub fn is_ok(&self) -> bool {
        self.answers.is_ok()
    }
}

/// The one answer, at the precision the model gave it.
///
/// `stars` is the model's own 1–5 choice — the whole judgement of the frame,
/// never a threshold over anything else — and `confidence` is the top-option
/// probability of that choice.
///
/// Serialised as-is into a ledger record, so a resumed frame's answer is the one
/// the model gave and not a re-reading of it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Answers {
    /// The rating, 1–5, straight from the `choice` key.
    pub stars: u8,
    /// The top-option probability of `stars`: the cheapest available "the model
    /// is unsure" signal.
    pub confidence: f64,
}

/// Read one answer document against the rubric that asked the question.
///
/// Infallible by design: a document that cannot be read comes back as
/// `answers: Err(bad_response)` with the document intact, so the caller can
/// record the row and still write the raw JSON.
pub fn read(rubric: &Rubric, response: Response) -> Reading {
    let answers = extract(rubric, answer_document(&response.document));
    Reading {
        answers,
        usage: usage(&response.document, response.client_latency_ms),
        document: response.document,
    }
}

/// The document the answers live in: the body itself, or the body's `result`
/// object when a Workers AI-style envelope wraps it.
///
/// The rule is one substitution, never a search. A top-level `answers` wins;
/// otherwise a `result` object that has `answers` is read instead; otherwise the
/// body is what it is and the "no `answers` object" failure fires as it always
/// has. So a document that has genuinely drifted still fails loudly rather than
/// being found somewhere deeper by luck.
///
/// Fixture-driven: Clef's own envelope is still open (`cloudflare-clef` ticket
/// 09), and the branch is what protects the next Workers AI model either way.
///
/// Shared with [`keeper::read`](crate::keeper::read), so a comparison and a
/// rating can never disagree about which object answered them.
pub(crate) fn answer_document(document: &Value) -> &Value {
    if document.get("answers").is_some() {
        return document;
    }
    match document.get("result") {
        Some(result) if result.get("answers").is_some() => result,
        _ => document,
    }
}

/// The token and latency numbers. Both are advisory: a server that omits them
/// still answered the question, and the request still took the time it took.
///
/// Read from wherever the answers were read, so a wrapper's own `usage` never
/// shadows the one that belongs to the answer.
fn usage(document: &Value, client_latency_ms: u64) -> Usage {
    let usage = answer_document(document).get("usage");
    let read = |field: &str| {
        usage
            .and_then(|usage| usage.get(field))
            .and_then(Value::as_u64)
    };
    Usage {
        input_tokens: read("input_tokens"),
        server_latency_ms: read("latency_ms"),
        client_latency_ms,
    }
}

fn extract(rubric: &Rubric, document: &Value) -> Result<Answers> {
    let answers = document
        .get("answers")
        .and_then(Value::as_object)
        .ok_or_else(|| {
            bad("the answer document has no `answers` object — the wire contract may have drifted")
        })?;

    let (stars_key, confidence) = choice(answers, RATING_ID)?;
    let question = rubric
        .question(RATING_ID)
        .ok_or_else(|| bad("this rubric has no `stars` question to read the answer against"))?;
    if !question.offers(stars_key) {
        return Err(bad(format!(
            "the `stars` answer `{stars_key}` is not one of the rubric's options"
        )));
    }
    let stars: u8 = stars_key.parse().map_err(|_| {
        bad(format!(
            "the `stars` answer `{stars_key}` is not a whole number"
        ))
    })?;

    Ok(Answers { stars, confidence })
}

/// A `choice` answer: the option key, and its confidence.
fn choice<'a>(answers: &'a Map<String, Value>, id: &str) -> Result<(&'a str, f64)> {
    let answer = answer(answers, id)?;
    let key = answer
        .get("choice")
        .and_then(Value::as_str)
        .ok_or_else(|| bad(format!("the `{id}` answer has no `choice` string")))?;
    let confidence = confidence(answer, id)?;
    Ok((key, confidence))
}

fn answer<'a>(answers: &'a Map<String, Value>, id: &str) -> Result<&'a Value> {
    let answer = answers
        .get(id)
        .ok_or_else(|| bad(format!("the answer document has no answer for `{id}`")))?;
    if !answer.is_object() {
        return Err(bad(format!("the `{id}` answer is not an object")));
    }
    Ok(answer)
}

fn confidence(answer: &Value, id: &str) -> Result<f64> {
    answer
        .get("confidence")
        .and_then(Value::as_f64)
        .ok_or_else(|| bad(format!("the `{id}` answer has no numeric `confidence`")))
}

fn bad(message: impl Into<String>) -> Error {
    Error::new(ErrorKind::BadResponse, message)
}
