//! The unit of typing practice produced by an [`crate::extractor::Extractor`].

use std::ops::Range;
use std::path::PathBuf;

/// A self-contained unit of typing practice extracted from a source file.
///
/// The `text` field holds the dedented, comment-stripped string the user
/// types; the other fields are metadata pointing back to the source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Exercise {
    pub source_path: PathBuf,
    pub byte_range: Range<usize>,
    pub text: String,
    pub indent_unit: IndentUnit,
}

/// The dominant indent unit detected in the source file. Used by the typing
/// engine to compute the expected column on auto-indent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndentUnit {
    Spaces(u8),
    Tab,
}
