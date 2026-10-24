//! Input value types shared by several features.

use crate::auth::FieldError;

/// Records a field's validation error, if any, so a request can report
/// every invalid field at once.
pub fn check<T>(
    errors: &mut Vec<FieldError>,
    field: &'static str,
    result: Result<T, String>,
) -> Option<T> {
    result
        .map_err(|message| errors.push(FieldError { field, message }))
        .ok()
}

/// A trimmed, non-empty display name with a feature-specific length limit
/// and no control characters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisplayName(String);

impl DisplayName {
    pub fn parse(input: &str, max_chars: usize) -> Result<Self, String> {
        let name = input.trim();
        if name.is_empty() {
            Err("must not be empty".to_owned())
        } else if name.chars().count() > max_chars {
            Err(format!("must be at most {max_chars} characters"))
        } else if name.chars().any(char::is_control) {
            Err("must not contain control characters".to_owned())
        } else {
            Ok(Self(name.to_owned()))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A three-letter currency code, normalized to uppercase.
///
/// Only the format is checked; codes are not matched against the ISO 4217
/// list (ADR-PORT-05).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CurrencyCode(String);

impl CurrencyCode {
    pub fn parse(input: &str) -> Result<Self, String> {
        let code = input.trim();
        if code.len() == 3 && code.chars().all(|c| c.is_ascii_alphabetic()) {
            Ok(Self(code.to_ascii_uppercase()))
        } else {
            Err("must be a three-letter currency code such as USD or THB".to_owned())
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_trimmed_and_bounded() {
        assert_eq!(
            DisplayName::parse("  Retirement  ", 100).unwrap().as_str(),
            "Retirement"
        );
        assert_eq!(
            DisplayName::parse(&"é".repeat(100), 100).map(|n| n.0.chars().count()),
            Ok(100)
        );
        assert!(DisplayName::parse("   ", 100).is_err());
        assert_eq!(
            DisplayName::parse(&"x".repeat(101), 100),
            Err("must be at most 100 characters".to_owned())
        );
        assert!(DisplayName::parse("line\nbreak", 100).is_err());
    }

    #[test]
    fn currency_codes_are_three_letters_uppercased() {
        assert_eq!(CurrencyCode::parse(" thb ").unwrap().as_str(), "THB");
        for bad in ["", "US", "USDX", "U$D", "123", "ÜSD"] {
            assert!(CurrencyCode::parse(bad).is_err(), "{bad:?}");
        }
    }
}
