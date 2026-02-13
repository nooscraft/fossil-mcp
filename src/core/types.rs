//! Core type definitions for the Fossil toolkit.
//!
//! All types are self-contained — no external axiom dependencies.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::sync::atomic::{AtomicU32, Ordering};

// =============================================================================
// Language
// =============================================================================

/// Programming languages supported by Fossil (18 languages).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    Python,
    JavaScript,
    TypeScript,
    Java,
    Go,
    Rust,
    CSharp,
    Ruby,
    PHP,
    Cpp,
    C,
    Kotlin,
    Swift,
    Sql,
    Bash,
    Scala,
    Dart,
    R,
}

impl Language {
    /// File extensions for this language (without dots).
    pub fn extensions(self) -> &'static [&'static str] {
        match self {
            Language::Python => &["py"],
            Language::JavaScript => &["js", "jsx", "mjs"],
            Language::TypeScript => &["ts", "tsx"],
            Language::Java => &["java"],
            Language::Go => &["go"],
            Language::Rust => &["rs"],
            Language::CSharp => &["cs"],
            Language::Ruby => &["rb"],
            Language::PHP => &["php"],
            Language::Cpp => &["cpp", "cc", "cxx", "c++", "hpp"],
            Language::C => &["c", "h"],
            Language::Kotlin => &["kt", "kts"],
            Language::Swift => &["swift"],
            Language::Sql => &["sql"],
            Language::Bash => &["sh", "bash"],
            Language::Scala => &["scala"],
            Language::Dart => &["dart"],
            Language::R => &["r", "R"],
        }
    }

    /// Detect language from a file extension (case-insensitive).
    pub fn from_extension(ext: &str) -> Option<Self> {
        let ext = ext.to_lowercase();
        match ext.as_str() {
            "py" => Some(Language::Python),
            "js" | "jsx" | "mjs" => Some(Language::JavaScript),
            "ts" | "tsx" => Some(Language::TypeScript),
            "java" => Some(Language::Java),
            "go" => Some(Language::Go),
            "rs" => Some(Language::Rust),
            "cs" => Some(Language::CSharp),
            "rb" => Some(Language::Ruby),
            "php" => Some(Language::PHP),
            "cpp" | "cc" | "cxx" | "c++" | "hpp" => Some(Language::Cpp),
            "c" | "h" => Some(Language::C),
            "kt" | "kts" => Some(Language::Kotlin),
            "swift" => Some(Language::Swift),
            "sql" => Some(Language::Sql),
            "sh" | "bash" => Some(Language::Bash),
            "scala" => Some(Language::Scala),
            "dart" => Some(Language::Dart),
            "r" => Some(Language::R),
            _ => None,
        }
    }

    /// Detect language from a file path.
    pub fn from_path(path: &std::path::Path) -> Option<Self> {
        path.extension()
            .and_then(|ext| ext.to_str())
            .and_then(Self::from_extension)
    }

    /// Canonical display name.
    pub fn name(self) -> &'static str {
        match self {
            Language::Python => "Python",
            Language::JavaScript => "JavaScript",
            Language::TypeScript => "TypeScript",
            Language::Java => "Java",
            Language::Go => "Go",
            Language::Rust => "Rust",
            Language::CSharp => "C#",
            Language::Ruby => "Ruby",
            Language::PHP => "PHP",
            Language::Cpp => "C++",
            Language::C => "C",
            Language::Kotlin => "Kotlin",
            Language::Swift => "Swift",
            Language::Sql => "SQL",
            Language::Bash => "Bash",
            Language::Scala => "Scala",
            Language::Dart => "Dart",
            Language::R => "R",
        }
    }

    /// Whether two languages can interoperate (share call edges).
    ///
    /// JS/TS and C/C++ are compatible pairs — code in these languages
    /// can reference each other's functions. All other languages are
    /// only compatible with themselves.
    pub fn is_compatible_with(self, other: Language) -> bool {
        if self == other {
            return true;
        }
        matches!(
            (self, other),
            (Language::JavaScript, Language::TypeScript)
                | (Language::TypeScript, Language::JavaScript)
                | (Language::C, Language::Cpp)
                | (Language::Cpp, Language::C)
                | (Language::Java, Language::Kotlin)
                | (Language::Kotlin, Language::Java)
        )
    }

    /// All supported languages.
    pub fn all() -> &'static [Language] {
        &[
            Language::Python,
            Language::JavaScript,
            Language::TypeScript,
            Language::Java,
            Language::Go,
            Language::Rust,
            Language::CSharp,
            Language::Ruby,
            Language::PHP,
            Language::Cpp,
            Language::C,
            Language::Kotlin,
            Language::Swift,
            Language::Sql,
            Language::Bash,
            Language::Scala,
            Language::Dart,
            Language::R,
        ]
    }

    /// Parse language from its display name (case-insensitive).
    /// Examples: "rust", "Rust", "RUST" → Language::Rust
    pub fn from_name(name: &str) -> Option<Self> {
        let lower = name.to_lowercase();
        Self::all()
            .iter()
            .find(|lang| lang.name().to_lowercase() == lower)
            .copied()
    }

    /// Parse comma-separated language list, with validation.
    /// Returns languages and list of unrecognized names.
    /// Example: "rust,python,invalid" → (vec![Rust, Python], vec!["invalid"])
    pub fn parse_list(input: &str) -> (Vec<Language>, Vec<String>) {
        let mut languages = Vec::new();
        let mut invalid = Vec::new();

        for name in input.split(',') {
            let name = name.trim();
            if name.is_empty() {
                continue;
            }
            match Self::from_name(name) {
                Some(lang) => {
                    if !languages.contains(&lang) {
                        languages.push(lang);
                    }
                }
                None => invalid.push(name.to_string()),
            }
        }

        (languages, invalid)
    }

    /// Determine language from file path by extension.
    /// Returns None if extension is not recognized.
    pub fn from_file_path<P: AsRef<std::path::Path>>(path: P) -> Option<Self> {
        let path = path.as_ref();
        let ext = path.extension()?.to_str()?;
        Self::all()
            .iter()
            .find(|lang| lang.extensions().contains(&ext))
            .copied()
    }
}

impl fmt::Display for Language {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.name())
    }
}

// =============================================================================
// Severity & Confidence
// =============================================================================

/// Severity level for findings.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Info = 1,
    Low = 2,
    Medium = 3,
    High = 4,
    Critical = 5,
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Severity::Critical => write!(f, "CRITICAL"),
            Severity::High => write!(f, "HIGH"),
            Severity::Medium => write!(f, "MEDIUM"),
            Severity::Low => write!(f, "LOW"),
            Severity::Info => write!(f, "INFO"),
        }
    }
}

/// Confidence level in a detection.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Confidence {
    Low = 1,
    Medium = 2,
    High = 3,
    Certain = 4,
}

impl fmt::Display for Confidence {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Confidence::Certain => write!(f, "certain"),
            Confidence::High => write!(f, "high"),
            Confidence::Medium => write!(f, "medium"),
            Confidence::Low => write!(f, "low"),
        }
    }
}

// =============================================================================
// NodeId
// =============================================================================

static NODE_ID_COUNTER: AtomicU32 = AtomicU32::new(1);

/// Unique identifier for code nodes (u32 for memory efficiency).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct NodeId(u32);

impl NodeId {
    /// Create a new unique NodeId via global atomic counter.
    pub fn new() -> Self {
        let id = NODE_ID_COUNTER.fetch_add(1, Ordering::Relaxed);
        assert!(id != 0, "NodeId counter overflow");
        NodeId(id)
    }

    pub fn from_u32(id: u32) -> Self {
        NodeId(id)
    }

    pub fn as_u32(self) -> u32 {
        self.0
    }

    /// Reset counter (testing only).
    #[cfg(test)]
    pub fn reset_counter(value: u32) {
        NODE_ID_COUNTER.store(value, Ordering::Relaxed);
    }
}

impl Default for NodeId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for NodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Node({})", self.0)
    }
}

// =============================================================================
// Source Locations
// =============================================================================

/// Source location with file path and line/column ranges.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SourceLocation {
    pub file: String,
    pub line_start: usize,
    pub line_end: usize,
    pub column_start: usize,
    pub column_end: usize,
}

impl SourceLocation {
    pub fn new(
        file: String,
        line_start: usize,
        line_end: usize,
        column_start: usize,
        column_end: usize,
    ) -> Self {
        Self {
            file,
            line_start,
            line_end,
            column_start,
            column_end,
        }
    }

    pub fn lines(&self) -> usize {
        self.line_end.saturating_sub(self.line_start) + 1
    }
}

impl fmt::Display for SourceLocation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}:{}", self.file, self.line_start, self.column_start)
    }
}

/// Byte-level span within source text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SourceSpan {
    pub start: usize,
    pub end: usize,
}

impl SourceSpan {
    pub fn new(start: usize, end: usize) -> Self {
        Self { start, end }
    }

    pub fn len(&self) -> usize {
        self.end.saturating_sub(self.start)
    }

    pub fn is_empty(&self) -> bool {
        self.start >= self.end
    }
}

// =============================================================================
// NodeKind & Visibility
// =============================================================================

/// Type of code node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum NodeKind {
    Function,
    Method,
    AsyncFunction,
    AsyncMethod,
    Class,
    Struct,
    Enum,
    Trait,
    Interface,
    Module,
    Package,
    Variable,
    Constant,
    Parameter,
    ImportDeclaration,
    ExportDeclaration,
    TypeAlias,
    Macro,
    Constructor,
    StaticMethod,
    Lambda,
    Closure,
}

impl fmt::Display for NodeKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NodeKind::Function => write!(f, "function"),
            NodeKind::Method => write!(f, "method"),
            NodeKind::AsyncFunction => write!(f, "async function"),
            NodeKind::AsyncMethod => write!(f, "async method"),
            NodeKind::Class => write!(f, "class"),
            NodeKind::Struct => write!(f, "struct"),
            NodeKind::Enum => write!(f, "enum"),
            NodeKind::Trait => write!(f, "trait"),
            NodeKind::Interface => write!(f, "interface"),
            NodeKind::Module => write!(f, "module"),
            NodeKind::Package => write!(f, "package"),
            NodeKind::Variable => write!(f, "variable"),
            NodeKind::Constant => write!(f, "constant"),
            NodeKind::Parameter => write!(f, "parameter"),
            NodeKind::ImportDeclaration => write!(f, "import"),
            NodeKind::ExportDeclaration => write!(f, "export"),
            NodeKind::TypeAlias => write!(f, "type alias"),
            NodeKind::Macro => write!(f, "macro"),
            NodeKind::Constructor => write!(f, "constructor"),
            NodeKind::StaticMethod => write!(f, "static method"),
            NodeKind::Lambda => write!(f, "lambda"),
            NodeKind::Closure => write!(f, "closure"),
        }
    }
}

/// Code element visibility.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Visibility {
    Public,
    Internal,
    Protected,
    Private,
    Unknown,
}

impl fmt::Display for Visibility {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Visibility::Public => write!(f, "public"),
            Visibility::Internal => write!(f, "internal"),
            Visibility::Protected => write!(f, "protected"),
            Visibility::Private => write!(f, "private"),
            Visibility::Unknown => write!(f, "unknown"),
        }
    }
}

/// Confidence level for call edges.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, PartialOrd, Ord)]
pub enum EdgeConfidence {
    Certain,
    HighLikely,
    Possible,
    Unknown,
}

// =============================================================================
// CodeNode
// =============================================================================

/// A node in the code graph (function, class, method, etc.).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CodeNode {
    pub id: NodeId,
    pub name: String,
    pub kind: NodeKind,
    pub location: SourceLocation,
    pub language: Language,
    pub visibility: Visibility,
    pub lines_of_code: usize,
    pub full_name: String,
    pub parent_id: Option<NodeId>,
    pub is_async: bool,
    pub is_test: bool,
    pub is_generated: bool,
    pub attributes: Vec<String>,
    pub documentation: Option<String>,
}

impl CodeNode {
    pub fn new(
        name: String,
        kind: NodeKind,
        location: SourceLocation,
        language: Language,
        visibility: Visibility,
    ) -> Self {
        let id = NodeId::new();
        Self {
            id,
            full_name: name.clone(),
            name,
            kind,
            location,
            language,
            visibility,
            lines_of_code: 0,
            parent_id: None,
            is_async: false,
            is_test: false,
            is_generated: false,
            attributes: Vec::new(),
            documentation: None,
        }
    }

    pub fn with_lines_of_code(mut self, loc: usize) -> Self {
        self.lines_of_code = loc;
        self
    }

    pub fn with_parent_id(mut self, parent_id: NodeId) -> Self {
        self.parent_id = Some(parent_id);
        self
    }

    pub fn with_full_name(mut self, full_name: String) -> Self {
        self.full_name = full_name;
        self
    }

    pub fn with_async(mut self) -> Self {
        self.is_async = true;
        self
    }

    pub fn with_test(mut self) -> Self {
        self.is_test = true;
        self
    }

    pub fn with_attributes(mut self, attributes: Vec<String>) -> Self {
        self.attributes = attributes;
        self
    }

    pub fn with_documentation(mut self, doc: String) -> Self {
        self.documentation = Some(doc);
        self
    }
}

// =============================================================================
// CallEdge
// =============================================================================

/// An edge in the call graph.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CallEdge {
    pub from: NodeId,
    pub to: NodeId,
    pub confidence: EdgeConfidence,
    pub is_conditional: bool,
    pub is_recursive: bool,
    pub call_count: usize,
}

impl CallEdge {
    pub fn new(from: NodeId, to: NodeId, confidence: EdgeConfidence) -> Self {
        Self {
            from,
            to,
            confidence,
            is_conditional: false,
            is_recursive: false,
            call_count: 1,
        }
    }

    pub fn certain(from: NodeId, to: NodeId) -> Self {
        Self::new(from, to, EdgeConfidence::Certain)
    }

    pub fn with_conditional(mut self) -> Self {
        self.is_conditional = true;
        self
    }

    pub fn with_recursive(mut self) -> Self {
        self.is_recursive = true;
        self
    }

    pub fn with_call_count(mut self, count: usize) -> Self {
        self.call_count = count;
        self
    }
}

/// A call that couldn't be resolved within a single file.
#[derive(Debug, Clone)]
pub struct UnresolvedCall {
    pub caller_id: NodeId,
    pub callee_name: String,
    pub imported_as: Option<String>,
    pub source_module: Option<String>,
    pub call_line: usize,
}

impl UnresolvedCall {
    pub fn new(caller_id: NodeId, callee_name: String, call_line: usize) -> Self {
        Self {
            caller_id,
            callee_name,
            imported_as: None,
            source_module: None,
            call_line,
        }
    }

    pub fn with_import_path(mut self, path: String) -> Self {
        self.imported_as = Some(path);
        self
    }

    pub fn with_source_module(mut self, module: String) -> Self {
        self.source_module = Some(module);
        self
    }
}

/// Class/interface inheritance relationship.
#[derive(Debug, Clone)]
pub struct ClassRelation {
    pub class_name: String,
    pub parents: Vec<String>,
    pub line: usize,
}

// =============================================================================
// Finding & Rule (unified across dead-code, clones, security)
// =============================================================================

/// Classification of dead code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum FossilType {
    Unreachable,
    TransitivelyDead,
    TestOnlyCode,
    UnusedExport,
    UnusedImport,
    UnusedVariable,
    UnusedParameter,
    DeadFunction,
    UnusedField,
    DeadBranch,
}

impl fmt::Display for FossilType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FossilType::Unreachable => write!(f, "unreachable"),
            FossilType::TransitivelyDead => write!(f, "transitively dead"),
            FossilType::TestOnlyCode => write!(f, "test-only code"),
            FossilType::UnusedExport => write!(f, "unused export"),
            FossilType::UnusedImport => write!(f, "unused import"),
            FossilType::UnusedVariable => write!(f, "unused variable"),
            FossilType::UnusedParameter => write!(f, "unused parameter"),
            FossilType::DeadFunction => write!(f, "dead function"),
            FossilType::UnusedField => write!(f, "unused field"),
            FossilType::DeadBranch => write!(f, "dead branch"),
        }
    }
}

/// Impact of removing dead code.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RemovalImpact {
    Safe,
    RisksBreakage,
    Unknown,
    HasDocumentation,
}

/// Type of pattern used in a rule.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PatternType {
    Regex,
    TreeSitterQuery,
    Structural,
    Taint,
}

/// A security/analysis rule.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Rule {
    pub id: String,
    pub name: String,
    pub description: String,
    pub severity: Severity,
    pub confidence: Confidence,
    pub languages: Vec<Language>,
    pub pattern: String,
    pub pattern_type: PatternType,
    pub cwe: Option<String>,
    pub owasp: Option<String>,
    pub fix_suggestion: Option<String>,
    pub tags: Vec<String>,
    pub enabled: bool,
    /// CVSS 3.1 base score (0.0-10.0) for security-severity mapping.
    #[serde(default)]
    pub cvss_score: Option<f64>,
    /// List of CVE IDs associated with this rule (e.g., "CVE-2021-44228").
    #[serde(default)]
    pub cve_references: Vec<String>,
}

impl Rule {
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        pattern: impl Into<String>,
        severity: Severity,
        languages: Vec<Language>,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            description: String::new(),
            severity,
            confidence: Confidence::Medium,
            languages,
            pattern: pattern.into(),
            pattern_type: PatternType::Regex,
            cwe: None,
            owasp: None,
            fix_suggestion: None,
            tags: Vec::new(),
            enabled: true,
            cvss_score: None,
            cve_references: Vec::new(),
        }
    }

    /// Set the CVSS 3.1 base score.
    pub fn with_cvss_score(mut self, score: f64) -> Self {
        self.cvss_score = Some(score);
        self
    }

    /// Set CVE references.
    pub fn with_cve_references(mut self, refs: Vec<String>) -> Self {
        self.cve_references = refs;
        self
    }
}

/// A unified finding from any analysis.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Finding {
    pub rule_id: String,
    pub title: String,
    pub description: String,
    pub severity: Severity,
    pub confidence: Confidence,
    pub location: SourceLocation,
    pub code_snippet: Option<String>,
    pub fix_suggestion: Option<String>,
    pub cwe: Option<String>,
    pub tags: Vec<String>,
    /// Related locations for taint paths, clone locations, etc.
    #[serde(default)]
    pub related_locations: Vec<SourceLocation>,
    /// Suggested fix as replacement text.
    #[serde(default)]
    pub fix_text: Option<String>,
}

impl Finding {
    pub fn new(
        rule_id: impl Into<String>,
        title: impl Into<String>,
        severity: Severity,
        location: SourceLocation,
    ) -> Self {
        Self {
            rule_id: rule_id.into(),
            title: title.into(),
            description: String::new(),
            severity,
            confidence: Confidence::Medium,
            location,
            code_snippet: None,
            fix_suggestion: None,
            cwe: None,
            tags: Vec::new(),
            related_locations: Vec::new(),
            fix_text: None,
        }
    }

    pub fn with_description(mut self, desc: impl Into<String>) -> Self {
        self.description = desc.into();
        self
    }

    pub fn with_confidence(mut self, confidence: Confidence) -> Self {
        self.confidence = confidence;
        self
    }

    pub fn with_snippet(mut self, snippet: impl Into<String>) -> Self {
        self.code_snippet = Some(snippet.into());
        self
    }

    /// Set related locations (taint paths, clone locations, etc.).
    pub fn with_related_locations(mut self, locs: Vec<SourceLocation>) -> Self {
        self.related_locations = locs;
        self
    }

    /// Set suggested fix replacement text.
    pub fn with_fix_text(mut self, text: impl Into<String>) -> Self {
        self.fix_text = Some(text.into());
        self
    }
}

// =============================================================================
// Parsed structures
// =============================================================================

/// A parsed file with extracted code nodes and edges.
#[derive(Debug, Clone)]
pub struct ParsedFile {
    pub path: String,
    pub language: Language,
    pub source: String,
    pub nodes: Vec<CodeNode>,
    pub edges: Vec<CallEdge>,
    pub entry_points: Vec<NodeId>,
    pub unresolved_calls: Vec<UnresolvedCall>,
    pub class_relations: Vec<ClassRelation>,
    pub parse_duration_ms: u32,
}

impl ParsedFile {
    pub fn new(path: String, language: Language, source: String) -> Self {
        Self {
            path,
            language,
            source,
            nodes: Vec::new(),
            edges: Vec::new(),
            entry_points: Vec::new(),
            unresolved_calls: Vec::new(),
            class_relations: Vec::new(),
            parse_duration_ms: 0,
        }
    }
}

/// Aggregated structure from multiple parsed files.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParsedStructure {
    pub nodes: Vec<CodeNode>,
    pub edges: Vec<CallEdge>,
    pub entry_points: Vec<NodeId>,
}

/// Framework-specific detection patterns.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FrameworkPattern {
    pub name: String,
    pub patterns: Vec<String>,
    pub entry_point_indicators: Vec<String>,
}

impl FrameworkPattern {
    pub fn new(name: String, patterns: Vec<String>, indicators: Vec<String>) -> Self {
        Self {
            name,
            patterns,
            entry_point_indicators: indicators,
        }
    }
}

/// Parser configuration options.
#[derive(Debug, Clone)]
pub struct ParserConfig {
    pub follow_imports: bool,
    pub detect_dynamic_calls: bool,
    pub include_generated_code: bool,
    pub include_tests: bool,
    pub max_depth: Option<usize>,
}

impl Default for ParserConfig {
    fn default() -> Self {
        Self {
            follow_imports: true,
            detect_dynamic_calls: false,
            include_generated_code: false,
            include_tests: true,
            max_depth: None,
        }
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_language_from_extension() {
        assert_eq!(Language::from_extension("py"), Some(Language::Python));
        assert_eq!(Language::from_extension("rs"), Some(Language::Rust));
        assert_eq!(Language::from_extension("ts"), Some(Language::TypeScript));
        assert_eq!(Language::from_extension("tsx"), Some(Language::TypeScript));
        assert_eq!(Language::from_extension("unknown"), None);
    }

    #[test]
    fn test_language_all() {
        assert_eq!(Language::all().len(), 18);
    }

    #[test]
    fn test_node_id_sequential() {
        NodeId::reset_counter(1000);
        let id1 = NodeId::new();
        let id2 = NodeId::new();
        assert_eq!(id1.as_u32(), 1000);
        assert_eq!(id2.as_u32(), 1001);
    }

    #[test]
    fn test_node_id_size() {
        assert_eq!(std::mem::size_of::<NodeId>(), 4);
    }

    #[test]
    fn test_source_location_display() {
        let loc = SourceLocation::new("test.rs".to_string(), 10, 20, 5, 15);
        assert_eq!(loc.to_string(), "test.rs:10:5");
        assert_eq!(loc.lines(), 11);
    }

    #[test]
    fn test_source_span() {
        let span = SourceSpan::new(10, 20);
        assert_eq!(span.len(), 10);
        assert!(!span.is_empty());
        assert!(SourceSpan::new(5, 5).is_empty());
    }

    #[test]
    fn test_confidence_ordering() {
        assert!(Confidence::Low < Confidence::Medium);
        assert!(Confidence::Medium < Confidence::High);
        assert!(Confidence::High < Confidence::Certain);
    }

    #[test]
    fn test_severity_ordering() {
        assert!(Severity::Info < Severity::Low);
        assert!(Severity::Low < Severity::Medium);
        assert!(Severity::High < Severity::Critical);
    }

    #[test]
    fn test_code_node_builder() {
        let loc = SourceLocation::new("test.rs".to_string(), 1, 10, 0, 20);
        let node = CodeNode::new(
            "my_fn".to_string(),
            NodeKind::Function,
            loc,
            Language::Rust,
            Visibility::Public,
        )
        .with_async()
        .with_test()
        .with_lines_of_code(10);

        assert!(node.is_async);
        assert!(node.is_test);
        assert_eq!(node.lines_of_code, 10);
        assert_eq!(node.full_name, "my_fn");
    }

    #[test]
    fn test_call_edge() {
        let edge = CallEdge::certain(NodeId::from_u32(1), NodeId::from_u32(2))
            .with_recursive()
            .with_call_count(3);
        assert_eq!(edge.confidence, EdgeConfidence::Certain);
        assert!(edge.is_recursive);
        assert_eq!(edge.call_count, 3);
    }

    #[test]
    fn test_finding_builder() {
        let loc = SourceLocation::new("test.py".to_string(), 5, 5, 0, 30);
        let finding = Finding::new("SEC001", "SQL Injection", Severity::Critical, loc)
            .with_description("User input flows to query")
            .with_confidence(Confidence::High);
        assert_eq!(finding.rule_id, "SEC001");
        assert_eq!(finding.severity, Severity::Critical);
        assert_eq!(finding.confidence, Confidence::High);
    }

    #[test]
    fn test_fossil_type_display() {
        assert_eq!(FossilType::Unreachable.to_string(), "unreachable");
        assert_eq!(FossilType::TestOnlyCode.to_string(), "test-only code");
    }

    #[test]
    fn test_serialization_roundtrip() {
        let lang = Language::Python;
        let json = serde_json::to_string(&lang).unwrap();
        let back: Language = serde_json::from_str(&json).unwrap();
        assert_eq!(lang, back);
    }
}
