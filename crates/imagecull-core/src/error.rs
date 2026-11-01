//! The failure taxonomy from the spec (§10), as types.
//!
//! A run over a shoot produces one [`Error`] per row at most, and that error's
//! [`ErrorKind`] is the `status` column of the CSV. Run-level failures — bad
//! arguments, a missing directory, an unreachable server, a rejected rubric —
//! are [`Error`]s with no kind; they stop the run instead of being recorded.
//!
//! Nothing here formats for a terminal, asks whether stdout is a TTY, or exits
//! the process. Those belong to the CLI front end, so the same core can be
//! driven by the eventual GUI.

use std::error::Error as StdError;
use std::fmt;

/// Every way one file can fail once a run is going.
///
/// The string form ([`ErrorKind::as_str`]) is the `status` value written to the
/// CSV and the ledger, so it is part of the output contract: do not rename a
/// variant's string without treating it as a format change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ErrorKind {
    /// The file could not be opened or parsed.
    Unreadable,
    /// The file carries no embedded preview JPEG.
    NoPreview,
    /// The embedded preview exists but does not decode as a JPEG.
    PreviewBroken,
    /// The server rejected the question schema (HTTP 400): the rubric is wrong.
    Http400,
    /// The request did not fit the model's context (HTTP 413).
    Http413,
    /// The server rejected the request body (HTTP 422): a client bug.
    Http422,
    /// The server failed on its own side (HTTP 5xx): a transient hiccup.
    Http5xx,
    /// Nothing answered at the endpoint: connection refused, DNS failure.
    ServerUnreachable,
    /// The read timeout expired before the server answered.
    Timeout,
    /// The answer was missing, misnamed, or of the wrong type.
    BadResponse,
    /// The frame's existing sidecar cannot be read as text, or has no
    /// description this tool can safely write into, so it was left alone.
    XmpUnreadable,
    /// The frame's sidecar could not be written: a read-only mount, a full
    /// disk, a file that is not ours to replace.
    XmpWriteFailed,
    /// The frame's sidecar belongs to another application, which will rewrite it
    /// from its own history: the run refused to write into it.
    XmpForeign,
}

impl ErrorKind {
    /// Every kind, in spec order. Useful for exhaustive tests and doc tables.
    pub const ALL: [ErrorKind; 13] = [
        ErrorKind::Unreadable,
        ErrorKind::NoPreview,
        ErrorKind::PreviewBroken,
        ErrorKind::Http400,
        ErrorKind::Http413,
        ErrorKind::Http422,
        ErrorKind::Http5xx,
        ErrorKind::ServerUnreachable,
        ErrorKind::Timeout,
        ErrorKind::BadResponse,
        ErrorKind::XmpUnreadable,
        ErrorKind::XmpWriteFailed,
        ErrorKind::XmpForeign,
    ];

    /// The `status` string for this kind (never `"ok"`: that is the absence of
    /// an error, not an error).
    pub fn as_str(self) -> &'static str {
        match self {
            ErrorKind::Unreadable => "unreadable",
            ErrorKind::NoPreview => "no_preview",
            ErrorKind::PreviewBroken => "preview_broken",
            ErrorKind::Http400 => "http_400",
            ErrorKind::Http413 => "http_413",
            ErrorKind::Http422 => "http_422",
            ErrorKind::Http5xx => "http_5xx",
            ErrorKind::ServerUnreachable => "server_unreachable",
            ErrorKind::Timeout => "timeout",
            ErrorKind::BadResponse => "bad_response",
            ErrorKind::XmpUnreadable => "xmp_unreadable",
            ErrorKind::XmpWriteFailed => "xmp_write_failed",
            ErrorKind::XmpForeign => "xmp_foreign",
        }
    }

    /// The kind a `status` string names, if it names one (`"ok"` names no kind:
    /// that is the absence of an error). The ledger reads its records back with
    /// this, so a stored status and a live one cannot drift apart.
    pub fn from_status(status: &str) -> Option<ErrorKind> {
        ErrorKind::ALL
            .into_iter()
            .find(|kind| kind.as_str() == status)
    }

    /// The kind for an HTTP status this taxonomy names, if it names it.
    ///
    /// The whole 5xx range is one kind because the retry rule treats it as one:
    /// a server that failed is worth another attempt whatever it called itself
    /// (spec §10).
    pub fn from_status_code(status: u16) -> Option<ErrorKind> {
        match status {
            400 => Some(ErrorKind::Http400),
            413 => Some(ErrorKind::Http413),
            422 => Some(ErrorKind::Http422),
            500..=599 => Some(ErrorKind::Http5xx),
            _ => None,
        }
    }

    /// What the run should do about this kind (spec §10, "action" column).
    pub fn recovery(self) -> Recovery {
        match self {
            ErrorKind::Unreadable | ErrorKind::NoPreview | ErrorKind::PreviewBroken => {
                Recovery::Continue
            }
            ErrorKind::Http413 | ErrorKind::BadResponse => Recovery::Continue,
            ErrorKind::XmpUnreadable => Recovery::Continue,
            // Every remaining frame would fail the same way, and each attempt
            // has already been paid for: stop, do not collect 400 failures.
            ErrorKind::XmpWriteFailed => Recovery::AbortRun,
            // One darktable sidecar is not a trend: the photographer may well
            // want the rest of the folder rated.
            ErrorKind::XmpForeign => Recovery::Continue,
            ErrorKind::Http400 | ErrorKind::Http422 => Recovery::AbortRun,
            ErrorKind::Http5xx => Recovery::RetryThenRecord,
            ErrorKind::ServerUnreachable => Recovery::RetryThenAbort,
            ErrorKind::Timeout => Recovery::RetryThenRecord,
        }
    }
}

impl fmt::Display for ErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Whether a failure is worth another attempt, and whether it ends the run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Recovery {
    /// Record the row, keep going. Retrying would fail identically.
    Continue,
    /// Retry with backoff, then record the row and keep going.
    RetryThenRecord,
    /// Retry with backoff, then stop the run.
    RetryThenAbort,
    /// Stop the run now; a retry cannot change the answer.
    AbortRun,
}

impl Recovery {
    /// Whether this recovery ends the run rather than recording one bad row.
    pub fn aborts_run(self) -> bool {
        matches!(self, Recovery::RetryThenAbort | Recovery::AbortRun)
    }

    /// Whether this recovery is worth another attempt: the transient kinds
    /// only, never a 4xx (spec §10).
    pub fn is_retried(self) -> bool {
        matches!(self, Recovery::RetryThenRecord | Recovery::RetryThenAbort)
    }
}

/// Something went wrong, with the taxonomy kind when the failure belongs to a
/// single file.
#[derive(Debug)]
pub struct Error {
    kind: Option<ErrorKind>,
    message: String,
    source: Option<Box<dyn StdError + Send + Sync + 'static>>,
    http_status: Option<u16>,
}

impl Error {
    /// A file-scoped failure: it becomes a recorded row with `kind` as its
    /// status.
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Error {
            kind: Some(kind),
            message: message.into(),
            source: None,
            http_status: None,
        }
    }

    /// A run-scoped failure: bad arguments, a missing directory, a rejected
    /// rubric. Not a row; the run stops (exit code 2).
    pub fn fatal(message: impl Into<String>) -> Self {
        Error {
            kind: None,
            message: message.into(),
            source: None,
            http_status: None,
        }
    }

    /// Attach the underlying error for `--verbose` and backtraces.
    pub fn with_source(mut self, source: impl StdError + Send + Sync + 'static) -> Self {
        self.source = Some(Box::new(source));
        self
    }

    /// Remember the HTTP status, for callers that explain failures (401 is
    /// "the server wants a key", not "that is not oMLX").
    pub fn with_http_status(mut self, status: u16) -> Self {
        self.http_status = Some(status);
        self
    }

    /// The HTTP status this failure came from, when it came from HTTP.
    pub fn http_status(&self) -> Option<u16> {
        self.http_status
    }

    /// Note how many attempts a retried failure cost, so the row's `error`
    /// cell says why it is not a one-off. A failure that was never retried
    /// stays as it is: there is nothing to explain.
    pub fn with_attempts(mut self, attempts: usize) -> Self {
        if attempts > 1 {
            self.message = format!("{} — {attempts} attempts", self.message);
        }
        self
    }

    /// The taxonomy kind, or `None` when this is a run-level failure.
    pub fn kind(&self) -> Option<ErrorKind> {
        self.kind
    }

    /// The human-readable message, without the kind prefix.
    pub fn message(&self) -> &str {
        &self.message
    }

    /// Whether this failure stops the run instead of recording a row.
    pub fn is_fatal(&self) -> bool {
        self.kind.is_none()
    }

    /// How the run should react to this failure.
    pub fn recovery(&self) -> Recovery {
        match self.kind {
            Some(kind) => kind.recovery(),
            None => Recovery::AbortRun,
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.kind {
            Some(kind) => write!(f, "{}: {}", kind.as_str(), self.message),
            None => f.write_str(&self.message),
        }
    }
}

impl StdError for Error {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        self.source
            .as_ref()
            .map(|boxed| boxed.as_ref() as &(dyn StdError + 'static))
    }
}

/// Every `Result` in the core is an [`Error`].
pub type Result<T> = std::result::Result<T, Error>;

/// How the process should exit (spec §10).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ExitCode {
    /// Every row was scored.
    Ok,
    /// The run finished but left failed rows behind.
    FailedRows,
    /// Nothing could be done: bad arguments, no server, rejected rubric.
    Fatal,
    /// SIGINT; the ledger is intact and `--resume` picks it up.
    Interrupted,
}

impl ExitCode {
    /// The number the shell sees.
    pub fn code(self) -> u8 {
        match self {
            ExitCode::Ok => 0,
            ExitCode::FailedRows => 1,
            ExitCode::Fatal => 2,
            ExitCode::Interrupted => 3,
        }
    }

    /// The exit code for a run-stopping error.
    pub fn for_error(error: &Error) -> ExitCode {
        if error.is_fatal() {
            ExitCode::Fatal
        } else {
            // A row-scoped error only reaches this path when its recovery
            // aborts the run, which the spec still classifies as fatal.
            ExitCode::Fatal
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_strings_are_the_spec_vocabulary() {
        let strings: Vec<&str> = ErrorKind::ALL.iter().map(|kind| kind.as_str()).collect();
        assert_eq!(
            strings,
            [
                "unreadable",
                "no_preview",
                "preview_broken",
                "http_400",
                "http_413",
                "http_422",
                "http_5xx",
                "server_unreachable",
                "timeout",
                "bad_response",
                // The sidecar's own failures: the rating's only home is the
                // document beside the frame, so its refusals are frame statuses
                // like any other.
                "xmp_unreadable",
                "xmp_write_failed",
                "xmp_foreign",
            ]
        );
        assert!(!strings.contains(&"ok"));
    }

    #[test]
    fn recovery_matches_the_action_column() {
        assert_eq!(ErrorKind::Http400.recovery(), Recovery::AbortRun);
        assert_eq!(ErrorKind::Http422.recovery(), Recovery::AbortRun);
        assert_eq!(ErrorKind::Http413.recovery(), Recovery::Continue);
        assert_eq!(ErrorKind::Http5xx.recovery(), Recovery::RetryThenRecord);
        assert_eq!(ErrorKind::Unreadable.recovery(), Recovery::Continue);
        assert_eq!(ErrorKind::NoPreview.recovery(), Recovery::Continue);
        assert_eq!(ErrorKind::PreviewBroken.recovery(), Recovery::Continue);
        assert_eq!(ErrorKind::BadResponse.recovery(), Recovery::Continue);
        assert_eq!(ErrorKind::Timeout.recovery(), Recovery::RetryThenRecord);
        assert_eq!(
            ErrorKind::ServerUnreachable.recovery(),
            Recovery::RetryThenAbort
        );
        assert!(ErrorKind::Http400.recovery().aborts_run());
        assert!(!ErrorKind::Timeout.recovery().aborts_run());
    }

    #[test]
    fn only_the_named_statuses_become_kinds() {
        assert_eq!(ErrorKind::from_status_code(400), Some(ErrorKind::Http400));
        assert_eq!(ErrorKind::from_status_code(413), Some(ErrorKind::Http413));
        assert_eq!(ErrorKind::from_status_code(422), Some(ErrorKind::Http422));
        assert_eq!(ErrorKind::from_status_code(500), Some(ErrorKind::Http5xx));
        assert_eq!(ErrorKind::from_status_code(503), Some(ErrorKind::Http5xx));
        assert_eq!(ErrorKind::from_status_code(599), Some(ErrorKind::Http5xx));
        assert_eq!(ErrorKind::from_status_code(404), None);
    }

    #[test]
    fn only_the_transient_kinds_are_worth_another_attempt() {
        for kind in ErrorKind::ALL {
            assert_eq!(
                kind.recovery().is_retried(),
                matches!(
                    kind,
                    ErrorKind::ServerUnreachable | ErrorKind::Timeout | ErrorKind::Http5xx
                ),
                "{kind}"
            );
        }
    }

    #[test]
    fn a_retried_failure_names_how_many_attempts_it_cost() {
        let error =
            Error::new(ErrorKind::Http5xx, "POST /v1/systemone returned HTTP 503").with_attempts(4);
        assert_eq!(
            error.message(),
            "POST /v1/systemone returned HTTP 503 — 4 attempts"
        );

        let once =
            Error::new(ErrorKind::Http400, "POST /v1/systemone returned HTTP 400").with_attempts(1);
        assert_eq!(
            once.message(),
            "POST /v1/systemone returned HTTP 400",
            "a failure that was never retried says nothing about attempts"
        );
    }

    #[test]
    fn fatal_errors_carry_no_kind_and_abort() {
        let fatal = Error::fatal("input directory does not exist");
        assert!(fatal.is_fatal());
        assert_eq!(fatal.kind(), None);
        assert_eq!(fatal.recovery(), Recovery::AbortRun);
        assert_eq!(ExitCode::for_error(&fatal), ExitCode::Fatal);
        assert_eq!(fatal.to_string(), "input directory does not exist");
    }

    #[test]
    fn row_errors_display_their_status() {
        let row = Error::new(ErrorKind::NoPreview, "DSC_0001.NEF has no embedded JPEG");
        assert!(!row.is_fatal());
        assert_eq!(
            row.to_string(),
            "no_preview: DSC_0001.NEF has no embedded JPEG"
        );
    }

    #[test]
    fn a_status_carrying_error_remembers_its_code() {
        let error = Error::new(ErrorKind::BadResponse, "GET /v1/models returned HTTP 401")
            .with_http_status(401);
        assert_eq!(error.http_status(), Some(401));
        assert_eq!(Error::fatal("nope").http_status(), None);
    }

    #[test]
    fn exit_codes_are_the_spec_numbers() {
        assert_eq!(ExitCode::Ok.code(), 0);
        assert_eq!(ExitCode::FailedRows.code(), 1);
        assert_eq!(ExitCode::Fatal.code(), 2);
        assert_eq!(ExitCode::Interrupted.code(), 3);
    }
}
