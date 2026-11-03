//! The scripted [`Decider`]: the seam every scoring test drives the pipeline
//! through.
//!
//! It records what it was asked — the alias, the state, the exact preview bytes
//! and the questions, in order — and answers from a script, so a test asserts on
//! what the model would have seen and on the run that follows from a chosen
//! answer, with no server involved.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use imagecull_core::error::Result;
use imagecull_core::keeper::Comparison;
use imagecull_core::preprocess::Encoded;
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

/// One comparative request as the scripted decider saw it: the frames it was
/// shown, in the order the criteria named them.
#[derive(Debug, Clone)]
pub struct SeenComparison {
    pub model: String,
    pub state: State,
    pub previews: Vec<Encoded>,
}

impl SeenComparison {
    /// The frames' names, taken from the state the group was judged in.
    pub fn group(&self) -> &str {
        &self.state.filename
    }
}

/// A decider that records what it was asked and answers from a script.
pub struct Scripted {
    answers: RefCell<VecDeque<Result<Value>>>,
    comparisons: RefCell<VecDeque<Result<Value>>>,
    seen: RefCell<Vec<Seen>>,
    seen_comparisons: RefCell<Vec<SeenComparison>>,
    in_flight: Cell<usize>,
    overlapped: Cell<bool>,
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
            comparisons: RefCell::new(VecDeque::new()),
            seen: RefCell::new(Vec::new()),
            seen_comparisons: RefCell::new(Vec::new()),
            in_flight: Cell::new(0),
            overlapped: Cell::new(false),
            cancel_after: None,
            latency_ms: 812,
        }
    }

    /// A decider that answers every frame with the same readable document.
    pub fn always_reading(frames: usize) -> Scripted {
        Self::answering((0..frames).map(|_| Ok(document("4"))).collect())
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

    /// Answer the comparative requests, in order, from `answers`: the script a
    /// group of frames is judged against, with no server involved.
    pub fn comparing(self, answers: Vec<Result<Value>>) -> Scripted {
        Scripted {
            comparisons: RefCell::new(answers.into()),
            ..self
        }
    }

    /// Every request the pipeline made, in order.
    pub fn seen(&self) -> Vec<Seen> {
        self.seen.borrow().clone()
    }

    /// Every comparative request the pipeline made, in order.
    pub fn compared(&self) -> Vec<SeenComparison> {
        self.seen_comparisons.borrow().clone()
    }

    /// Whether a second request was ever in flight while the first was
    /// unanswered: the one thing a sequential run must never do.
    pub fn overlapped(&self) -> bool {
        self.overlapped.get()
    }
}

impl Decider for Scripted {
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

    fn compare(&self, comparison: &Comparison) -> Result<Response> {
        self.seen_comparisons.borrow_mut().push(SeenComparison {
            model: comparison.model.clone(),
            state: comparison.state.clone(),
            previews: comparison.previews.clone(),
        });
        let answer = self
            .comparisons
            .borrow_mut()
            .pop_front()
            .expect("the script must have an answer for every comparative request");
        answer.map(|document| Response {
            document,
            client_latency_ms: self.latency_ms,
        })
    }
}

/// A [`Scripted`] behind a lock: the concurrent cull needs a `Sync` decider, and
/// the script itself is single-threaded.
///
/// Every request is serialised by the lock, so this wrapper can never report two
/// requests in flight; a test that wants to observe overlap brings its own
/// decider with atomics instead.
pub struct Guarded(Mutex<Scripted>);

impl Guarded {
    /// Wrap a script for a pass that runs groups on several threads.
    pub fn new(scripted: Scripted) -> Guarded {
        Guarded(Mutex::new(scripted))
    }

    /// The script, for the assertions a test makes after the pass.
    pub fn get(&self) -> std::sync::MutexGuard<'_, Scripted> {
        self.0.lock().expect("a decider lock")
    }
}

impl Decider for Guarded {
    fn decide(&self, request: &Request<'_>) -> Result<Response> {
        self.get().decide(request)
    }

    fn compare(&self, comparison: &Comparison) -> Result<Response> {
        self.get().compare(comparison)
    }
}

/// A readable answer document with a chosen star key.
pub fn document(stars: &str) -> Value {
    json!({
        "model": "clef-flash-oQ4e",
        "answers": {
            "stars": {"type": "choice", "choice": stars, "confidence": 0.63}
        },
        "usage": {"input_tokens": 1327}
    })
}

/// A readable comparative answer naming the frame at `position`, counting from
/// 1 in the order the request showed them.
pub fn keeper(position: &str, confidence: f64) -> Value {
    json!({
        "model": "clef-flash-oQ4e",
        "answers": {
            "keeper": {"type": "choice", "choice": position, "confidence": confidence}
        },
        "usage": {"input_tokens": 557}
    })
}
