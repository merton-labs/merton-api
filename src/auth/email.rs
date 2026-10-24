//! Email address validation and normalization.
//!
//! Only unquoted ASCII addresses with a dotted domain are accepted. The
//! normalized form (trimmed, ASCII-lowercased) is the identity used for
//! uniqueness and lookup; the trimmed original is kept for display.

const MAX_EMAIL_LEN: usize = 254;
const MAX_LOCAL_LEN: usize = 64;
const MAX_LABEL_LEN: usize = 63;
const LOCAL_SPECIALS: &str = "!#$%&'*+/=?^_`{|}~-";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Email {
    display: String,
    normalized: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("must be a valid email address")]
pub struct InvalidEmail;

impl Email {
    pub fn parse(input: &str) -> Result<Self, InvalidEmail> {
        let display = input.trim();
        if display.len() > MAX_EMAIL_LEN || !display.is_ascii() {
            return Err(InvalidEmail);
        }
        let (local, domain) = display.split_once('@').ok_or(InvalidEmail)?;
        if !is_valid_local_part(local) || !is_valid_domain(domain) {
            return Err(InvalidEmail);
        }

        Ok(Self {
            display: display.to_owned(),
            normalized: display.to_ascii_lowercase(),
        })
    }

    pub fn display(&self) -> &str {
        &self.display
    }

    pub fn normalized(&self) -> &str {
        &self.normalized
    }
}

fn is_valid_local_part(local: &str) -> bool {
    !local.is_empty()
        && local.len() <= MAX_LOCAL_LEN
        && !local.starts_with('.')
        && !local.ends_with('.')
        && !local.contains("..")
        && local
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || LOCAL_SPECIALS.contains(c))
}

fn is_valid_domain(domain: &str) -> bool {
    let labels: Vec<&str> = domain.split('.').collect();
    labels.len() >= 2
        && labels.iter().all(|label| {
            !label.is_empty()
                && label.len() <= MAX_LABEL_LEN
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_case_and_whitespace() {
        let email = Email::parse("  Alice.Smith+tag@Example.COM ").unwrap();

        assert_eq!(email.display(), "Alice.Smith+tag@Example.COM");
        assert_eq!(email.normalized(), "alice.smith+tag@example.com");
    }

    #[test]
    fn accepts_common_addresses() {
        for input in ["a@b.co", "first.last@sub.example.org", "x_y-z@my-host.io"] {
            assert!(Email::parse(input).is_ok(), "{input}");
        }
    }

    #[test]
    fn rejects_invalid_addresses() {
        let long_local = format!("{}@example.com", "a".repeat(65));
        for input in [
            "",
            "plainaddress",
            "@example.com",
            "user@",
            "user@localhost",
            "user@@example.com",
            "a@b@example.com",
            ".user@example.com",
            "user.@example.com",
            "us..er@example.com",
            "user@-example.com",
            "user@example..com",
            "us er@example.com",
            "\"quoted\"@example.com",
            "üser@example.com",
            long_local.as_str(),
        ] {
            assert_eq!(Email::parse(input), Err(InvalidEmail), "{input:?}");
        }
    }
}
