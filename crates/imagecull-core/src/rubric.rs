//! The rubric: the one question, the TOML it is written in, and the hash that
//! names it.
//!
//! A rubric is a file a photographer edits between shoots, so it is loaded,
//! validated and turned into an identity before a single request goes out
//! (spec §8). Three of them are compiled into the binary ([`Builtin`]) and
//! selected by name; any other is a file selected by path ([`RubricSource`]).
//! A cull asks one `choice` question — the 1–5 rating — and nothing else: the
//! frame is judged whole in a single call, so there is no set of sub-scores to
//! combine or to reconcile. Criteria are ordered arrays rather than tables
//! because the order is part of the legend the model reads.

use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use crate::error::{Error, Result};

/// The five-way choice: the 1–5 rating, the whole of a cull's judgement.
pub const RATING_ID: &str = "stars";

/// The ids a rubric must ask, in the canonical order they are reported in.
///
/// A rubric that renames or drops one is rejected at load: the CSV, the ledger
/// and (v2) the XMP sidecar all have a column for this one.
pub const REQUIRED_IDS: [&str; 1] = [RATING_ID];

/// The rating keys, in order. They are also what the integer column parses.
const RATING_KEYS: [&str; 5] = ["1", "2", "3", "4", "5"];

/// The rubrics compiled into the binary, one file each, each of which stays in
/// the tree as the copy a photographer can read and start a rubric of their own
/// from.
const GENERAL: &str = include_str!("../rubrics/general.toml");
const PRESS: &str = include_str!("../rubrics/press.toml");
const HOLIDAY: &str = include_str!("../rubrics/holiday.toml");

/// A loaded, validated rubric.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rubric {
    /// Which built-in this rubric was selected as, when a name selected it.
    /// Provenance only: two rubrics with the same words are the same judgement,
    /// however a run reached one (spec §8.3).
    builtin: Option<Builtin>,
    model: String,
    questions: Vec<Question>,
}

impl Rubric {
    /// The rubric a run judges by: the one selected, or the default built-in
    /// when nothing selected another — `--rubric` and `[run] rubric`.
    pub fn select(source: Option<&RubricSource>) -> Result<Rubric> {
        match source {
            None => Builtin::DEFAULT.rubric(),
            Some(RubricSource::Builtin(builtin)) => builtin.rubric(),
            Some(RubricSource::File(path)) => Self::load(path),
        }
    }

    /// Load the rubric file at `path`.
    pub fn load(path: &Path) -> Result<Rubric> {
        let text = fs::read_to_string(path).map_err(|error| {
            Error::fatal(format!(
                "cannot read the rubric file {}: {error}{}",
                path.display(),
                built_in_hint(path)
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

    /// Which built-in a name selected this rubric as; `None` for a file.
    pub fn builtin(&self) -> Option<Builtin> {
        self.builtin
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
    /// A `choice`'s criteria are an object keyed by option, in declaration
    /// order, because that is the shape its choice head reads; the loader
    /// guarantees the keys are `"1"`…`"5"` in order. Option order survives.
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
        // v2: the schema is one `choice` question, so no hash from the
        // six-question era can collide with one from this one.
        field("imagecull-rubric-v2");
        field(model);
        field(&self.questions_json().to_string());
        RubricHash(hex(digest.finalize()))
    }
}

/// A rubric compiled into the binary, named the way a photographer names it.
///
/// The three differ only in wording: the same `stars` id and the same 1–5
/// keys, so their reports share a shape and never share a hash.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Builtin {
    /// No assumed genre: what a run uses when nothing selects another.
    General,
    /// A photo editor culling a photojournalistic shoot.
    Press,
    /// A family album from a trip.
    Holiday,
}

impl Builtin {
    /// The built-in a run uses when nothing selects another.
    pub const DEFAULT: Builtin = Builtin::General;

    /// Every built-in, in the order help text and error messages list them.
    pub const ALL: [Builtin; 3] = [Builtin::General, Builtin::Press, Builtin::Holiday];

    /// The name a photographer types after `--rubric`.
    pub fn name(self) -> &'static str {
        self.entry().0
    }

    /// The built-in `name` names, matched exactly: three words, spelled as they
    /// are. `None` means "not a built-in", which leaves the value a path.
    pub fn from_name(name: &str) -> Option<Builtin> {
        Self::ALL.into_iter().find(|builtin| builtin.name() == name)
    }

    /// This rubric, parsed from the copy compiled into the binary.
    ///
    /// Parsed on every call rather than cached: it is a few kilobytes of TOML,
    /// and a cached copy would be one more thing to invalidate.
    pub fn rubric(self) -> Result<Rubric> {
        let mut rubric = Rubric::from_toml(self.toml()).map_err(|error| {
            Error::fatal(format!(
                "the built-in rubric `{}` is broken: {}",
                self.name(),
                error.message()
            ))
        })?;
        rubric.builtin = Some(self);
        Ok(rubric)
    }

    /// The TOML compiled in from this rubric's file.
    fn toml(self) -> &'static str {
        self.entry().1
    }

    /// This built-in's name and its TOML, in one place: adding a rubric is a
    /// variant, a file and a row here, and nothing that can drift apart.
    fn entry(self) -> (&'static str, &'static str) {
        match self {
            Builtin::General => ("general", GENERAL),
            Builtin::Press => ("press", PRESS),
            Builtin::Holiday => ("holiday", HOLIDAY),
        }
    }
}

impl fmt::Display for Builtin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// How a run was told which rubric to judge by: one setting, two forms.
///
/// A bare `general`, `press` or `holiday` names a built-in; anything else is a
/// path to a file. Selection never consults the filesystem, so a bare name is a
/// built-in however the working directory happens to be named, and `./holiday`
/// is unambiguously the file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RubricSource {
    /// A rubric compiled into the binary.
    Builtin(Builtin),
    /// A rubric file.
    File(PathBuf),
}

impl RubricSource {
    /// Read a value that may be a built-in's name or a path: `--rubric` and
    /// `[run] rubric` both hold one.
    ///
    /// The only failure is a selection that names nothing at all; a path that
    /// does not exist is [`Rubric::load`]'s failure, where the file is opened.
    pub fn parse(text: &str) -> std::result::Result<RubricSource, String> {
        if text.trim().is_empty() {
            return Err(format!(
                "no rubric named: use one of the built-in rubrics — {} — or a path to a rubric file",
                builtin_names()
            ));
        }
        Ok(match Builtin::from_name(text) {
            Some(builtin) => RubricSource::Builtin(builtin),
            None => RubricSource::File(PathBuf::from(text)),
        })
    }

    /// This selection with its path anchored to `base`, when it is a path: a
    /// config file's paths are relative to the file, its names are not.
    pub fn anchored(self, base: Option<&Path>) -> RubricSource {
        match (self, base) {
            (RubricSource::File(path), Some(base)) if path.is_relative() => {
                RubricSource::File(base.join(path))
            }
            (source, _) => source,
        }
    }
}

/// `general`, `press` and `holiday`, as a message spells them.
fn builtin_names() -> String {
    let mut names = String::new();
    for (position, builtin) in Builtin::ALL.iter().enumerate() {
        if position > 0 {
            names.push_str(if position + 1 == Builtin::ALL.len() {
                " and "
            } else {
                ", "
            });
        }
        names.push('`');
        names.push_str(builtin.name());
        names.push('`');
    }
    names
}

/// A path with no directory and no extension is a bare word, and a bare word
/// that could not be opened is more often a mistyped built-in name than a file,
/// so name the three the run does know.
fn built_in_hint(path: &Path) -> String {
    let bare = path.extension().is_none()
        && path
            .parent()
            .is_none_or(|parent| parent.as_os_str().is_empty());
    if bare {
        let names = builtin_names();
        format!(" — built-in rubrics are {names}; pass one by name, or a path to a rubric file")
    } else {
        String::new()
    }
}

/// A digest as this crate spells it: 64 lowercase hex characters.
///
/// The rubric hash, the SHA-256 of the bytes that were sent and the dedupe key
/// are all SHA-256 digests that end up in a CSV cell or a ledger line, so they
/// are spelled here, once.
pub(crate) fn hex(bytes: impl IntoIterator<Item = u8>) -> String {
    use fmt::Write as _;
    let mut out = String::new();
    for byte in bytes {
        let _ = write!(out, "{byte:02x}");
    }
    out
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
///
/// A cull asks exactly one: the `stars` choice, whose criteria are the five
/// rating levels. There is no question type field because there is no second
/// type to distinguish it from — the wire always says `choice`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Question {
    id: String,
    instructions: String,
    criteria: Vec<Criterion>,
}

impl Question {
    /// The question's key in the request's `questions` object and the answer's
    /// key in `answers`.
    pub fn id(&self) -> &str {
        &self.id
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

    /// This question as the request carries it: a `choice` renders its criteria
    /// as an object keyed by option, in declaration order (ADR 0001).
    fn to_json(&self) -> Value {
        let mut criteria = Map::new();
        for criterion in &self.criteria {
            criteria.insert(
                criterion.key.clone(),
                Value::String(criterion.description.clone()),
            );
        }
        let mut question = Map::new();
        question.insert("type".to_string(), Value::String("choice".to_string()));
        question.insert(
            "instructions".to_string(),
            Value::String(self.instructions.clone()),
        );
        question.insert("criteria".to_string(), Value::Object(criteria));
        Value::Object(question)
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
        if questions.len() != REQUIRED_IDS.len() {
            return Err(Error::fatal(format!(
                "a rubric asks exactly one question — `{}` — and this one asks: {}",
                REQUIRED_IDS[0],
                describe(&ids),
            )));
        }
        for id in REQUIRED_IDS {
            if !ids.contains(&id) {
                return Err(Error::fatal(format!(
                    "a rubric asks exactly one question — `{}` — and this one asks: {}",
                    REQUIRED_IDS[0],
                    describe(&ids),
                )));
            }
        }

        Ok(Rubric {
            builtin: None,
            model,
            questions,
        })
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
        let kind = self.kind.trim();
        if kind != "choice" {
            return Err(Error::fatal(format!(
                "rubric question `{id}` has type `{kind}`; a cull asks one `choice` question, the \
                 1-5 rating"
            )));
        }
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

        // The one id a cull reports, and the exact legend it must offer: the
        // CSV, the ledger and the v2 sidecar all depend on this shape.
        if id != RATING_ID {
            return Err(Error::fatal(format!(
                "rubric question `{id}` is not one a cull reports; a rubric asks: {}",
                REQUIRED_IDS[0]
            )));
        }
        let keys: Vec<&str> = criteria
            .iter()
            .map(|criterion| criterion.key.as_str())
            .collect();
        if keys != RATING_KEYS {
            return Err(Error::fatal(format!(
                "rubric question `{id}` must offer {}, in that order; it offers: {}",
                describe(&RATING_KEYS),
                describe(&keys),
            )));
        }
        if criteria.iter().any(|c| c.description.is_empty()) {
            return Err(Error::fatal(format!(
                "rubric question `{id}` must describe every level of its legend"
            )));
        }

        Ok(Question {
            id,
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
