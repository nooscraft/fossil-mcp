//! Unified configuration with TOML/YAML/JSON support and environment overrides.

pub mod cache;
pub mod presets;

use std::path::Path;

use serde::{Deserialize, Serialize};

/// Top-level Fossil configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct FossilConfig {
    pub dead_code: DeadCodeConfig,
    pub clones: ClonesConfig,
    pub security: SecurityConfig,
    pub output: OutputConfig,
    pub entry_points: EntryPointConfig,
    pub ci: CiConfig,
    pub cache: cache::CacheConfig,
}

impl FossilConfig {
    /// Load config from a file (auto-detects format from extension).
    pub fn load(path: &Path) -> Result<Self, crate::core::Error> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| crate::core::Error::config(format!("Cannot read config: {e}")))?;

        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("toml");
        match ext {
            "toml" => toml::from_str(&content)
                .map_err(|e| crate::core::Error::config(format!("TOML error: {e}"))),
            "yml" | "yaml" => serde_yaml_ng::from_str(&content)
                .map_err(|e| crate::core::Error::config(format!("YAML error: {e}"))),
            "json" => serde_json::from_str(&content)
                .map_err(|e| crate::core::Error::config(format!("JSON error: {e}"))),
            _ => Err(crate::core::Error::config(format!(
                "Unsupported config format: {ext}"
            ))),
        }
    }

    /// Try to find and load a config file from common locations.
    pub fn discover(root: &Path) -> Self {
        let candidates = [
            "fossil.toml",
            ".fossil.toml",
            "fossil.yml",
            "fossil.yaml",
            "fossil.json",
        ];

        for name in &candidates {
            let path = root.join(name);
            if path.exists() {
                if let Ok(config) = Self::load(&path) {
                    return config;
                }
            }
        }

        Self::default()
    }

    /// Apply environment variable overrides.
    pub fn apply_env_overrides(&mut self) {
        if let Ok(val) = std::env::var("FOSSIL_MIN_CONFIDENCE") {
            self.dead_code.min_confidence = val;
        }
        if let Ok(val) = std::env::var("FOSSIL_MIN_LINES") {
            if let Ok(n) = val.parse() {
                self.clones.min_lines = n;
            }
        }
        if let Ok(val) = std::env::var("FOSSIL_SIMILARITY") {
            if let Ok(n) = val.parse() {
                self.clones.similarity_threshold = n;
            }
        }
        if let Ok(val) = std::env::var("FOSSIL_MIN_SEVERITY") {
            self.security.min_severity = val;
        }
        if let Ok(val) = std::env::var("FOSSIL_OUTPUT_FORMAT") {
            self.output.format = val;
        }
        // CI-specific overrides
        if let Ok(val) = std::env::var("FOSSIL_CI_MAX_DEAD_CODE") {
            if let Ok(n) = val.parse() {
                self.ci.max_dead_code = Some(n);
            }
        }
        if let Ok(val) = std::env::var("FOSSIL_CI_MAX_CLONES") {
            if let Ok(n) = val.parse() {
                self.ci.max_clones = Some(n);
            }
        }
        if let Ok(val) = std::env::var("FOSSIL_CI_MIN_CONFIDENCE") {
            self.ci.min_confidence = Some(val);
        }
        if let Ok(val) = std::env::var("FOSSIL_CI_FAIL_ON_SCAFFOLDING") {
            self.ci.fail_on_scaffolding = Some(val.to_lowercase().parse().unwrap_or(false));
        }
        if let Ok(val) = std::env::var("FOSSIL_CI_MAX_SCAFFOLDING") {
            if let Ok(n) = val.parse() {
                self.ci.max_scaffolding = Some(n);
            }
        }
    }
}

/// Dead code detection configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DeadCodeConfig {
    pub enabled: bool,
    pub min_confidence: String,
    pub include_tests: bool,
    pub exclude: Vec<String>,
}

impl Default for DeadCodeConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            min_confidence: "low".to_string(),
            include_tests: true,
            exclude: vec!["tests/**".to_string(), "vendor/**".to_string()],
        }
    }
}

/// Clone detection configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ClonesConfig {
    pub enabled: bool,
    pub min_lines: usize,
    pub similarity_threshold: f64,
    pub types: Vec<String>,
}

impl Default for ClonesConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            min_lines: 6,
            similarity_threshold: 0.8,
            types: vec![
                "type1".to_string(),
                "type2".to_string(),
                "type3".to_string(),
            ],
        }
    }
}

/// Security scanning configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SecurityConfig {
    pub enabled: bool,
    pub rules_dir: Option<String>,
    pub min_severity: String,
    pub enable_taint: bool,
}

impl Default for SecurityConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            rules_dir: None,
            min_severity: "info".to_string(),
            enable_taint: false,
        }
    }
}

/// Output configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct OutputConfig {
    pub format: String,
    pub output_file: Option<String>,
    pub verbose: bool,
    pub quiet: bool,
}

impl Default for OutputConfig {
    fn default() -> Self {
        Self {
            format: "text".to_string(),
            output_file: None,
            verbose: false,
            quiet: false,
        }
    }
}

/// Entry point detection configuration.
///
/// Controls how Fossil identifies program entry points (main functions,
/// framework handlers, etc.) for dead code analysis.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct EntryPointConfig {
    /// Glob patterns for files that should be treated as entry points.
    pub files: Vec<String>,
    /// Function name patterns to treat as entry points.
    pub functions: Vec<String>,
    /// Attribute patterns that mark a function as an entry point.
    /// Supports exact matches ("Bean") and prefix matches ("impl_trait:").
    pub attributes: Vec<String>,
    /// Config files to scan for entry point references (Dockerfile, cdk.json).
    pub config_files: Vec<String>,
    /// Preset names to activate. If empty and `auto_detect_presets` is true,
    /// presets are auto-detected from project files.
    pub presets: Vec<String>,
    /// Whether to auto-detect presets from project files. Default: true.
    pub auto_detect_presets: bool,
}

impl Default for EntryPointConfig {
    fn default() -> Self {
        Self {
            files: Vec::new(),
            functions: Vec::new(),
            attributes: Vec::new(),
            config_files: vec![
                "Dockerfile".into(),
                "docker-compose.yml".into(),
                "package.json".into(),
            ],
            presets: Vec::new(),
            auto_detect_presets: true,
        }
    }
}

/// CI/CD mode configuration.
///
/// Controls how Fossil behaves in CI pipelines, including thresholds for failing builds
/// and diff-aware mode for PR-based analysis.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct CiConfig {
    /// Maximum number of dead code findings allowed before failing the check.
    /// None = no threshold (always pass).
    pub max_dead_code: Option<usize>,

    /// Maximum number of clone groups allowed before failing the check.
    /// None = no threshold (always pass).
    pub max_clones: Option<usize>,

    /// Minimum confidence level for counting findings (low, medium, high, certain).
    /// If set, only findings at or above this confidence are counted toward thresholds.
    /// None = count all findings.
    pub min_confidence: Option<String>,

    /// Maximum number of scaffolding artifacts allowed before failing the check.
    /// None = no threshold (always pass).
    pub max_scaffolding: Option<usize>,

    /// Whether to fail the check if any scaffolding artifacts are found.
    /// None = don't fail on scaffolding.
    pub fail_on_scaffolding: Option<bool>,
}

/// Compiled entry point rules ready for fast matching.
#[derive(Debug, Clone, Default)]
pub struct ResolvedEntryPointRules {
    /// Exact attribute matches.
    pub exact_attributes: std::collections::HashSet<String>,
    /// Prefix patterns for attribute matching (e.g. "impl_trait:").
    pub prefix_attributes: Vec<String>,
    /// Exact function name matches.
    pub exact_functions: std::collections::HashSet<String>,
}

impl ResolvedEntryPointRules {
    /// Build from an EntryPointConfig, merging preset defaults.
    pub fn from_config(config: &EntryPointConfig, project_root: Option<&Path>) -> Self {
        let mut rules = Self::with_defaults();

        // Auto-detect and apply presets
        let active_presets = if config.auto_detect_presets && config.presets.is_empty() {
            if let Some(root) = project_root {
                presets::auto_detect_presets(root)
            } else {
                Vec::new()
            }
        } else {
            config.presets.clone()
        };

        for preset_name in &active_presets {
            if let Some(preset) = presets::get_preset(preset_name) {
                for attr in preset.entry_attributes {
                    if attr.ends_with(':') || attr.ends_with('*') {
                        rules
                            .prefix_attributes
                            .push(attr.trim_end_matches('*').to_string());
                    } else {
                        rules.exact_attributes.insert(attr.to_string());
                    }
                }
                for func in preset.entry_functions {
                    rules.exact_functions.insert(func.to_string());
                }
                for method in preset.lifecycle_methods {
                    rules.exact_functions.insert(method.to_string());
                }
            }
        }

        // Apply user-specified attributes
        for attr in &config.attributes {
            if attr.ends_with('*') {
                rules
                    .prefix_attributes
                    .push(attr.trim_end_matches('*').to_string());
            } else {
                rules.exact_attributes.insert(attr.clone());
            }
        }

        // Apply user-specified function names
        for func in &config.functions {
            rules.exact_functions.insert(func.clone());
        }

        rules
    }

    /// Create rules with the hardcoded defaults (backward compatible).
    pub fn with_defaults() -> Self {
        let mut rules = Self::default();

        // Prefix patterns
        for prefix in &[
            "impl_trait:",
            "derive:",
            "serde_default:",
            "serde_serialize_with:",
            "serde_deserialize_with:",
            "extends:",
            "implements:",
            "trait_default:", // Rust trait default implementations (#20)
        ] {
            rules.prefix_attributes.push(prefix.to_string());
        }

        // Exact attribute matches
        for attr in &[
            // Route/handler patterns
            "route",
            "handler",
            "api",
            "endpoint",
            // Spring (Java)
            "Bean",
            "Controller",
            "RestController",
            "Service",
            "Component",
            "Scheduled",
            "PostConstruct",
            "RequestMapping",
            // ASP.NET (C#)
            "HttpGet",
            "HttpPost",
            "HttpPut",
            "HttpDelete",
            "ApiController",
            // Python
            "dataclass",
            "attrs",
            // Java Lombok
            "Data",
            "Getter",
            "Setter",
            "Builder",
            "NoArgsConstructor",
            "AllArgsConstructor",
            "RequiredArgsConstructor",
            "Value",
            "EqualsAndHashCode",
            "ToString",
            // JPA
            "Entity",
            "Table",
            "MappedSuperclass",
            "Embeddable",
            // C# serialization
            "Serializable",
            "DataContract",
            "DataMember",
            "JsonConverter",
            "ProtoContract",
            // Kotlin
            "Parcelize",
            // TS/JS framework components (NestJS, Angular, etc.)
            "component",
            // Rust/Python FFI and benchmarking (#18, #19)
            "pymethods",
            "pyfunction",
            "pyclass",
            "bench",
            // Feature gates (#21)
            "cfg_feature",
        ] {
            rules.exact_attributes.insert(attr.to_string());
        }

        // Framework lifecycle methods — called by the runtime, never by user code
        for func in &[
            // React class lifecycle
            "componentDidMount",
            "componentDidUpdate",
            "componentWillUnmount",
            "componentDidCatch",
            "getDerivedStateFromError",
            "shouldComponentUpdate",
            "getSnapshotBeforeUpdate",
            "render",
            // Vue lifecycle
            "mounted",
            "created",
            "beforeDestroy",
            "destroyed",
            "beforeMount",
            // Angular lifecycle
            "ngOnInit",
            "ngOnDestroy",
            "ngOnChanges",
            "ngAfterViewInit",
        ] {
            rules.exact_functions.insert(func.to_string());
        }

        rules
    }

    /// Check if an attribute matches any rule.
    pub fn matches_attribute(&self, attr: &str) -> bool {
        if self.exact_attributes.contains(attr) {
            return true;
        }
        self.prefix_attributes
            .iter()
            .any(|prefix| attr.starts_with(prefix))
    }

    /// Check if a function name matches any rule.
    pub fn matches_function(&self, name: &str) -> bool {
        self.exact_functions.contains(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = FossilConfig::default();
        assert!(config.dead_code.enabled);
        assert!(config.clones.enabled);
        assert!(config.security.enabled);
        assert_eq!(config.clones.min_lines, 6);
    }

    #[test]
    fn test_toml_roundtrip() {
        let config = FossilConfig::default();
        let toml_str = toml::to_string_pretty(&config).unwrap();
        let parsed: FossilConfig = toml::from_str(&toml_str).unwrap();
        assert_eq!(parsed.clones.min_lines, 6);
    }

    #[test]
    fn test_json_roundtrip() {
        let config = FossilConfig::default();
        let json = serde_json::to_string_pretty(&config).unwrap();
        let parsed: FossilConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.security.min_severity, "info");
    }

    #[test]
    fn test_discover_defaults() {
        let dir = std::env::temp_dir();
        let config = FossilConfig::discover(&dir);
        assert!(config.dead_code.enabled);
    }
}
