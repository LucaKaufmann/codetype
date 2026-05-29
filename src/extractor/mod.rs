//! Turning source files into [`Exercise`]s.
//!
//! The extraction *algorithm* is identical for every language: walk the
//! tree-sitter AST, keep declarations whose size falls in a sensible band,
//! recurse into the ones that are too big, strip comments, and dedent. The
//! only things that vary per language are the grammar and which node kinds
//! count as "a declaration" or "a comment". We capture that varying part in
//! the [`LanguageConfig`] trait and keep the invariant algorithm in the
//! single [`Extractor`] driver. Adding a language is then just a new
//! `LanguageConfig` impl — see `swift.rs` for the reference one.

use std::ops::Range;
use std::path::Path;

use anyhow::{Context, anyhow};
use tree_sitter::{Node, Parser};

use crate::exercise::{Exercise, IndentUnit};

pub mod registry;
pub mod rust;
pub mod swift;
pub mod typescript;

/// The per-language vocabulary the [`Extractor`] needs. This is the *only*
/// surface that differs between languages; everything else is shared.
///
/// It's deliberately a "description" trait (predicates + the grammar) rather
/// than a "behavior" trait — the behavior lives in [`Extractor`]. It's also
/// dyn-compatible on purpose: language selection happens at runtime based on
/// what's in the repo, so we store the chosen config as a `Box<dyn
/// LanguageConfig>` rather than a generic type parameter.
pub trait LanguageConfig {
    /// Human-readable label shown in the UI (e.g. `"swift"`).
    fn language(&self) -> &'static str;

    /// File extensions this language claims (e.g. `&["swift"]`). Lowercase,
    /// without the leading dot.
    fn extensions(&self) -> &[&'static str];

    /// The tree-sitter grammar to parse this language with.
    fn tree_sitter_language(&self) -> tree_sitter::Language;

    /// Whether a node of this `kind` is a declaration we'd consider turning
    /// into an Exercise (function, type, etc.).
    fn is_declaration(&self, kind: &str) -> bool;

    /// Whether a node of this `kind` is a comment to strip from Exercises.
    fn is_comment(&self, kind: &str) -> bool;
}

/// Size bounds for an extractable Exercise. Declarations are kept iff their
/// line count and byte length both fall within these bounds.
///
/// Defaults are 4–25 lines, 100–600 bytes. The struct is exposed so tests
/// (and any future "give me everything" mode) can override.
#[derive(Debug, Clone, Copy)]
pub struct LengthFilter {
    pub min_lines: usize,
    pub max_lines: usize,
    pub min_bytes: usize,
    pub max_bytes: usize,
}

impl Default for LengthFilter {
    fn default() -> Self {
        Self {
            min_lines: 4,
            max_lines: 25,
            min_bytes: 100,
            max_bytes: 600,
        }
    }
}

enum SizeFit {
    TooSmall,
    Fits,
    TooBig,
}

impl LengthFilter {
    fn classify(&self, lines: usize, bytes: usize) -> SizeFit {
        // "Too big" wins over "too small" if both bounds are violated
        // simultaneously — pathological one-liners with very long content
        // recurse rather than getting silently skipped.
        if lines > self.max_lines || bytes > self.max_bytes {
            SizeFit::TooBig
        } else if lines < self.min_lines || bytes < self.min_bytes {
            SizeFit::TooSmall
        } else {
            SizeFit::Fits
        }
    }
}

/// The shared extraction driver. Holds the chosen language vocabulary and the
/// size filter; the methods below are the language-agnostic algorithm.
pub struct Extractor {
    config: Box<dyn LanguageConfig>,
    filter: LengthFilter,
}

impl Extractor {
    /// Build an extractor for a given language with the default size filter.
    pub fn new(config: Box<dyn LanguageConfig>) -> Self {
        Self {
            config,
            filter: LengthFilter::default(),
        }
    }

    /// Builder-style override of the length filter. Use for tests, and for
    /// any future "scan with looser bounds" mode.
    pub fn with_filter(mut self, filter: LengthFilter) -> Self {
        self.filter = filter;
        self
    }

    /// Human-readable language label, for the UI.
    pub fn language(&self) -> &'static str {
        self.config.language()
    }

    /// File extensions this extractor's language claims.
    pub fn extensions(&self) -> &[&'static str] {
        self.config.extensions()
    }

    /// Extract Exercises from a source file's contents.
    pub fn extract(&self, path: &Path, source: &[u8]) -> anyhow::Result<Vec<Exercise>> {
        let mut parser = Parser::new();
        parser
            .set_language(&self.config.tree_sitter_language())
            .with_context(|| format!("set tree-sitter language for {}", self.config.language()))?;
        let tree = parser
            .parse(source, None)
            .ok_or_else(|| anyhow!("tree-sitter failed to parse {}", path.display()))?;

        let indent_unit = detect_indent_unit(source);
        let mut exercises = Vec::new();
        self.walk_for_declarations(&tree.root_node(), source, path, indent_unit, &mut exercises)?;
        Ok(exercises)
    }

    /// Walk a node looking for declarations to dispatch to
    /// [`Self::handle_declaration`]. When a child is itself a declaration, we
    /// hand it off (which may emit it, or recurse further if it's too big).
    /// When a child is *not* a declaration (a class body, function body,
    /// import, etc.), we transparently descend through it — this is how
    /// methods get found inside their enclosing body wrapper.
    fn walk_for_declarations(
        &self,
        node: &Node,
        source: &[u8],
        path: &Path,
        indent_unit: IndentUnit,
        out: &mut Vec<Exercise>,
    ) -> anyhow::Result<()> {
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            if self.config.is_declaration(child.kind()) {
                self.handle_declaration(&child, source, path, indent_unit, out)?;
            } else {
                self.walk_for_declarations(&child, source, path, indent_unit, out)?;
            }
        }
        Ok(())
    }

    /// Decide what to do with one declaration node:
    /// - `Fits`: emit as Exercise. No further descent — even if the
    ///   declaration contains nested declarations, the user is drilling the
    ///   whole outer thing.
    /// - `TooBig`: descend into its declaration children. The same policy
    ///   applies recursively, so a too-big class might surface its
    ///   too-big-but-recursively-extractable nested type, etc.
    /// - `TooSmall`: skip. Nothing to do.
    fn handle_declaration(
        &self,
        node: &Node,
        source: &[u8],
        path: &Path,
        indent_unit: IndentUnit,
        out: &mut Vec<Exercise>,
    ) -> anyhow::Result<()> {
        let lines = node_line_count(node);
        let bytes = node.byte_range().len();
        match self.filter.classify(lines, bytes) {
            SizeFit::Fits => {
                let text = self
                    .build_exercise_text(*node, source)
                    .with_context(|| format!("build exercise text in {}", path.display()))?;
                out.push(Exercise {
                    source_path: path.to_path_buf(),
                    byte_range: node.byte_range(),
                    text,
                    indent_unit,
                });
            }
            SizeFit::TooBig => {
                self.walk_for_declarations(node, source, path, indent_unit, out)?;
            }
            SizeFit::TooSmall => {
                // Drop on the floor.
            }
        }
        Ok(())
    }

    /// Build the textual content of an Exercise from a declaration node:
    /// strip comment ranges, collapse the blank-line residue, then dedent.
    fn build_exercise_text(&self, node: Node, source: &[u8]) -> anyhow::Result<String> {
        let outer = node.byte_range();
        let comments = self.collect_comment_ranges(node);
        let stripped = strip_ranges(source, outer, &comments)?;
        let collapsed = collapse_blank_lines(&stripped);
        Ok(dedent(&collapsed))
    }

    /// Walk `node`'s entire subtree (all children, named or anonymous) and
    /// collect the byte ranges of comment nodes. Recursion stops at each
    /// comment — we don't descend into them.
    fn collect_comment_ranges(&self, node: Node) -> Vec<Range<usize>> {
        let mut ranges = Vec::new();
        self.visit_for_comments(node, &mut ranges);
        ranges
    }

    fn visit_for_comments(&self, node: Node, ranges: &mut Vec<Range<usize>>) {
        if self.config.is_comment(node.kind()) {
            ranges.push(node.byte_range());
            return;
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.visit_for_comments(child, ranges);
        }
    }
}

/// Count the number of source lines spanned by a node, inclusive of the
/// start and end rows. `Point::row` is 0-indexed; a node on row 5 only is
/// 1 line, a node spanning rows 5..=8 is 4 lines.
fn node_line_count(node: &Node) -> usize {
    node.end_position().row - node.start_position().row + 1
}

/// Extract the bytes of `outer` from `source`, excluding any bytes that fall
/// inside one of `to_strip`. The strip ranges may be in any order, may
/// overlap, and may extend beyond `outer` (we clip).
fn strip_ranges(
    source: &[u8],
    outer: Range<usize>,
    to_strip: &[Range<usize>],
) -> anyhow::Result<String> {
    // Filter to ranges that actually overlap `outer`, then sort by start.
    let mut sorted: Vec<Range<usize>> = to_strip
        .iter()
        .filter(|r| r.start < outer.end && r.end > outer.start)
        .cloned()
        .collect();
    sorted.sort_by_key(|r| r.start);

    let mut out = Vec::with_capacity(outer.len());
    let mut cursor = outer.start;
    for r in sorted {
        let strip_start = r.start.max(outer.start);
        let strip_end = r.end.min(outer.end);
        if cursor < strip_start {
            out.extend_from_slice(&source[cursor..strip_start]);
        }
        cursor = cursor.max(strip_end);
    }
    if cursor < outer.end {
        out.extend_from_slice(&source[cursor..outer.end]);
    }

    String::from_utf8(out).map_err(|e| anyhow!("utf-8 decode after strip: {e}"))
}

/// Per-line cleanup: trim trailing whitespace, drop whitespace-only lines.
/// The result has no trailing newline.
///
/// **Known limitation**: this also removes blank lines that appeared in the
/// original source as intentional spacing, and would corrupt multi-line
/// string literals that contain blank lines as content. We accept both —
/// multi-line strings with blank-line content are rare in source.
fn collapse_blank_lines(s: &str) -> String {
    s.lines()
        .map(|line| line.trim_end())
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Strip the common leading whitespace from a multi-line text.
///
/// **Skip-line-1 rule**: tree-sitter's `utf8_text` slices the node starting
/// at its first character, so a method extracted from a class always has
/// zero leading whitespace on its first line — even when the method was
/// indented in the source. We therefore compute the common prefix across
/// lines `[1..]` (the body lines), then strip from *all* lines (line 0 is
/// unaffected because it has no leading whitespace to strip).
///
/// Single-line input is returned as-is.
fn dedent(s: &str) -> String {
    let lines: Vec<&str> = s.lines().collect();
    if lines.len() < 2 {
        return s.to_string();
    }

    // Compute common leading whitespace across body lines (lines[1..]).
    let mut common = leading_whitespace(lines[1]);
    for line in &lines[2..] {
        let ws = leading_whitespace(line);
        common = common_byte_prefix(common, ws);
    }

    lines
        .iter()
        .map(|line| line.strip_prefix(common).unwrap_or(line))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Return the leading whitespace prefix of `s` as a borrowed slice.
fn leading_whitespace(s: &str) -> &str {
    let trimmed = s.trim_start();
    &s[..s.len() - trimmed.len()]
}

/// The longest common byte prefix of two strings, returned as a slice of
/// `a`. Whitespace characters (space and tab) are ASCII single-byte, so we
/// never split a multi-byte char.
fn common_byte_prefix<'a>(a: &'a str, b: &str) -> &'a str {
    let i = a.bytes().zip(b.bytes()).take_while(|(x, y)| x == y).count();
    &a[..i]
}

/// Detect the file's dominant indent unit from the first indented line.
/// Returns `IndentUnit::Tab` if the first indented line starts with `'\t'`,
/// or `IndentUnit::Spaces(N)` for the count of leading spaces. Falls back
/// to `Spaces(4)` if the source contains no indented lines or isn't UTF-8.
fn detect_indent_unit(source: &[u8]) -> IndentUnit {
    let Ok(s) = std::str::from_utf8(source) else {
        return IndentUnit::Spaces(4);
    };

    for line in s.lines() {
        let bytes = line.as_bytes();
        match bytes.first() {
            Some(b'\t') => return IndentUnit::Tab,
            Some(b' ') => {
                let n = bytes.iter().take_while(|&&b| b == b' ').count();
                // Cap at u8::MAX defensively; realistic indents are <= 16.
                return IndentUnit::Spaces(n.min(u8::MAX as usize) as u8);
            }
            _ => continue,
        }
    }

    IndentUnit::Spaces(4)
}

#[cfg(test)]
mod tests {
    //! Tests for the language-agnostic pipeline. Per-language grammar
    //! coverage lives next to each `LanguageConfig` (e.g. `swift.rs`).
    use super::*;
    use pretty_assertions::assert_eq;

    // -- size classification: pure helper ------------------------------------

    #[test]
    fn classify_fits() {
        let f = LengthFilter::default();
        assert!(matches!(f.classify(10, 200), SizeFit::Fits));
    }

    #[test]
    fn classify_too_small_by_lines() {
        let f = LengthFilter::default();
        assert!(matches!(f.classify(2, 200), SizeFit::TooSmall));
    }

    #[test]
    fn classify_too_small_by_bytes() {
        let f = LengthFilter::default();
        assert!(matches!(f.classify(10, 50), SizeFit::TooSmall));
    }

    #[test]
    fn classify_too_big_by_lines() {
        let f = LengthFilter::default();
        assert!(matches!(f.classify(30, 200), SizeFit::TooBig));
    }

    #[test]
    fn classify_too_big_by_bytes() {
        let f = LengthFilter::default();
        assert!(matches!(f.classify(10, 700), SizeFit::TooBig));
    }

    #[test]
    fn classify_too_big_takes_precedence_over_too_small() {
        let f = LengthFilter::default();
        // Pathological one-liner: 1 line, 800 bytes. Too big by bytes,
        // too small by lines. "Too big" wins → would recurse.
        assert!(matches!(f.classify(1, 800), SizeFit::TooBig));
    }

    // -- strip_ranges: pure helper -------------------------------------------

    #[test]
    fn strip_ranges_empty_strip_list_returns_full_range() {
        let src = b"hello world";
        let result = strip_ranges(src, 0..src.len(), &[]).unwrap();
        assert_eq!(result, "hello world");
    }

    #[test]
    // `&[2..4]` is genuinely a one-element slice of ranges (the strip list),
    // not a mistaken range-init, so the lint is a false positive here.
    #[allow(clippy::single_range_in_vec_init)]
    fn strip_ranges_removes_a_middle_range() {
        let src = b"abcdef";
        let result = strip_ranges(src, 0..6, &[2..4]).unwrap();
        assert_eq!(result, "abef");
    }

    #[test]
    fn strip_ranges_handles_unsorted_input() {
        let src = b"abcdefgh";
        // Strip ranges supplied out of order — function must sort them.
        let result = strip_ranges(src, 0..8, &[4..6, 1..2]).unwrap();
        assert_eq!(result, "acdgh");
    }

    #[test]
    fn strip_ranges_clipped_to_outer() {
        let src = b"abcdef";
        // Outer is "cde" (indices 2..5). Strip ranges extend outside.
        let result = strip_ranges(src, 2..5, &[0..3, 4..10]).unwrap();
        // After clipping: strip 2..3 ('c') and 4..5 ('e'). Keep 'd'.
        assert_eq!(result, "d");
    }

    // -- collapse_blank_lines: pure helper -----------------------------------

    #[test]
    fn collapse_blank_lines_trims_trailing_whitespace() {
        assert_eq!(collapse_blank_lines("hello   \nworld\n"), "hello\nworld");
    }

    #[test]
    fn collapse_blank_lines_drops_whitespace_only_lines() {
        assert_eq!(collapse_blank_lines("a\n\nb\n   \nc"), "a\nb\nc");
    }

    #[test]
    fn collapse_blank_lines_no_trailing_newline() {
        assert_eq!(collapse_blank_lines("a\nb\nc"), "a\nb\nc");
    }

    #[test]
    fn collapse_blank_lines_empty_input() {
        assert_eq!(collapse_blank_lines(""), "");
    }

    // -- dedent --------------------------------------------------------------

    #[test]
    fn dedent_single_line_unchanged() {
        assert_eq!(dedent("func foo()"), "func foo()");
    }

    #[test]
    fn dedent_top_level_function_unchanged() {
        // Body is indented; line 1 has no leading whitespace, line 3 (`}`)
        // also has none. Common prefix is empty → no dedent.
        let src = "func foo() {\n    let x = 1\n}";
        assert_eq!(dedent(src), src);
    }

    #[test]
    fn dedent_strips_method_body_indent() {
        // Simulates a method extracted from inside a class: line 1 has no
        // leading whitespace (tree-sitter slices at the `func`), but body
        // lines retain their original column offsets.
        let src = "func foo() {\n        let x = 1\n    }";
        let expected = "func foo() {\n    let x = 1\n}";
        assert_eq!(dedent(src), expected);
    }

    #[test]
    fn dedent_strips_deeply_nested_method() {
        let src = "func bar() {\n            let x = 1\n        }";
        let expected = "func bar() {\n    let x = 1\n}";
        assert_eq!(dedent(src), expected);
    }

    #[test]
    fn dedent_tab_indent() {
        let src = "func foo() {\n\t\tlet x = 1\n\t}";
        let expected = "func foo() {\n\tlet x = 1\n}";
        assert_eq!(dedent(src), expected);
    }

    #[test]
    fn dedent_mixed_tab_and_spaces_produces_empty_prefix() {
        // Line 2 starts with tab, line 3 starts with space — they share no
        // common byte prefix, so nothing is dedented.
        let src = "func foo() {\n\tlet x = 1\n  let y = 2";
        assert_eq!(dedent(src), src);
    }

    // -- common_byte_prefix and leading_whitespace pure helpers --------------

    #[test]
    fn leading_whitespace_basic() {
        assert_eq!(leading_whitespace("    code"), "    ");
        assert_eq!(leading_whitespace("\t\tcode"), "\t\t");
        assert_eq!(leading_whitespace("no_indent"), "");
        assert_eq!(leading_whitespace(""), "");
    }

    #[test]
    fn common_byte_prefix_basic() {
        assert_eq!(common_byte_prefix("    ", "    "), "    ");
        assert_eq!(common_byte_prefix("    ", "  "), "  ");
        assert_eq!(common_byte_prefix("    ", "\t   "), "");
        assert_eq!(common_byte_prefix("    ", ""), "");
    }

    // -- detect_indent_unit --------------------------------------------------

    #[test]
    fn detect_indent_unit_spaces_4() {
        let src = b"func foo() {\n    let x = 1\n}\n";
        assert_eq!(detect_indent_unit(src), IndentUnit::Spaces(4));
    }

    #[test]
    fn detect_indent_unit_spaces_2() {
        let src = b"func foo() {\n  let x = 1\n}\n";
        assert_eq!(detect_indent_unit(src), IndentUnit::Spaces(2));
    }

    #[test]
    fn detect_indent_unit_tab() {
        let src = b"func foo() {\n\tlet x = 1\n}\n";
        assert_eq!(detect_indent_unit(src), IndentUnit::Tab);
    }

    #[test]
    fn detect_indent_unit_falls_back_with_no_indented_lines() {
        let src = b"func foo() {}\nlet x = 1\n";
        assert_eq!(detect_indent_unit(src), IndentUnit::Spaces(4));
    }

    #[test]
    fn detect_indent_unit_falls_back_on_invalid_utf8() {
        let src = &[0xFF_u8, 0xFE, 0xFD];
        assert_eq!(detect_indent_unit(src), IndentUnit::Spaces(4));
    }
}
