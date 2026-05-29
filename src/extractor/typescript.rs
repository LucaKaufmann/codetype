//! TypeScript language vocabulary for the shared [`Extractor`](super::Extractor).
//!
//! Scoped to `.ts` files using tree-sitter-typescript's TypeScript grammar.
//! `.tsx` (the JSX-aware grammar) is a deliberate follow-up: a `LanguageConfig`
//! owns exactly one grammar, and the TS and TSX grammars disagree on a couple
//! of syntaxes (`<T>x` type assertions vs. JSX), so supporting both cleanly
//! means a second config or per-file grammar selection.

use super::LanguageConfig;

/// TypeScript's [`LanguageConfig`]. Zero-sized.
pub struct TypeScriptConfig;

impl LanguageConfig for TypeScriptConfig {
    fn language(&self) -> &'static str {
        "typescript"
    }

    fn extensions(&self) -> &[&'static str] {
        &["ts"]
    }

    fn tree_sitter_language(&self) -> tree_sitter::Language {
        tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()
    }

    /// Declaration node kinds that should become Exercises. Pinned by the
    /// `*_is_extracted` tests below.
    ///
    /// Note: top-level `export class Foo {}` parses as an `export_statement`
    /// wrapping a `class_declaration`. `export_statement` is intentionally
    /// *not* listed — the driver descends through non-declaration nodes, so
    /// the inner declaration is still found (and emitted without the bare
    /// `export` keyword stripped, since we extract the inner node).
    fn is_declaration(&self, kind: &str) -> bool {
        matches!(
            kind,
            "function_declaration"
                | "class_declaration"
                | "abstract_class_declaration"
                | "method_definition"
                | "interface_declaration"
                | "type_alias_declaration"
                | "enum_declaration"
                | "lexical_declaration"
        )
    }

    fn is_comment(&self, kind: &str) -> bool {
        matches!(kind, "comment")
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
        Extractor::new(Box::new(TypeScriptConfig))
            .with_filter(PERMISSIVE)
            .extract(Path::new("test.ts"), source.as_bytes())
            .expect("extract should not fail on well-formed TypeScript")
    }

    /// Inspection: dump top-level named node kinds.
    /// ```text
    /// cargo test --lib -- --ignored --nocapture typescript::tests::inspect_top_level_kinds
    /// ```
    #[test]
    #[ignore]
    fn inspect_top_level_kinds() {
        let source = br#"
import { x } from "y";
const answer = 42;
let mutable: string = "x";
function hello() {}
class Foo { bar() {} }
abstract class Base { abstract f(): void; }
interface Shape { area(): number; }
type MyInt = number;
enum Color { Red, Green }
export class Exported {}
export function exportedFn() {}
"#;
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into())
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

    /// Inspection: dump comment node kinds.
    /// ```text
    /// cargo test --lib -- --ignored --nocapture typescript::tests::inspect_comment_kinds
    /// ```
    #[test]
    #[ignore]
    fn inspect_comment_kinds() {
        let source: &[u8] = br#"
// line comment
/* block comment */
/** jsdoc comment */
function foo() {
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
            .set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        walk(tree.root_node(), source);
    }

    // -- grammar coverage ----------------------------------------------------

    #[test]
    fn function_is_extracted() {
        assert_eq!(extract_permissive("function hello() {}\n").len(), 1);
    }

    #[test]
    fn class_is_extracted() {
        assert_eq!(extract_permissive("class Foo { bar() {} }\n").len(), 1);
    }

    #[test]
    fn abstract_class_is_extracted() {
        assert_eq!(
            extract_permissive("abstract class Base { abstract f(): void; }\n").len(),
            1
        );
    }

    #[test]
    fn interface_is_extracted() {
        assert_eq!(
            extract_permissive("interface Shape { area(): number; }\n").len(),
            1
        );
    }

    #[test]
    fn type_alias_is_extracted() {
        assert_eq!(extract_permissive("type MyInt = number;\n").len(), 1);
    }

    #[test]
    fn enum_is_extracted() {
        assert_eq!(extract_permissive("enum Color { Red, Green }\n").len(), 1);
    }

    #[test]
    fn lexical_declaration_is_extracted() {
        assert_eq!(extract_permissive("const answer = 42;\n").len(), 1);
    }

    #[test]
    fn import_is_not_extracted() {
        assert_eq!(extract_permissive("import { x } from \"y\";\n").len(), 0);
    }

    #[test]
    fn exported_class_is_found_through_export_statement() {
        // `export class Exported {}` is an export_statement wrapping a
        // class_declaration. The driver descends through the export and emits
        // the inner declaration.
        let result = extract_permissive("export class Exported { run() {} }\n");
        assert_eq!(result.len(), 1);
        assert!(result[0].text.starts_with("class Exported"));
    }

    // -- comment stripping ---------------------------------------------------

    #[test]
    fn strips_line_comment() {
        let src = "\
function foo() {
    // a comment
    const x = 1;
    console.log(x);
}
";
        let result = extract_permissive(src);
        assert_eq!(result.len(), 1);
        assert!(!result[0].text.contains("//"));
        assert!(!result[0].text.contains("a comment"));
    }

    #[test]
    fn strips_block_and_jsdoc_comments() {
        let src = "\
/** JSDoc for foo. */
function foo() {
    /* inner block */
    const x = 1;
    console.log(x);
}
";
        let result = extract_permissive(src);
        assert!(result.iter().all(|e| !e.text.contains("/*")));
        assert!(result.iter().all(|e| !e.text.contains("JSDoc")));
        assert!(result.iter().all(|e| !e.text.contains("inner block")));
    }

    // -- recursive descent: too-big class surfaces its methods ---------------

    #[test]
    fn too_big_class_descends_into_methods() {
        let filter = LengthFilter {
            min_lines: 4,
            max_lines: 8,
            min_bytes: 1,
            max_bytes: 1_000,
        };
        let method =
            "    m(x: number): number {\n        const r = x * 2;\n        return r;\n    }\n";
        let src = format!("class C {{\n{method}{method}{method}}}\n");
        let result = Extractor::new(Box::new(TypeScriptConfig))
            .with_filter(filter)
            .extract(Path::new("test.ts"), src.as_bytes())
            .unwrap();
        assert_eq!(result.len(), 3);
        for ex in &result {
            assert!(ex.text.starts_with("m("));
        }
    }
}
