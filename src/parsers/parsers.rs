//! Language parser definitions using the `define_parser!` macro.
//!
//! Each parser is generated with a single macro invocation, replacing
//! thousands of lines of copy-paste adapter code.
//!
//! Parsers with incompatible tree-sitter bindings (Dart, SQL, Kotlin)
//! gracefully fail at initialization.

use super::parser_macro::define_parser;
use crate::core::Language;

// 14 language parsers via macro (all using tree-sitter 0.24 compatible LANGUAGE constant)
define_parser!(
    PythonParser,
    Language::Python,
    tree_sitter_python::LANGUAGE,
    &["py"]
);
define_parser!(
    JavaScriptParser,
    Language::JavaScript,
    tree_sitter_javascript::LANGUAGE,
    &["js", "jsx", "mjs"]
);
define_parser!(
    TypeScriptParser,
    Language::TypeScript,
    tree_sitter_typescript::LANGUAGE_TYPESCRIPT,
    &["ts"]
);
define_parser!(
    TsxParser,
    Language::TypeScript,
    tree_sitter_typescript::LANGUAGE_TSX,
    &["tsx"]
);
define_parser!(
    JavaParser,
    Language::Java,
    tree_sitter_java::LANGUAGE,
    &["java"]
);
define_parser!(GoParser, Language::Go, tree_sitter_go::LANGUAGE, &["go"]);
define_parser!(
    RustParser,
    Language::Rust,
    tree_sitter_rust::LANGUAGE,
    &["rs"]
);
define_parser!(
    CSharpParser,
    Language::CSharp,
    tree_sitter_c_sharp::LANGUAGE,
    &["cs"]
);
define_parser!(
    RubyParser,
    Language::Ruby,
    tree_sitter_ruby::LANGUAGE,
    &["rb"]
);
define_parser!(
    PhpParser,
    Language::PHP,
    tree_sitter_php::LANGUAGE_PHP,
    &["php"]
);
define_parser!(
    CppParser,
    Language::Cpp,
    tree_sitter_cpp::LANGUAGE,
    &["cpp", "cc", "cxx", "hpp"]
);
define_parser!(CParser, Language::C, tree_sitter_c::LANGUAGE, &["c", "h"]);
define_parser!(
    SwiftParser,
    Language::Swift,
    tree_sitter_swift::LANGUAGE,
    &["swift"]
);
define_parser!(
    BashParser,
    Language::Bash,
    tree_sitter_bash::LANGUAGE,
    &["sh", "bash"]
);
define_parser!(
    ScalaParser,
    Language::Scala,
    tree_sitter_scala::LANGUAGE,
    &["scala"]
);

// Dart, SQL, and Kotlin use older tree-sitter bindings incompatible with 0.24.
// These return errors at initialization; the registry skips them gracefully.
macro_rules! stub_parser {
    ($name:ident, $lang_name:expr) => {
        #[allow(dead_code)]
        pub struct $name;
        #[allow(dead_code)]
        impl $name {
            pub fn new() -> Result<Self, crate::core::Error> {
                Err(crate::core::Error::parse(concat!(
                    $lang_name,
                    " parser not available (incompatible tree-sitter binding)"
                )))
            }
        }
    };
}

stub_parser!(DartParser, "Dart");
stub_parser!(SqlParser, "SQL");
stub_parser!(KotlinParser, "Kotlin");
