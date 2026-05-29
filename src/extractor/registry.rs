//! The set of languages CodeType knows how to drill, plus selection logic.
//!
//! Two ways a language gets chosen: the user names it explicitly with
//! `--lang` (→ [`by_name`]), or we infer it from what's in the repo
//! (→ [`detect_dominant`]). Both return a boxed [`LanguageConfig`] ready to
//! hand to an [`Extractor`](super::Extractor).

use std::path::{Path, PathBuf};

use super::LanguageConfig;
use super::rust::RustConfig;
use super::swift::SwiftConfig;
use super::typescript::TypeScriptConfig;

/// Every language CodeType supports, in priority order. The order is the
/// tie-break for [`detect_dominant`] when two languages claim the same number
/// of files, so keep the most "definitive" languages first.
pub fn available() -> Vec<Box<dyn LanguageConfig>> {
    vec![
        Box::new(SwiftConfig),
        Box::new(RustConfig),
        Box::new(TypeScriptConfig),
    ]
}

/// Comma-separated list of supported language names, for help/error text.
pub fn supported_names() -> String {
    available()
        .iter()
        .map(|c| c.language())
        .collect::<Vec<_>>()
        .join(", ")
}

/// Look up a config by its `language()` name (case-insensitive), accepting a
/// few common aliases (`ts`, `rs`). Returns `None` for an unknown name.
pub fn by_name(name: &str) -> Option<Box<dyn LanguageConfig>> {
    let want = name.trim().to_ascii_lowercase();
    available()
        .into_iter()
        .find(|c| c.language() == want || is_alias(c.language(), &want))
}

fn is_alias(language: &str, want: &str) -> bool {
    matches!((language, want), ("typescript", "ts") | ("rust", "rs"))
}

/// Pick the language that claims the most files among `files`. Ties break by
/// the order in [`available`]. Returns `None` if no file matches any
/// supported language.
pub fn detect_dominant(files: &[PathBuf]) -> Option<Box<dyn LanguageConfig>> {
    let configs = available();
    // Track the best (count, registry index). We only replace on a *strictly*
    // greater count, so the earliest config in `available` wins ties.
    let mut best: Option<(usize, usize)> = None;
    for (index, config) in configs.iter().enumerate() {
        let count = files
            .iter()
            .filter(|p| extension_matches(p, config.extensions()))
            .count();
        if count == 0 {
            continue;
        }
        if best.is_none_or(|(best_count, _)| count > best_count) {
            best = Some((count, index));
        }
    }
    best.map(|(_, index)| configs.into_iter().nth(index).unwrap())
}

fn extension_matches(path: &Path, extensions: &[&'static str]) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| extensions.contains(&e))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(names: &[&str]) -> Vec<PathBuf> {
        names.iter().map(PathBuf::from).collect()
    }

    #[test]
    fn by_name_resolves_canonical_names() {
        assert_eq!(by_name("swift").unwrap().language(), "swift");
        assert_eq!(by_name("rust").unwrap().language(), "rust");
        assert_eq!(by_name("typescript").unwrap().language(), "typescript");
    }

    #[test]
    fn by_name_is_case_insensitive_and_trims() {
        assert_eq!(by_name("  Rust ").unwrap().language(), "rust");
    }

    #[test]
    fn by_name_accepts_aliases() {
        assert_eq!(by_name("ts").unwrap().language(), "typescript");
        assert_eq!(by_name("rs").unwrap().language(), "rust");
    }

    #[test]
    fn by_name_unknown_is_none() {
        assert!(by_name("cobol").is_none());
    }

    #[test]
    fn detect_picks_the_majority_language() {
        let files = paths(&["a.ts", "b.ts", "c.ts", "d.rs", "e.swift"]);
        assert_eq!(detect_dominant(&files).unwrap().language(), "typescript");
    }

    #[test]
    fn detect_ignores_unknown_extensions() {
        let files = paths(&["a.py", "b.go", "c.rs", "README.md"]);
        assert_eq!(detect_dominant(&files).unwrap().language(), "rust");
    }

    #[test]
    fn detect_none_when_no_supported_files() {
        let files = paths(&["a.py", "b.go", "README.md"]);
        assert!(detect_dominant(&files).is_none());
    }

    #[test]
    fn detect_tie_breaks_by_registry_order() {
        // One file each of swift and rust. `available()` lists swift first,
        // so swift wins the tie.
        let files = paths(&["a.rs", "b.swift"]);
        assert_eq!(detect_dominant(&files).unwrap().language(), "swift");
    }
}
