//! The scripted [`Decider`]: the seam every scoring test drives the pipeline
//! through.
//!
//! It records what it was asked — the alias, the state, the exact preview bytes
//! and the questions, in order — and answers from a script, so a test asserts on
//! what the model would have seen and on the run that follows from a chosen
//! answer, with no server involved.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::sync::Arc;

use imagecull_core::error::{Error, Result};
use imagecull_core::scoring::{Cancel, Decider, Request, Response, State};
use serde_json::{Value, json};

/// One request as the scripted decider saw it, copied out of the borrow.
#[derive(Debug, Clone)]
pub struct Seen {
    pub model: String,
    pub state: State,
    pub width: u32,
    pub height: u32,
    pub jpeg: Vec<u8>,
    pub question_ids: Vec<String>,
}

/// A decider that records what it was asked and answers from a script.
pub struct Scripted {
    answers: RefCell<VecDeque<Result<Value>>>,
    seen: RefCell<Vec<Seen>>,
    in_flight: Cell<usize>,
    overlapped: Cell<bool>,
    /// What the preflight answers, when a test gives it something to say.
    preflight: RefCell<Option<Error>>,
    /// A cancel flag to pull, and after how many requests.
    cancel_after: Option<(usize, Arc<Cancel>)>,
    /// The latency this decider reports for every request, as if measured on
    /// this side of a socket.
    pub latency_ms: u64,
}

impl Scripted {
    /// A decider that answers the requests, in order, from `answers`.
    pub fn answering(answers: Vec<Result<Value>>) -> Scripted {
        Scripted {
            answers: RefCell::new(answers.into()),
            seen: RefCell::new(Vec::new()),
            in_flight: Cell::new(0),
            overlapped: Cell::new(false),
            preflight: RefCell::new(None),
            cancel_after: None,
            latency_ms: 812,
        }
    }

    /// A decider that answers every frame with the same readable document.
    pub fn always_reading(frames: usize) -> Scripted {
        Self::answering((0..frames).map(|_| Ok(document(6.4237, "4"))).collect())
    }

    /// Refuse the preflight with `error`, the way a real decider refuses a
    /// server that is not answering or does not serve the alias. The refusal is
    /// handed over once, because that is how often a run asks.
    pub fn refusing_preflight(error: Error) -> Scripted {
        let decider = Scripted::answering(Vec::new());
        *decider.preflight.borrow_mut() = Some(error);
        decider
    }

    /// Pull `cancel` after this many requests, from inside the decider: the
    /// lever a photographer's Ctrl-C pulls, at the point a run checks it.
    pub fn cancelling_after(self, requests: usize, cancel: Arc<Cancel>) -> Scripted {
        Scripted {
            cancel_after: Some((requests, cancel)),
            ..self
        }
    }

    /// Answer with a client-measured latency of `ms` instead of the default.
    pub fn taking(self, ms: u64) -> Scripted {
        Scripted {
            latency_ms: ms,
            ..self
        }
    }

    /// Every request the pipeline made, in order.
    pub fn seen(&self) -> Vec<Seen> {
        self.seen.borrow().clone()
    }

    /// Whether a second request was ever in flight while the first was
    /// unanswered: the one thing a sequential run must never do.
    pub fn overlapped(&self) -> bool {
        self.overlapped.get()
    }
}

impl Decider for Scripted {
    fn preflight(&self) -> Result<()> {
        match self.preflight.borrow_mut().take() {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    fn decide(&self, request: &Request<'_>) -> Result<Response> {
        self.in_flight.set(self.in_flight.get() + 1);
        if self.in_flight.get() > 1 {
            self.overlapped.set(true);
        }
        self.seen.borrow_mut().push(Seen {
            model: request.model.to_string(),
            state: request.state.clone(),
            width: request.preview.width,
            height: request.preview.height,
            jpeg: request.preview.jpeg.clone(),
            question_ids: request
                .rubric
                .questions()
                .iter()
                .map(|question| question.id().to_string())
                .collect(),
        });
        if let Some((after, cancel)) = &self.cancel_after
            && self.seen.borrow().len() == *after
        {
            cancel.cancel();
        }
        let answer = self
            .answers
            .borrow_mut()
            .pop_front()
            .expect("the script must have an answer for every request");
        self.in_flight.set(self.in_flight.get() - 1);
        answer.map(|document| Response {
            document,
            client_latency_ms: self.latency_ms,
        })
    }
}

/// A readable six-answer document with a chosen `overall` and star key.
pub fn document(overall: f64, stars: &str) -> Value {
    json!({
        "model": "clef-flash-oQ4e",
        "answers": {
            "overall": {"type": "score", "score": overall, "confidence": 0.41},
            "technical": {"type": "score", "score": 7.8125, "confidence": 0.55},
            "composition": {"type": "score", "score": 5.3398, "confidence": 0.33},
            "impact": {"type": "score", "score": 4.9601, "confidence": 0.62},
            "stars": {"type": "choice", "choice": stars, "confidence": 0.63},
            "verdict": {"type": "choice", "choice": "like", "confidence": 0.88}
        },
        "usage": {"input_tokens": 1327}
    })
}
