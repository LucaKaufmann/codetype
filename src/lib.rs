//! CodeType — terminal typing trainer for developers.
//!
//! CodeType turns the source code of a local Git repository into typing
//! exercises. It supports multiple languages (Swift, Rust, TypeScript),
//! auto-detecting the dominant one, and offers two typing modes: lenient
//! (the default, Monkeytype-style — wrong keys advance and can be fixed with
//! backspace) and strict (the expected key must be typed to advance).
//!
//! Module layout:
//! - [`app`] — the TUI application loop and screen state.
//! - [`command`] — parsing for the `:` command palette.
//! - [`engine`] — the per-keystroke typing state machine that scores a run.
//! - [`exercise`] — the [`exercise::Exercise`] unit of practice and its metadata.
//! - [`extractor`] — a generic tree-sitter extraction driver plus per-language
//!   `LanguageConfig` implementations (`swift`, `rust`, `typescript`) wired up
//!   through `registry`.
//! - [`scanner`] — enumerates candidate source files in the repository.
//! - [`scores`] — persistent, checked-in `CODETYPE.md` leaderboard.
//! - [`stats`] — accuracy and speed metrics for a completed run.
//! - [`tour`] — round-robin selection of which exercise to present next.

pub mod app;
pub mod command;
pub mod engine;
pub mod exercise;
pub mod extractor;
pub mod scanner;
pub mod scores;
pub mod stats;
pub mod tour;
