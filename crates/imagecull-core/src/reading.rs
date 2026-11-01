//! Reading the model's answer document into six typed answers.
//!
//! This is where the wire contract is made strict (spec §7.2): a `score` is a
//! float recorded at full precision, a `choice` is a string key from the
//! rubric, a confidence is the top-option probability, and `probabilities` is
//! display-only — never a source for the recorded answer. Anything missing or
//! mistyped becomes a classified `bad_response` rather than a guess, and the
//! raw document is carried along either way so nothing is lost.

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Map, Value};

use crate::error::{Error, ErrorKind, Result};
use crate::rubric::{Rubric, SCORE_IDS, STARS_ID, VERDICT_ID};
use crate::scoring::{Response, Usage};

/// One frame's answers as read, plus the document they came from.
#[derive(Debug)]
pub struct Reading {
    /// The six typed answers, or the classified reason the document could not
    /// be read. A failure here is `bad_response`, not a missing row.
    pub answers: Result<Answers>,
    /// The tokens and latency the request cost.
    pub usage: Usage,
    /// The raw document, exactly as the server sent it.
    pub document: Value,
}

impl Reading {
    /// Whether six answers came out.
    pub fn is_ok(&self) -> bool {
        self.answers.is_ok()
    }
}

/// The six answers, at the precision the model gave them.
///
/// The four scores are kept apart on purpose: never combined, never rescaled
/// to 100. `stars` is the model's own five-way choice, not a threshold over the
/// scores, and `verdict` is its keep/drop opinion.
///
/// Serialised as-is into a ledger record, so a resumed frame's answers are the
/// ones the model gave and not a re-reading of them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Answers {
    /// The keeper judgement, framed around photojournalistic keepers.
    pub overall: f64,
    /// Technical quality only.
    pub technical: f64,
    /// Framing and graphic design only.
    pub composition: f64,
    /// Moment, gesture and light only.
    pub impact: f64,
    /// The star rating, 1–5, straight from the `choice` key.
    pub stars: u8,
    /// The keep/drop verdict, straight from the `choice` key.
    pub verdict: Verdict,
    /// Per-answer confidences: the cheapest available "the model is unsure"
    /// signal.
    pub confidence: Confidences,
}

/// The top-option probability of each answer.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Confidences {
    /// Confidence in `overall`.
    pub overall: f64,
    /// Confidence in `technical`.
    pub technical: f64,
    /// Confidence in `composition`.
    pub composition: f64,
    /// Confidence in `impact`.
    pub impact: f64,
    /// Confidence in `stars`.
    pub stars: f64,
    /// Confidence in `verdict`.
    pub verdict: f64,
}

/// The three-way keep/drop opinion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Verdict {
    /// The model would not keep the frame.
    Dislike,
    /// The model could keep it or not.
    Neutral,
    /// The model would keep and show it.
    Like,
}

impl Verdict {
    /// The key as the rubric and the CSV spell it.
    pub fn as_str(self) -> &'static str {
        match self {
            Verdict::Dislike => "dislike",
            Verdict::Neutral => "neutral",
            Verdict::Like => "like",
        }
    }

    /// The verdict a rubric key names, if it names one.
    pub fn from_key(key: &str) -> Option<Verdict> {
        match key {
            "dislike" => Some(Verdict::Dislike),
            "neutral" => Some(Verdict::Neutral),
            "like" => Some(Verdict::Like),
            _ => None,
        }
    }
}

/// A verdict in a ledger record is the key the rubric and the CSV use, and the
/// same spellings [`Verdict::as_str`] and [`Verdict::from_key`] give: one
/// spelling per concept, not a second serde-derived one.
impl Serialize for Verdict {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for Verdict {
    fn deserialize<D: Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Verdict, D::Error> {
        let key = String::deserialize(deserializer)?;
        Verdict::from_key(&key)
            .ok_or_else(|| serde::de::Error::custom(format!("`{key}` is not a verdict")))
    }
}

/// Read one answer document against the rubric that asked the questions.
///
/// Infallible by design: a document that cannot be read comes back as
/// `answers: Err(bad_response)` with the document intact, so the caller can
/// record the row and still write the raw JSON.
pub fn read(rubric: &Rubric, response: Response) -> Reading {
    let answers = extract(rubric, &response.document);
    Reading {
        answers,
        usage: usage(&response.document, response.client_latency_ms),
        document: response.document,
    }
}

/// The token and latency numbers. Both are advisory: a server that omits them
/// still answered the questions, and the request still took the time it took.
fn usage(document: &Value, client_latency_ms: u64) -> Usage {
    let usage = document.get("usage");
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

    let (overall, conf_overall) = score(answers, SCORE_IDS[0])?;
    let (technical, conf_technical) = score(answers, SCORE_IDS[1])?;
    let (composition, conf_composition) = score(answers, SCORE_IDS[2])?;
    let (impact, conf_impact) = score(answers, SCORE_IDS[3])?;

    let (stars_key, conf_stars) = choice(answers, STARS_ID)?;
    let stars_question = rubric
        .question(STARS_ID)
        .ok_or_else(|| bad("this rubric has no `stars` question to read the answer against"))?;
    if !stars_question.offers(stars_key) {
        return Err(bad(format!(
            "the `stars` answer `{stars_key}` is not one of the rubric's options"
        )));
    }
    let stars: u8 = stars_key.parse().map_err(|_| {
        bad(format!(
            "the `stars` answer `{stars_key}` is not a whole number"
        ))
    })?;

    let (verdict_key, conf_verdict) = choice(answers, VERDICT_ID)?;
    let verdict_question = rubric
        .question(VERDICT_ID)
        .ok_or_else(|| bad("this rubric has no `verdict` question to read the answer against"))?;
    if !verdict_question.offers(verdict_key) {
        return Err(bad(format!(
            "the `verdict` answer `{verdict_key}` is not one of the rubric's options"
        )));
    }
    let verdict = Verdict::from_key(verdict_key).ok_or_else(|| {
        bad(format!(
            "the `verdict` answer `{verdict_key}` is not a keep/drop verdict"
        ))
    })?;

    Ok(Answers {
        overall,
        technical,
        composition,
        impact,
        stars,
        verdict,
        confidence: Confidences {
            overall: conf_overall,
            technical: conf_technical,
            composition: conf_composition,
            impact: conf_impact,
            stars: conf_stars,
            verdict: conf_verdict,
        },
    })
}

/// A `score` answer: the expected value over the legend, and its confidence.
///
/// `probabilities` and `legend` are deliberately not read: they are display
/// only, and deriving the recorded value from them would hide the server
/// changing what it puts in `score` (spec §7.2).
fn score(answers: &Map<String, Value>, id: &str) -> Result<(f64, f64)> {
    let answer = answer(answers, id)?;
    let value = answer
        .get("score")
        .and_then(Value::as_f64)
        .ok_or_else(|| bad(format!("the `{id}` answer has no numeric `score`")))?;
    let confidence = confidence(answer, id)?;
    Ok((value, confidence))
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
