//! Parser registry for dynamic dispatch to language-specific parsers.

use super::parsers::*;
use crate::core::{Language, LanguageParser};

/// Registry of all available language parsers.
pub struct ParserRegistry {
    parsers: Vec<Box<dyn LanguageParser>>,
}

impl ParserRegistry {
    /// Create a registry with all default parsers.
    pub fn with_defaults() -> Result<Self, crate::core::Error> {
        let mut parsers: Vec<Box<dyn LanguageParser>> = Vec::new();

        // Register all parsers, skipping any that fail to initialize
        macro_rules! try_register {
            ($parser:ty) => {
                if let Ok(p) = <$parser>::new() {
                    parsers.push(Box::new(p));
                }
            };
        }

        try_register!(PythonParser);
        try_register!(JavaScriptParser);
        try_register!(TypeScriptParser);
        try_register!(TsxParser);
        try_register!(JavaParser);
        try_register!(GoParser);
        try_register!(RustParser);
        try_register!(CSharpParser);
        try_register!(RubyParser);
        try_register!(PhpParser);
        try_register!(CppParser);
        try_register!(CParser);
        try_register!(SwiftParser);
        try_register!(BashParser);
        try_register!(ScalaParser);
        try_register!(RParser);
        // Dart, SQL, Kotlin use incompatible tree-sitter bindings — skipped

        if parsers.is_empty() {
            return Err(crate::core::Error::parse("No parsers could be initialized"));
        }

        Ok(Self { parsers })
    }

    /// Get a parser for a specific language.
    pub fn get_parser(&self, language: Language) -> Option<&dyn LanguageParser> {
        self.parsers
            .iter()
            .find(|p| p.language() == language)
            .map(|p| p.as_ref())
    }

    /// Get a parser for a file extension.
    pub fn get_parser_for_extension(&self, ext: &str) -> Option<&dyn LanguageParser> {
        self.parsers
            .iter()
            .find(|p| p.extensions().iter().any(|e| e.eq_ignore_ascii_case(ext)))
            .map(|p| p.as_ref())
    }

    /// Check if any parser supports the given file extension.
    pub fn supports_extension(&self, ext: &str) -> bool {
        self.get_parser_for_extension(ext).is_some()
    }

    /// Get all supported languages.
    pub fn supported_languages(&self) -> Vec<Language> {
        self.parsers.iter().map(|p| p.language()).collect()
    }

    /// Total number of registered parsers.
    pub fn len(&self) -> usize {
        self.parsers.len()
    }

    pub fn is_empty(&self) -> bool {
        self.parsers.is_empty()
    }
}

impl std::fmt::Debug for ParserRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ParserRegistry")
            .field("languages", &self.supported_languages())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_registry_creation() {
        let registry = ParserRegistry::with_defaults().unwrap();
        assert!(registry.len() >= 10, "Should have at least 10 parsers");
    }

    #[test]
    fn test_get_parser_by_language() {
        let registry = ParserRegistry::with_defaults().unwrap();
        assert!(registry.get_parser(Language::Python).is_some());
        assert!(registry.get_parser(Language::Rust).is_some());
        assert!(registry.get_parser(Language::JavaScript).is_some());
    }

    #[test]
    fn test_get_parser_by_extension() {
        let registry = ParserRegistry::with_defaults().unwrap();
        assert!(registry.get_parser_for_extension("py").is_some());
        assert!(registry.get_parser_for_extension("rs").is_some());
        assert!(registry.get_parser_for_extension("js").is_some());
        assert!(registry.get_parser_for_extension("unknown").is_none());
    }
}
