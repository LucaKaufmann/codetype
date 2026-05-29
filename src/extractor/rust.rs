//! Rust language vocabulary for the shared [`Extractor`](super::Extractor).
//!
//! Everything Rust-specific lives here: the tree-sitter-rust grammar and the
//! set of node kinds that count as declarations and comments.

use super::LanguageConfig;

/// Rust's [`LanguageConfig`]. Zero-sized — carries only the per-language
/// vocabulary expressed as trait methods.
pub struct RustConfig;

impl LanguageConfig for RustConfig {
    fn language(&self) -> &'static str {
        "rust"
    }

    fn extensions(&self) -> &[&'static str] {
        &["rs"]
    }

    fn tree_sitter_language(&self) -> tree_sitter::Language {
        tree_sitter_rust::LANGUAGE.into()
    }

    /// Declaration node kinds (`*_item` in tree-sitter-rust) that should
    /// become Exercises. Pinned by the `*_is_extracted` tests below.
    fn is_declaration(&self, kind: &str) -> bool {
        matches!(
            kind,
            "function_item"
                | "struct_item"
                | "enum_item"
                | "union_item"
                | "trait_item"
                | "impl_item"
                | "mod_item"
                | "const_item"
                | "static_item"
                | "type_item"
                | "macro_definition"
        )
    }

    fn is_comment(&self, kind: &str) -> bool {
        matches!(kind, "line_comment" | "block_comment")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exercise::Exercise;
    use crate::extractor::{Extractor, LengthFilter};
    use pretty_assertions::assert_eq;
    use std::path::Path;
    use tree_sitter::{Node, Parser};

    const PERMISSIVE: LengthFilter = LengthFilter {
        min_lines: 1,
        max_lines: 10_000,
        min_bytes: 1,
        max_bytes: 1_000_000,
    };

    fn extract_permissive(source: &str) -> Vec<Exercise> {
        Extractor::new(Box::new(RustConfig))
            .with_filter(PERMISSIVE)
            .extract(Path::new("test.rs"), source.as_bytes())
            .expect("extract should not fail on well-formed Rust")
    }

    /// Inspection: dumps the kind of every top-level named node. `#[ignore]`d
    /// so it only runs on demand; surfaces grammar drift on a version bump.
    ///
    /// ```text
    /// cargo test --lib -- --ignored --nocapture rust::tests::inspect_top_level_kinds
    /// ```
    #[test]
    #[ignore]
    fn inspect_top_level_kinds() {
        let source = br#"
use std::fmt;
const MAX: u32 = 10;
static NAME: &str = "x";
fn hello() {}
struct Point { x: i32 }
enum Color { Red, Green }
union U { a: u32 }
trait T { fn required(&self); }
impl Point { fn new() -> Self { Point { x: 0 } } }
mod thing { pub fn f() {} }
type MyInt = i32;
macro_rules! mymac { () => {}; }
/// a doc comment
fn documented() {}
"#;
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_rust::LANGUAGE.into())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let root = tree.root_node();
        eprintln!("root kind: {}", root.kind());
        let mut cursor = root.walk();
        for (i, c) in root.named_children(&mut cursor).enumerate() {
            let first_line = c.utf8_text(source).unwrap().lines().next().unwrap_or("");
            eprintln!("  [{i}] kind={:?} text={:?}", c.kind(), first_line);
        }
    }

    /// Inspection: dumps every node whose kind contains "comment".
    /// ```text
    /// cargo test --lib -- --ignored --nocapture rust::tests::inspect_comment_kinds
    /// ```
    #[test]
    #[ignore]
    fn inspect_comment_kinds() {
        let source: &[u8] = br#"
// line comment
/* block comment */
/// outer doc
//! inner doc
/** block doc */
fn foo() {
    let x = 1; // trailing
    /* mid */
    let y = 2;
}
"#;
        fn walk(node: Node, source: &[u8]) {
            if node.kind().contains("comment") {
                let text = std::str::from_utf8(&source[node.byte_range()]).unwrap_or("?");
                eprintln!("kind={:?} text={:?}", node.kind(), text);
            }
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                walk(child, source);
            }
        }
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_rust::LANGUAGE.into())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        walk(tree.root_node(), source);
    }

    // -- grammar coverage: one pinning test per Rust construct ---------------

    #[test]
    fn function_is_extracted() {
        assert_eq!(extract_permissive("fn hello() {}\n").len(), 1);
    }

    #[test]
    fn struct_is_extracted() {
        assert_eq!(extract_permissive("struct Point { x: i32 }\n").len(), 1);
    }

    #[test]
    fn enum_is_extracted() {
        assert_eq!(extract_permissive("enum Color { Red, Green }\n").len(), 1);
    }

    #[test]
    fn union_is_extracted() {
        assert_eq!(extract_permissive("union U { a: u32 }\n").len(), 1);
    }

    #[test]
    fn trait_is_extracted() {
        assert_eq!(
            extract_permissive("trait T { fn required(&self); }\n").len(),
            1
        );
    }

    #[test]
    fn impl_is_extracted() {
        assert_eq!(
            extract_permissive("impl Point { fn new() -> Self { Point { x: 0 } } }\n").len(),
            1
        );
    }

    #[test]
    fn mod_is_extracted() {
        assert_eq!(extract_permissive("mod thing { pub fn f() {} }\n").len(), 1);
    }

    #[test]
    fn const_is_extracted() {
        assert_eq!(extract_permissive("const MAX: u32 = 10;\n").len(), 1);
    }

    #[test]
    fn static_is_extracted() {
        assert_eq!(extract_permissive("static NAME: &str = \"x\";\n").len(), 1);
    }

    #[test]
    fn type_alias_is_extracted() {
        assert_eq!(extract_permissive("type MyInt = i32;\n").len(), 1);
    }

    #[test]
    fn macro_definition_is_extracted() {
        assert_eq!(
            extract_permissive("macro_rules! mymac { () => {}; }\n").len(),
            1
        );
    }

    #[test]
    fn use_declaration_is_not_extracted() {
        assert_eq!(extract_permissive("use std::fmt;\n").len(), 0);
    }

    // -- comment stripping ---------------------------------------------------

    #[test]
    fn strips_line_comment() {
        let src = "\
fn foo() {
    // a comment
    let x = 1;
    println!(\"{x}\");
}
";
        let result = extract_permissive(src);
        assert_eq!(result.len(), 1);
        assert!(!result[0].text.contains("//"));
        assert!(!result[0].text.contains("a comment"));
    }

    #[test]
    fn strips_block_comment() {
        let src = "\
fn foo() {
    /* a block comment */
    let x = 1;
    println!(\"{x}\");
}
";
        let result = extract_permissive(src);
        assert_eq!(result.len(), 1);
        assert!(!result[0].text.contains("/*"));
        assert!(!result[0].text.contains("a block comment"));
    }

    #[test]
    fn strips_doc_comment() {
        // `///` doc comments must be stripped too. If tree-sitter-rust ever
        // splits these into a distinct kind, this test fails and we extend
        // `is_comment`.
        let src = "\
/// A documenting comment.
fn foo() {
    let x = 1;
    println!(\"{x}\");
}
";
        let result = extract_permissive(src);
        assert!(result.iter().all(|e| !e.text.contains("///")));
        assert!(result.iter().all(|e| !e.text.contains("documenting")));
    }

    // -- recursive descent: too-big impl surfaces its methods ----------------

    #[test]
    fn too_big_impl_descends_into_methods() {
        let filter = LengthFilter {
            min_lines: 4,
            max_lines: 8,
            min_bytes: 1,
            max_bytes: 1_000,
        };
        let method = "    fn m(&self, x: i32) -> i32 {\n        let r = x * 2;\n        r\n    }\n";
        let src = format!("impl C {{\n{method}{method}{method}}}\n");
        let result = Extractor::new(Box::new(RustConfig))
            .with_filter(filter)
            .extract(Path::new("test.rs"), src.as_bytes())
            .unwrap();
        assert_eq!(result.len(), 3);
        for ex in &result {
            assert!(ex.text.starts_with("fn m"));
        }
    }
}
