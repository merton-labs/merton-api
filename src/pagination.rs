//! Bounded offset pagination (ADR-API-04).

use crate::auth::FieldError;

pub const DEFAULT_PAGE_SIZE: u64 = 50;
pub const MAX_PAGE_SIZE: u64 = 200;
/// Largest offset the database can bind (SQL offsets are signed 64-bit).
pub const MAX_OFFSET: u64 = i64::MAX as u64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Page {
    pub limit: u64,
    pub offset: u64,
}

impl Page {
    pub fn new(limit: Option<u64>, offset: Option<u64>) -> Result<Self, FieldError> {
        let limit = limit.unwrap_or(DEFAULT_PAGE_SIZE);
        if !(1..=MAX_PAGE_SIZE).contains(&limit) {
            return Err(FieldError {
                field: "limit",
                message: format!("must be between 1 and {MAX_PAGE_SIZE}"),
            });
        }
        let offset = offset.unwrap_or(0);
        if offset > MAX_OFFSET {
            return Err(FieldError {
                field: "offset",
                message: format!("must be at most {MAX_OFFSET}"),
            });
        }
        Ok(Self { limit, offset })
    }
}

/// One page of results plus the total number of matches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paged<T> {
    pub items: Vec<T>,
    pub total: u64,
    pub page: Page,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_defaults_and_bounds() {
        assert_eq!(
            Page::new(None, None).unwrap(),
            Page {
                limit: 50,
                offset: 0
            }
        );
        assert_eq!(Page::new(Some(200), Some(7)).unwrap().offset, 7);
        assert!(Page::new(Some(0), None).is_err());
        assert!(Page::new(Some(201), None).is_err());
        assert!(Page::new(None, Some(MAX_OFFSET)).is_ok());
        assert!(Page::new(None, Some(MAX_OFFSET + 1)).is_err());
    }
}
