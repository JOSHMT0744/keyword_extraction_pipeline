//! Extraction stages.
//!
//! Stages are **complements, not substitutes**: Stage 1 emits identifiers and technical
//! vocabulary, Stage 3 emits topical keyphrases. They do not compete for the same output
//! slots, so all of them run and the consumer filters on
//! [`crate::Kind`] rather than any stage being selected out.

pub mod definition;
pub mod shape;
pub mod topical;
