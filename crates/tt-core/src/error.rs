//! Domain-level errors.
//!
//! Storage and transport errors do not belong here -- those are the adapters'
//! problem. This is for values that are wrong on their own terms.

use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum DomainError {
    #[error("{field} is required")]
    Missing { field: &'static str },

    #[error("{field} has invalid value {value:?}")]
    Invalid { field: &'static str, value: String },
}

pub type Result<T> = std::result::Result<T, DomainError>;
