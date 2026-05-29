//! Swift language vocabulary for the shared [`Extractor`](super::Extractor).
//!
//! Everything Swift-specific lives here: the tree-sitter-swift grammar and the
//! set of node kinds that count as declarations and comments. The extraction
//! algorithm itself is in the parent module.

use super::LanguageConfig;

/// Swift's [`LanguageConfig`]. Zero-sized — it carries no state, just the
/// per-language vocabulary expressed as trait methods.
pub struct SwiftConfig;

impl LanguageConfig for SwiftConfig {
    fn language(&self) -> &'static str {
        "swift"
    }

    fn extensions(&self) -> &[&'static str] {
        &["swift"]
    }

    fn tree_sitter_language(&self) -> tree_sitter::Language {
        tree_sitter_swift::LANGUAGE.into()
    }

    /// Declaration node kinds that should become Exercises.
    ///
    /// Notable grammar quirk: `class_declaration` is tree-sitter-swift's
    /// umbrella kind for `class`, `struct`, `enum`, `extension`, and `actor`.
    /// The umbrella tests (`struct_is_extracted`, etc.) pin this behavior.
    fn is_declaration(&self, kind: &str) -> bool {
        matches!(
            kind,
            "function_declaration"
                | "class_declaration"
                | "protocol_declaration"
                | "property_declaration"
                | "typealias_declaration"
        )
    }

    fn is_comment(&self, kind: &str) -> bool {
        matches!(kind, "comment" | "multiline_comment")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exercise::{Exercise, IndentUnit};
    use crate::extractor::{Extractor, LengthFilter};
    use pretty_assertions::assert_eq;
    use std::path::Path;
    use tree_sitter::{Node, Parser};

    /// A filter that accepts essentially anything. Used by tests that check
    /// *what* gets extracted (grammar coverage, byte ranges, etc.) without
    /// involving the size-bounds logic.
    const PERMISSIVE: LengthFilter = LengthFilter {
        min_lines: 1,
        max_lines: 10_000,
        min_bytes: 1,
        max_bytes: 1_000_000,
    };

    fn swift_extractor(filter: LengthFilter) -> Extractor {
        Extractor::new(Box::new(SwiftConfig)).with_filter(filter)
    }

    fn extract_permissive(source: &str) -> Vec<Exercise> {
        swift_extractor(PERMISSIVE)
            .extract(Path::new("test.swift"), source.as_bytes())
            .expect("extract should not fail on well-formed Swift")
    }

    fn extract_with(filter: LengthFilter, source: &str) -> Vec<Exercise> {
        swift_extractor(filter)
            .extract(Path::new("test.swift"), source.as_bytes())
            .expect("extract should not fail on well-formed Swift")
    }

    /// One-shot inspection: dumps the kind of every top-level named node.
    /// `#[ignore]` so it doesn't run on every `cargo test` — its purpose is
    /// to make grammar-version drift visible when tree-sitter-swift is
    /// upgraded.
    ///
    /// Run with:
    /// ```text
    /// cargo test --lib -- --ignored --nocapture inspect_top_level_kinds
    /// ```
    #[test]
    #[ignore]
    fn inspect_top_level_kinds() {
        let source = br#"
import Foundation
let global = 42
func hello() {}
struct Point { var x: Int }
class Foo { func bar() {} }
protocol P { func required() }
extension Int { func double() -> Int { return self * 2 } }
typealias MyInt = Int
enum Color { case red, green, blue }
actor Counter { var count: Int = 0 }
"#;
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_swift::LANGUAGE.into())
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

    // -- Grammar coverage tests (use PERMISSIVE filter) ----------------------

    #[test]
    fn empty_source_produces_no_exercises() {
        assert_eq!(extract_permissive("").len(), 0);
    }

    #[test]
    fn finds_a_top_level_function() {
        let src = "func hello() {}\n";
        let result = extract_permissive(src);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].text, "func hello() {}");
    }

    #[test]
    fn finds_multiple_top_level_declarations() {
        let src = "\
func a() {}

class B {
    func b() {}
}

struct C {
    var x: Int
}
";
        let result = extract_permissive(src);
        assert_eq!(result.len(), 3);
    }

    #[test]
    fn imports_are_not_exercises() {
        let src = "import Foundation\n\nfunc hello() {}\n";
        let result = extract_permissive(src);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].text, "func hello() {}");
    }

    #[test]
    fn struct_is_extracted() {
        assert_eq!(extract_permissive("struct Point { var x: Int }\n").len(), 1);
    }

    #[test]
    fn class_is_extracted() {
        assert_eq!(extract_permissive("class Foo { func bar() {} }\n").len(), 1);
    }

    #[test]
    fn enum_is_extracted() {
        assert_eq!(
            extract_permissive("enum Color { case red, green }\n").len(),
            1
        );
    }

    #[test]
    fn extension_is_extracted() {
        assert_eq!(
            extract_permissive("extension Int { func double() -> Int { return self * 2 } }\n")
                .len(),
            1
        );
    }

    #[test]
    fn actor_is_extracted() {
        assert_eq!(
            extract_permissive("actor Counter { var count: Int = 0 }\n").len(),
            1
        );
    }

    #[test]
    fn protocol_is_extracted() {
        assert_eq!(
            extract_permissive("protocol P { func required() }\n").len(),
            1
        );
    }

    #[test]
    fn typealias_is_extracted() {
        assert_eq!(extract_permissive("typealias MyInt = Int\n").len(), 1);
    }

    #[test]
    fn property_declaration_is_extracted() {
        assert_eq!(extract_permissive("let global = 42\n").len(), 1);
        assert_eq!(extract_permissive("var name: String = \"hi\"\n").len(), 1);
    }

    #[test]
    fn byte_range_points_into_source() {
        let src = "func a() {}\nfunc b() {}\n";
        let result = extract_permissive(src);
        assert_eq!(result.len(), 2);
        // The byte_range points at the original source location. The text
        // may differ from `&src[byte_range]` after comment stripping, but
        // for these comment-free declarations they coincide.
        assert_eq!(result[0].byte_range.start, 0);
        assert!(result[1].byte_range.start > result[0].byte_range.end);
    }

    // -- Length filter + recursive descent tests -----------------------------

    /// Filter that's tight enough to make the length logic visible while
    /// keeping fixture sizes manageable.
    const TIGHT: LengthFilter = LengthFilter {
        min_lines: 4,
        max_lines: 8,
        min_bytes: 1,
        max_bytes: 1_000,
    };

    #[test]
    fn declaration_too_small_is_skipped() {
        let src = "func hello() {}\n";
        assert!(extract_with(TIGHT, src).is_empty());
    }

    #[test]
    fn declaration_in_range_is_emitted() {
        let src = "\
func hello() {
    let a = 1
    let b = 2
    print(a + b)
}
";
        let result = extract_with(TIGHT, src);
        assert_eq!(result.len(), 1);
        assert!(result[0].text.starts_with("func hello"));
    }

    #[test]
    fn declaration_too_big_with_no_recursable_children_is_skipped() {
        // A long function. Its body is statements, not declarations, so
        // recursion finds nothing extractable.
        let body: String = (0..20).map(|i| format!("    let v{i} = {i}\n")).collect();
        let src = format!("func big() {{\n{body}}}\n");
        assert!(extract_with(TIGHT, &src).is_empty());
    }

    #[test]
    fn too_big_class_descends_into_methods() {
        // Each method is 4 lines, fits TIGHT. Three methods = ~14 lines for
        // the class, which is over TIGHT's max_lines of 8.
        let method =
            "    func m(_ x: Int) -> Int {\n        let r = x * 2\n        return r\n    }\n";
        let src = format!("class C {{\n{method}{method}{method}}}\n");
        let result = extract_with(TIGHT, &src);
        assert_eq!(result.len(), 3);
        for ex in &result {
            assert!(ex.text.starts_with("func m"));
        }
    }

    #[test]
    fn fitting_outer_class_is_emitted_not_its_methods() {
        // The outer class is small enough to fit; we emit it once, *not* each
        // method inside it.
        let src = "\
class C {
    func a() {}
    func b() {}
}
";
        let result = extract_with(TIGHT, src);
        assert_eq!(result.len(), 1);
        assert!(result[0].text.starts_with("class C"));
    }

    #[test]
    fn nested_class_recursion_finds_inner_declarations() {
        // Outer class is too big. Inner class fits. Recursion should find the
        // inner class.
        let inner = "\
    class Inner {
        func a() {
            let r = 1
            print(r)
        }
    }
";
        // Pad the outer with extra methods so it's too big.
        let padding =
            "    func p() {\n        let a = 1\n        let b = 2\n        print(a, b)\n    }\n";
        let src = format!("class Outer {{\n{inner}{padding}{padding}{padding}}}\n");
        let result = extract_with(TIGHT, &src);
        // Inner class should appear; the padding methods also fit (4 lines each).
        assert!(result.iter().any(|e| e.text.starts_with("class Inner")));
        assert!(result.iter().any(|e| e.text.starts_with("func p")));
    }

    // -- Inspection: dump comment node kinds ---------------------------------

    /// Walks a parsed source and prints every node whose kind contains
    /// "comment". Run with:
    /// ```text
    /// cargo test --lib -- --ignored --nocapture inspect_comment_kinds
    /// ```
    #[test]
    #[ignore]
    fn inspect_comment_kinds() {
        let source: &[u8] = br#"
// line comment
/* block comment */
/// doc comment
/** doc block comment */
func foo() {
    // standalone inside
    let x = 1 // trailing
    /* mid */
    let y = 2
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
            .set_language(&tree_sitter_swift::LANGUAGE.into())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        walk(tree.root_node(), source);
    }

    // -- Comment stripping tests (end-to-end) --------------------------------

    #[test]
    fn strips_standalone_line_comment() {
        let src = "\
func foo() {
    // a comment
    let x = 1
    print(x)
}
";
        let result = extract_permissive(src);
        assert_eq!(result.len(), 1);
        assert!(!result[0].text.contains("//"));
        assert!(!result[0].text.contains("a comment"));
    }

    #[test]
    fn strips_inline_line_comment() {
        let src = "\
func foo() {
    let x = 1 // unused
    let y = 2
    print(x, y)
}
";
        let result = extract_permissive(src);
        assert_eq!(result.len(), 1);
        assert!(!result[0].text.contains("//"));
        assert!(!result[0].text.contains("unused"));
        // Trailing whitespace after the stripped comment must be trimmed.
        assert!(!result[0].text.contains("let x = 1 \n"));
        assert!(result[0].text.contains("let x = 1\n"));
    }

    #[test]
    fn strips_block_comment() {
        let src = "\
func foo() {
    /* a block comment */
    let x = 1
    print(x)
}
";
        let result = extract_permissive(src);
        assert_eq!(result.len(), 1);
        assert!(!result[0].text.contains("/*"));
        assert!(!result[0].text.contains("a block comment"));
    }

    #[test]
    fn strips_doc_comment() {
        // tree-sitter-swift uses `"comment"` for both `//` and `///`; this
        // test pins that — if the grammar ever splits doc comments into a
        // separate `doc_comment` kind, this test will fail and we'll know
        // to extend `is_comment`.
        let src = "\
/// A documenting comment.
func foo() {
    let x = 1
    print(x)
}
";
        let result = extract_permissive(src);
        assert!(result.iter().all(|e| !e.text.contains("///")));
        assert!(result.iter().all(|e| !e.text.contains("documenting")));
    }

    #[test]
    fn strips_block_doc_comment() {
        // `"multiline_comment"` covers both `/* */` and `/** */`.
        let src = "\
/** A documenting block comment. */
func foo() {
    let x = 1
    print(x)
}
";
        let result = extract_permissive(src);
        assert!(result.iter().all(|e| !e.text.contains("/*")));
        assert!(result.iter().all(|e| !e.text.contains("documenting")));
    }

    #[test]
    fn no_comments_produces_clean_text() {
        let src = "func foo() {\n    let x = 1\n    print(x)\n}\n";
        let result = extract_permissive(src);
        assert_eq!(result.len(), 1);
        assert_eq!(
            result[0].text,
            "func foo() {\n    let x = 1\n    print(x)\n}"
        );
    }

    #[test]
    fn multiple_comments_all_removed() {
        let src = "\
func foo() {
    // first
    let x = 1
    // second
    let y = 2
    // third
    print(x, y)
}
";
        let result = extract_permissive(src);
        assert_eq!(result.len(), 1);
        assert!(!result[0].text.contains("//"));
        assert!(!result[0].text.contains("first"));
        assert!(!result[0].text.contains("second"));
        assert!(!result[0].text.contains("third"));
    }

    #[test]
    fn intentional_blank_lines_are_also_collapsed() {
        let src = "\
func foo() {
    let x = 1

    let y = 2

    print(x, y)
}
";
        let result = extract_permissive(src);
        assert_eq!(result.len(), 1);
        // Known simplification: intentional blank lines disappear too.
        assert!(!result[0].text.contains("\n\n"));
    }

    #[test]
    fn exercise_text_has_no_trailing_newline() {
        let src = "func foo() {\n    let x = 1\n}\n";
        let result = extract_permissive(src);
        assert_eq!(result.len(), 1);
        assert!(!result[0].text.ends_with('\n'));
    }

    // -- end-to-end through extract() ----------------------------------------

    #[test]
    fn extracted_method_is_dedented() {
        // A class with one method inside. We use a deliberately tight
        // max_lines so the 7-line class doesn't fit and we descend into
        // the 5-line method. The method's body should come out dedented
        // to canonical column 0/4.
        let src = "\
class Foo {
    func bar() {
        let x = 1
        let y = 2
        print(x + y)
    }
}
";
        let filter = LengthFilter {
            min_lines: 4,
            max_lines: 6,
            min_bytes: 1,
            max_bytes: 1_000,
        };
        let result = extract_with(filter, src);
        assert_eq!(result.len(), 1);
        assert_eq!(
            result[0].text,
            "func bar() {\n    let x = 1\n    let y = 2\n    print(x + y)\n}"
        );
    }

    #[test]
    fn extracted_exercise_carries_detected_indent_unit() {
        // Spaces source → Spaces(4)
        let spaces_src = "\
func foo() {
    let x = 1
    let y = 2
    print(x + y)
}
";
        let exs = extract_with(TIGHT, spaces_src);
        assert_eq!(exs.len(), 1);
        assert_eq!(exs[0].indent_unit, IndentUnit::Spaces(4));

        // Tab source → Tab
        let tab_src = "func foo() {\n\tlet x = 1\n\tlet y = 2\n\tprint(x + y)\n}\n";
        let exs = extract_with(TIGHT, tab_src);
        assert_eq!(exs.len(), 1);
        assert_eq!(exs[0].indent_unit, IndentUnit::Tab);
    }
}
