//! A bearer token, wrapped so it cannot leak by accident.
//!
//! A token is the one value a run is given that must never reach a terminal, a
//! sidecar, the ledger, `--json-scores`, `--dry-run` output or an error
//! message. Rather than trust every call site to remember that, the value is a
//! [`Secret`]: `Debug` and `Display` print `<redacted>`, there is no `Deref`, no
//! `Serialize` and no conversion into a plain `String`, and [`Secret::as_str`]
//! is the one way out — so a review can grep for the few places that could leak
//! it instead of reading every `format!` in the crate.

use std::fmt;

/// A bearer token, compared and carried but never printed.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    /// Wrap a token.
    pub fn new(value: impl Into<String>) -> Secret {
        Secret(value.into())
    }

    /// The token itself: the only way to read it, and the one call a reviewer
    /// has to check.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// What `Debug` and `Display` print in place of the value.
const REDACTED: &str = "Secret(<redacted>)";

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(REDACTED)
    }
}

impl fmt::Display for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(REDACTED)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A struct that holds one, to prove a derived `Debug` cannot print it.
    #[derive(Debug)]
    struct Holder {
        token: Secret,
    }

    #[test]
    fn debug_and_display_redact_the_value() {
        let secret = Secret::new("hunter2-very-secret");
        for rendered in [
            format!("{secret:?}"),
            format!("{secret}"),
            format!("{secret:#?}"),
        ] {
            assert!(
                !rendered.contains("hunter2"),
                "a formatted secret printed its value: {rendered}"
            );
        }
    }

    #[test]
    fn a_struct_holding_one_redacts_it_in_pretty_debug() {
        let holder = Holder {
            token: Secret::new("hunter2-very-secret"),
        };
        let rendered = format!("{holder:#?}");
        assert_eq!(
            holder.token.as_str(),
            "hunter2-very-secret",
            "the value is still there; only the printing is withheld"
        );
        assert!(
            !rendered.contains("hunter2"),
            "a struct's derived Debug printed the secret: {rendered}"
        );
        assert!(
            rendered.contains("redacted"),
            "and says that something was withheld: {rendered}"
        );
    }

    #[test]
    fn equality_is_by_content() {
        assert_eq!(Secret::new("a"), Secret::new("a"));
        assert_ne!(Secret::new("a"), Secret::new("b"));
    }

    #[test]
    fn as_str_returns_the_value() {
        assert_eq!(Secret::new("xyz").as_str(), "xyz");
    }
}
