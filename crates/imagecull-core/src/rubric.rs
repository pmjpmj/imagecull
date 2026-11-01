//! The rubric: the six questions, the TOML they are written in, and the hash
//! that names them.
//!
//! A rubric is a file a photographer edits between shoots, so it is loaded,
//! validated and turned into an identity before a single request goes out
//! (spec §8). The four `score` questions are independent — never combined,
//! never rescaled — and the two `choice` questions carry the star rating and
//! the keep/drop verdict. Criteria are ordered arrays rather than tables
//! because the order is part of the legend the model reads.

use std::fmt;
use std::fs;
use std::path::Path;

use serde::Deserialize;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use crate::error::{Error, Result};

/// The four `score` questions, in the order a reading reports them.
pub const SCORE_IDS: [&str; 4] = ["overall", "technical", "composition", "impact"];

/// The five-way choice: the star rating.
pub const STARS_ID: &str = "stars";

/// The three-way choice: the keep/drop verdict.
pub const VERDICT_ID: &str = "verdict";

/// The ids a rubric must ask, in the canonical order they are reported in.
///
/// A rubric that renames or drops one is rejected at load: the CSV, the ledger
/// and (v2) the XMP sidecar all have a column for each of these six.
pub const REQUIRED_IDS: [&str; 6] = [
    SCORE_IDS[0],
    SCORE_IDS[1],
    SCORE_IDS[2],
    SCORE_IDS[3],
    STARS_ID,
    VERDICT_ID,
];

/// The star keys, in order. They are also what the integer column parses.
const STAR_KEYS: [&str; 5] = ["1", "2", "3", "4", "5"];

/// The verdict keys, in order.
const VERDICT_KEYS: [&str; 3] = ["dislike", "neutral", "like"];

/// The legend keys a `score` question must offer, in order: the answer is the
/// expected value over these levels, so a legend that starts at 1 or has twelve
/// levels would silently shift every score.
const SCORE_KEYS: [&str; 11] = ["0", "1", "2", "3", "4", "5", "6", "7", "8", "9", "10"];

/// The rubric shipped inside the binary (spec §8).
const EMBEDDED: &str = include_str!("../rubrics/default.toml");

/// A loaded, validated rubric.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rubric {
    model: String,
    questions: Vec<Question>,
}

impl Rubric {
    /// The rubric compiled into the binary.
    ///
    /// Parsed on every call rather than cached: it is a few kilobytes of TOML,
    /// and a cached copy would be one more thing to invalidate.
    pub fn embedded() -> Result<Rubric> {
        Self::from_toml(EMBEDDED).map_err(|error| {
            Error::fatal(format!(
                "the embedded rubric is broken: {}",
                error.message()
            ))
        })
    }

    /// Load `path`, or the embedded rubric when there is no path — `--rubric`.
    pub fn load(path: Option<&Path>) -> Result<Rubric> {
        match path {
            None => Self::embedded(),
            Some(path) => {
                let text = fs::read_to_string(path).map_err(|error| {
                    Error::fatal(format!(
                        "cannot read the rubric file {}: {error}",
                        path.display()
                    ))
                })?;
                Self::from_toml(&text).map_err(|error| {
                    Error::fatal(format!(
                        "the rubric file {} is not usable: {}",
                        path.display(),
                        error.message()
                    ))
                })
            }
        }
    }

    /// Parse and validate rubric TOML.
    ///
    /// Every failure here is fatal: a rubric that cannot be trusted would fail
    /// identically for all 400 files, so it must not become 400 bad rows.
    pub fn from_toml(text: &str) -> Result<Rubric> {
        let file: RubricFile = toml::from_str(text)
            .map_err(|error| Error::fatal(format!("the rubric is not valid TOML: {error}")))?;
        file.validate()
    }

    /// The alias this rubric was written for, overridable with `--model`.
    pub fn model(&self) -> &str {
        &self.model
    }

    /// The questions, in declaration order.
    pub fn questions(&self) -> &[Question] {
        &self.questions
    }

    /// The question with `id`, if the rubric asks it.
    pub fn question(&self, id: &str) -> Option<&Question> {
        self.questions.iter().find(|question| question.id == id)
    }

    /// The questions exactly as they go on the wire, keyed by id in
    /// declaration order.
    ///
    /// A `score`'s criteria are the ordered legend of level descriptions, whose
    /// index is the key (the loader guarantees the keys are `"0"`…`"10"` in
    /// order): that is the shape the API's score head reads. A `choice`'s
    /// criteria are an object keyed by option, in declaration order, because
    /// that is the shape its choice head reads. Option order survives in both.
    pub(crate) fn questions_json(&self) -> Value {
        let mut questions = Map::new();
        for question in &self.questions {
            questions.insert(question.id.clone(), question.to_json());
        }
        Value::Object(questions)
    }

    /// The rubric's identity: SHA-256 over the canonicalised schema plus the
    /// model alias that will actually be used (spec §8.3).
    ///
    /// Canonicalisation is by construction: this hashes the parsed schema
    /// rendered as it goes on the wire, not the file, so whitespace, comments,
    /// quoting style and key order are irrelevant while instructions, criteria,
    /// their order, the alias — and any change to how the schema is expressed
    /// to the model — are not.
    pub fn hash(&self, model: &str) -> RubricHash {
        let mut digest = Sha256::new();
        // Length-prefixed, so no instruction can forge a field boundary.
        let mut field = |value: &str| {
            digest.update(u64::try_from(value.len()).unwrap_or(u64::MAX).to_be_bytes());
            digest.update(value.as_bytes());
        };
        field("imagecull-rubric-v1");
        field(model);
        field(&self.questions_json().to_string());
        let bytes = digest.finalize();
        let mut hex = String::with_capacity(bytes.len() * 2);
        for byte in bytes {
            use fmt::Write as _;
            let _ = write!(hex, "{byte:02x}");
        }
        RubricHash(hex)
    }
}

/// A rubric's identity, as 64 lowercase hex characters.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RubricHash(String);

impl RubricHash {
    /// The full hash.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The first eight characters: what a CSV row carries, so a reader can see
    /// two rows came from the same rubric without a 64-character column.
    pub fn short(&self) -> &str {
        &self.0[..8]
    }
}

impl fmt::Display for RubricHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// One question, with its criteria in the order the model should read them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Question {
    id: String,
    kind: QuestionKind,
    instructions: String,
    criteria: Vec<Criterion>,
}

impl Question {
    /// The question's key in the request's `questions` object and the answer's
    /// key in `answers`.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Whether this question asks for a score or a choice.
    pub fn kind(&self) -> QuestionKind {
        self.kind
    }

    /// What the model is asked, in words.
    pub fn instructions(&self) -> &str {
        &self.instructions
    }

    /// The options, in declaration order.
    pub fn criteria(&self) -> &[Criterion] {
        &self.criteria
    }

    /// Whether `key` is one of the options this question offers.
    pub fn offers(&self, key: &str) -> bool {
        self.criteria.iter().any(|criterion| criterion.key == key)
    }

    /// This question as the request carries it.
    fn to_json(&self) -> Value {
        let criteria = match self.kind {
            QuestionKind::Score => Value::Array(
                self.criteria
                    .iter()
                    .map(|criterion| Value::String(criterion.description.clone()))
                    .collect(),
            ),
            QuestionKind::Choice => {
                let mut options = Map::new();
                for criterion in &self.criteria {
                    options.insert(
                        criterion.key.clone(),
                        Value::String(criterion.description.clone()),
                    );
                }
                Value::Object(options)
            }
        };
        let mut question = Map::new();
        question.insert(
            "type".to_string(),
            Value::String(self.kind.as_str().to_string()),
        );
        question.insert(
            "instructions".to_string(),
            Value::String(self.instructions.clone()),
        );
        question.insert("criteria".to_string(), criteria);
        Value::Object(question)
    }
}

/// The two question types a cull uses. `noul` exists in the wire contract but
/// an image cull has no use for a yes/no question.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuestionKind {
    /// An ordered legend; the answer is an expected value over it.
    Score,
    /// One named option, such as a star rating.
    Choice,
}

impl QuestionKind {
    /// The `type` on the wire.
    pub fn as_str(self) -> &'static str {
        match self {
            QuestionKind::Score => "score",
            QuestionKind::Choice => "choice",
        }
    }
}

/// One option: the key recorded in the answer, and the description the model
/// reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Criterion {
    key: String,
    description: String,
}

impl Criterion {
    /// The key, as the answer reports it.
    pub fn key(&self) -> &str {
        &self.key
    }

    /// The description shown to the model.
    pub fn description(&self) -> &str {
        &self.description
    }
}

/// The TOML file, before validation.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RubricFile {
    model: String,
    #[serde(default)]
    question: Vec<QuestionFile>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct QuestionFile {
    id: String,
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    instructions: String,
    #[serde(default)]
    criteria: Vec<CriterionFile>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CriterionFile {
    key: String,
    #[serde(default)]
    description: String,
}

impl RubricFile {
    fn validate(self) -> Result<Rubric> {
        let model = self.model.trim().to_string();
        if model.is_empty() {
            return Err(Error::fatal(
                "the rubric names no `model` alias — put the alias this wording was written for at \
                 the top of the file, or pass --model",
            ));
        }

        let mut questions = Vec::with_capacity(self.question.len());
        for (position, question) in self.question.into_iter().enumerate() {
            questions.push(question.validate(position)?);
        }

        let ids: Vec<&str> = questions
            .iter()
            .map(|question| question.id.as_str())
            .collect();
        for id in REQUIRED_IDS {
            if !ids.contains(&id) {
                return Err(Error::fatal(format!(
                    "a rubric asks exactly six questions — {} — and this one asks: {}. `{id}` is \
                     missing",
                    REQUIRED_IDS.join(", "),
                    describe(&ids),
                )));
            }
        }
        if questions.len() != REQUIRED_IDS.len() {
            return Err(Error::fatal(format!(
                "a rubric asks exactly six questions — {} — and this one asks: {}",
                REQUIRED_IDS.join(", "),
                describe(&ids),
            )));
        }

        Ok(Rubric { model, questions })
    }
}

impl QuestionFile {
    fn validate(self, position: usize) -> Result<Question> {
        let id = self.id.trim().to_string();
        if id.is_empty() {
            return Err(Error::fatal(format!(
                "rubric question {} has no `id`",
                position + 1
            )));
        }
        let kind = match self.kind.trim() {
            "score" => QuestionKind::Score,
            "choice" => QuestionKind::Choice,
            other => {
                return Err(Error::fatal(format!(
                    "rubric question `{id}` has type `{other}`; a cull uses `score` or `choice`"
                )));
            }
        };
        let instructions = self.instructions.trim().to_string();
        if instructions.is_empty() {
            return Err(Error::fatal(format!(
                "rubric question `{id}` has no instructions"
            )));
        }
        if self.criteria.is_empty() {
            return Err(Error::fatal(format!(
                "rubric question `{id}` offers no criteria"
            )));
        }

        let mut criteria = Vec::with_capacity(self.criteria.len());
        for criterion in self.criteria {
            let key = criterion.key.trim().to_string();
            if key.is_empty() {
                return Err(Error::fatal(format!(
                    "rubric question `{id}` has a criterion with no key"
                )));
            }
            if criteria.iter().any(|seen: &Criterion| seen.key == key) {
                return Err(Error::fatal(format!(
                    "rubric question `{id}` offers `{key}` twice"
                )));
            }
            criteria.push(Criterion {
                key,
                description: criterion.description.trim().to_string(),
            });
        }

        // Which id asks which kind of question, and what it must offer: the
        // CSV, the ledger and the v2 sidecar all depend on these six shapes.
        let (wanted, expected): (QuestionKind, &[&str]) = match id.as_str() {
            STARS_ID => (QuestionKind::Choice, &STAR_KEYS),
            VERDICT_ID => (QuestionKind::Choice, &VERDICT_KEYS),
            overall if SCORE_IDS.contains(&overall) => (QuestionKind::Score, &SCORE_KEYS),
            _ => {
                return Err(Error::fatal(format!(
                    "rubric question `{id}` is not one a cull reports; a rubric asks: {}",
                    REQUIRED_IDS.join(", ")
                )));
            }
        };
        if kind != wanted {
            return Err(Error::fatal(format!(
                "rubric question `{id}` must be a `{}` question, not `{}`",
                wanted.as_str(),
                kind.as_str()
            )));
        }
        let keys: Vec<&str> = criteria
            .iter()
            .map(|criterion| criterion.key.as_str())
            .collect();
        if keys != expected {
            return Err(Error::fatal(format!(
                "rubric question `{id}` must offer {}, in that order; it offers: {}",
                describe(expected),
                describe(&keys),
            )));
        }
        if kind == QuestionKind::Score && criteria.iter().any(|c| c.description.is_empty()) {
            return Err(Error::fatal(format!(
                "rubric score `{id}` must describe every level of its legend"
            )));
        }

        Ok(Question {
            id,
            kind,
            instructions,
            criteria,
        })
    }
}

/// `[a, b, c]`, or `nothing` when empty, for messages.
fn describe(items: &[&str]) -> String {
    if items.is_empty() {
        "nothing".to_string()
    } else {
        format!("[{}]", items.join(", "))
    }
}
