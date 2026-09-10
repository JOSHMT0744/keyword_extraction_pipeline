use thiserror::Error;

/// A failure that prevented canonical text from being produced at all.
///
/// Distinct from [`crate::DocumentStatus`], which reports *why* a document that parsed
/// without error still yielded no useful keywords. The separation exists because a
/// scanned PDF is not an error — it parses correctly and simply has no text layer.
#[derive(Debug, Error)]
pub enum ExtractError {
    #[error("unsupported format: {0}")]
    UnsupportedFormat(String),

    #[error("document is encrypted and no password was supplied")]
    Encrypted,

    #[error("parse failure: {0}")]
    Parse(String),

    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),

    #[error("resource override could not be loaded: {0}")]
    Resource(String),
}
