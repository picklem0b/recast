//! Claim-language lint.
//!
//! Document 07 section 5 fixes the wording a report may use. A phrase that
//! claims more than the evidence class supports fails the lint; CI runs the
//! lint over every rendered report, so a stronger word cannot reach a user
//! without a failing test.

/// One forbidden phrase and where it occurred.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimViolation {
    pub phrase: String,
    pub offset: usize,
}

/// Forbidden multi-word phrases, matched without regard to case.
const PHRASES: &[&str] = &[
    "proven correct",
    "proves correctness",
    "guaranteed identical",
    "guaranteed correct",
    "proof of equivalence",
    "proves equivalence",
    "mathematically equivalent",
];

/// Find claim-language violations in `text`.
pub fn lint(text: &str) -> Vec<ClaimViolation> {
    let lower = text.to_ascii_lowercase();
    let mut violations = Vec::new();
    for phrase in PHRASES {
        for (offset, _) in lower.match_indices(phrase) {
            violations.push(ClaimViolation {
                phrase: (*phrase).to_string(),
                offset,
            });
        }
    }
    for (offset, _) in lower.match_indices("safe") {
        if is_standalone_safe(&lower, offset) {
            violations.push(ClaimViolation {
                phrase: "safe (without qualification)".to_string(),
                offset,
            });
        }
    }
    violations.sort_by_key(|violation| violation.offset);
    violations
}

fn is_standalone_safe(text: &str, offset: usize) -> bool {
    let bytes = text.as_bytes();
    if offset > 0 {
        let previous = bytes[offset - 1];
        if previous.is_ascii_alphanumeric() || previous == b'_' || previous == b'-' {
            // `unsafe`, `memory-safe`, `type-safe` and identifiers qualify it.
            return false;
        }
    }
    if let Some(next) = bytes.get(offset + 4) {
        if next.is_ascii_alphanumeric() || *next == b'_' || *next == b'-' {
            // `safety` and `safe_mode` are not the adjective.
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forbidden_phrases_are_found_without_regard_to_case() {
        let violations = lint("This is Proven Correct.");
        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].phrase, "proven correct");
    }

    #[test]
    fn an_unqualified_safe_is_forbidden() {
        assert_eq!(lint("The translation is safe.").len(), 1);
        assert!(lint("The module is memory-safe.").is_empty());
        assert!(lint("No unsafe block was generated.").is_empty());
        assert!(lint("The safety gate rejected it.").is_empty());
    }
}
