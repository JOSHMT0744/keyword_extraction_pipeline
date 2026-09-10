//! Extraction lanes.
//!
//! Lanes are **complements, not substitutes**: Lane 1 emits identifiers and technical
//! vocabulary, Lane 3 emits topical keyphrases. They do not compete for the same output
//! slots, so all of them run and the consumer filters on
//! [`crate::Kind`] rather than any lane being selected out.

pub mod definition;
pub mod shape;
pub mod topical;
