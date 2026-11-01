//! Version strings as oMLX reports them (`0.7.1.dev1`), comparable enough to
//! answer one question: is this server at least the release that serves
//! decision models?
//!
//! This is not a full PEP 440 implementation. It parses numeric dot segments
//! plus one trailing pre-release tag, which covers what oMLX publishes, and it
//! says "I cannot read this" instead of guessing, so `doctor` can fall back to
//! probing capabilities.

use std::cmp::Ordering;
use std::fmt;

/// A parsed server version such as `0.7.1.dev1`.
///
/// Equality follows comparison, so `0.7` equals `0.7.0`.
#[derive(Debug, Clone)]
pub struct Version {
    numbers: Vec<u64>,
    pre: Option<PreRelease>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PreRelease {
    label: String,
    number: Option<u64>,
}

impl Version {
    /// Parse a version string, or return `None` when it is not one.
    ///
    /// Build metadata (`+abc`) is discarded. A leading `v` is accepted.
    pub fn parse(raw: &str) -> Option<Version> {
        let trimmed = raw.trim();
        let trimmed = trimmed.strip_prefix('v').unwrap_or(trimmed);
        let without_build = trimmed.split('+').next().unwrap_or(trimmed);
        if without_build.is_empty() {
            return None;
        }

        let mut numbers = Vec::new();
        let mut pre = None;
        let segments: Vec<&str> = without_build.split('.').collect();
        for (index, segment) in segments.iter().enumerate() {
            if let Ok(number) = segment.parse::<u64>() {
                if pre.is_some() {
                    // Numeric segment after a pre-release tag: not a version
                    // shape we understand.
                    return None;
                }
                numbers.push(number);
            } else {
                // A pre-release tag must be the last segment: `0.7.1.dev1`
                // parses, `0.x.1` (which would hide a segment) does not.
                if index == 0 || index + 1 != segments.len() {
                    return None;
                }
                pre = Some(parse_pre_release(segment)?);
            }
        }

        if numbers.is_empty() {
            return None;
        }
        Some(Version { numbers, pre })
    }

    /// The numeric segments, e.g. `[0, 7, 1]`.
    pub fn numbers(&self) -> &[u64] {
        &self.numbers
    }

    /// The pre-release tag, if this is a pre-release.
    pub fn is_pre_release(&self) -> bool {
        self.pre.is_some()
    }
}

/// Split one pre-release segment such as `dev1`, `rc2` or `b` into a label and
/// an optional number.
fn parse_pre_release(segment: &str) -> Option<PreRelease> {
    let label_end = segment
        .find(|character: char| character.is_ascii_digit())
        .unwrap_or(segment.len());
    let (label, digits) = segment.split_at(label_end);
    if label.is_empty()
        || !label
            .chars()
            .all(|character| character.is_ascii_alphabetic())
    {
        return None;
    }
    let number = if digits.is_empty() {
        None
    } else {
        Some(digits.parse::<u64>().ok()?)
    };
    Some(PreRelease {
        label: label.to_ascii_lowercase(),
        number,
    })
}

impl PartialEq for Version {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for Version {}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> Ordering {
        let width = self.numbers.len().max(other.numbers.len());
        for index in 0..width {
            let left = self.numbers.get(index).copied().unwrap_or(0);
            let right = other.numbers.get(index).copied().unwrap_or(0);
            match left.cmp(&right) {
                Ordering::Equal => {}
                ordering => return ordering,
            }
        }
        match (&self.pre, &other.pre) {
            // A release outranks any pre-release of the same numbers.
            (None, None) => Ordering::Equal,
            (None, Some(_)) => Ordering::Greater,
            (Some(_), None) => Ordering::Less,
            (Some(left), Some(right)) => left
                .label
                .cmp(&right.label)
                .then_with(|| left.number.cmp(&right.number)),
        }
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let numbers: Vec<String> = self.numbers.iter().map(u64::to_string).collect();
        f.write_str(&numbers.join("."))?;
        if let Some(pre) = &self.pre {
            write!(f, ".{}", pre.label)?;
            if let Some(number) = pre.number {
                write!(f, "{number}")?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn version(raw: &str) -> Version {
        Version::parse(raw).unwrap_or_else(|| panic!("{raw} should parse"))
    }

    #[test]
    fn parses_the_shapes_omlx_publishes() {
        let parsed = version("0.7.1.dev1");
        assert_eq!(parsed.numbers(), &[0, 7, 1]);
        assert!(parsed.is_pre_release());
        assert_eq!(parsed.to_string(), "0.7.1.dev1");

        assert_eq!(version("v0.7.1").numbers(), &[0, 7, 1]);
        assert_eq!(version("0.7.1+abc123").to_string(), "0.7.1");
        assert_eq!(version("1").to_string(), "1");
        assert!(Version::parse("main").is_none());
        assert!(Version::parse("").is_none());
        assert!(Version::parse("0.x.1").is_none());
    }

    #[test]
    fn release_outranks_its_pre_releases() {
        assert!(version("0.7.1") > version("0.7.1.dev1"));
        assert!(version("0.7.1.dev2") > version("0.7.1.dev1"));
        assert!(version("0.7.1.dev1") > version("0.7.1.dev"));
        assert!(version("0.7.1.dev") > version("0.7.0"));
    }

    #[test]
    fn the_decision_model_release_is_the_boundary() {
        let floor = version("0.7.1.dev1");
        assert!(version("0.7.0") < floor);
        assert!(version("0.7.0.post9") < floor);
        assert_eq!(version("0.7.1.dev1"), floor);
        assert!(version("0.7.1.dev1+local") == floor);
        assert!(version("0.8.0") > floor);
        assert!(version("1.0") > floor);
    }

    #[test]
    fn missing_segments_count_as_zero() {
        assert_eq!(version("0.7"), version("0.7.0"));
        assert!(version("0.10") > version("0.9.9"));
    }
}
