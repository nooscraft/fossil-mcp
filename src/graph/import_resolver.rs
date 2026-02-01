//! Import path → filesystem path resolution.
//!
//! Maps import source paths (e.g., `"./utils/db"`, `"auth.middleware"`) to
//! candidate filesystem paths using the set of known parsed file paths.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::core::{Language, ParsedFile};

/// Resolves import source paths to candidate filesystem paths.
pub struct ImportResolver {
    known_files: HashSet<String>,
}

impl ImportResolver {
    /// Build a resolver from parsed files.
    pub fn new(parsed_files: &[ParsedFile]) -> Self {
        let known_files: HashSet<String> = parsed_files.iter().map(|pf| pf.path.clone()).collect();
        Self { known_files }
    }

    /// Resolve an import source path to candidate filesystem paths.
    ///
    /// Returns the actual known file paths that match the import, not the
    /// generated candidates. This ensures the returned paths match exactly
    /// what the graph stores in node locations.
    ///
    /// Returns empty vec if resolution is not supported for the language
    /// or no candidates match known files.
    pub fn resolve(
        &self,
        source_module: &str,
        importing_file: &str,
        language: Language,
    ) -> Vec<String> {
        let candidates = self.generate_candidates(source_module, importing_file, language);
        let mut results = Vec::new();
        for c in &candidates {
            if let Some(known) = self.find_known_file(c) {
                if !results.contains(&known) {
                    results.push(known);
                }
            }
        }
        results
    }

    /// Generate candidate paths for an import source, before filtering against known files.
    fn generate_candidates(
        &self,
        source_module: &str,
        importing_file: &str,
        language: Language,
    ) -> Vec<String> {
        match language {
            Language::JavaScript | Language::TypeScript => {
                self.resolve_js_ts(source_module, importing_file)
            }
            Language::Python => self.resolve_python(source_module, importing_file),
            Language::Rust => self.resolve_rust(source_module, importing_file),
            Language::Go => self.resolve_go(source_module),
            Language::Java | Language::Kotlin => self.resolve_java(source_module),
            Language::CSharp => self.resolve_csharp(source_module),
            Language::Ruby => self.resolve_ruby(source_module, importing_file),
            Language::PHP => self.resolve_php(source_module),
            _ => Vec::new(),
        }
    }

    /// JS/TS: `"./utils/db"` → relative to importer, probe extensions.
    fn resolve_js_ts(&self, source_module: &str, importing_file: &str) -> Vec<String> {
        let mut candidates = Vec::new();

        if source_module.starts_with('.') {
            // Relative import
            let importer_dir = Path::new(importing_file).parent().unwrap_or(Path::new("."));
            let base = importer_dir.join(source_module);
            let base_str = normalize_path(&base);

            // Direct file extensions
            for ext in &[".ts", ".tsx", ".js", ".jsx", ".mjs"] {
                candidates.push(format!("{}{}", base_str, ext));
            }
            // Index files in directory
            for ext in &["/index.ts", "/index.tsx", "/index.js", "/index.jsx"] {
                candidates.push(format!("{}{}", base_str, ext));
            }
        } else {
            // Package/bare import — can't resolve to a file without node_modules mapping.
            // But try as a relative-like path for monorepo-style imports.
            let base = source_module;
            for ext in &[".ts", ".tsx", ".js", ".jsx"] {
                candidates.push(format!("{}{}", base, ext));
            }
            for ext in &["/index.ts", "/index.tsx", "/index.js", "/index.jsx"] {
                candidates.push(format!("{}{}", base, ext));
            }
        }

        candidates
    }

    /// Python: `"auth.middleware"` → `auth/middleware.py` or `auth/middleware/__init__.py`.
    fn resolve_python(&self, source_module: &str, importing_file: &str) -> Vec<String> {
        let mut candidates = Vec::new();

        if source_module.starts_with('.') {
            // Relative import
            let dots = source_module.chars().take_while(|&c| c == '.').count();
            let rest = &source_module[dots..];
            let mut base_dir = Path::new(importing_file).to_path_buf();
            for _ in 0..dots {
                base_dir = base_dir.parent().unwrap_or(Path::new(".")).to_path_buf();
            }
            let module_path = rest.replace('.', "/");
            if !module_path.is_empty() {
                let base = base_dir.join(&module_path);
                let base_str = normalize_path(&base);
                candidates.push(format!("{}.py", base_str));
                candidates.push(format!("{}/__init__.py", base_str));
            }
        } else {
            let module_path = source_module.replace('.', "/");
            candidates.push(format!("{}.py", module_path));
            candidates.push(format!("{}/__init__.py", module_path));
        }

        candidates
    }

    /// Rust: `"crate::auth::middleware"` → `src/auth/middleware.rs` or `src/auth/middleware/mod.rs`.
    fn resolve_rust(&self, source_module: &str, _importing_file: &str) -> Vec<String> {
        let mut candidates = Vec::new();

        if let Some(rest) = source_module.strip_prefix("crate::") {
            let module_path = rest.replace("::", "/");
            candidates.push(format!("src/{}.rs", module_path));
            candidates.push(format!("src/{}/mod.rs", module_path));
        } else if source_module.contains("::") {
            // External crate or fully qualified — can't resolve
        }

        candidates
    }

    /// Go: `"internal/db"` → match directory suffix in known files.
    fn resolve_go(&self, source_module: &str) -> Vec<String> {
        // Go imports are packages (directories), not files.
        // Match any known file that lives under a path ending with the import path.
        let suffix = format!("/{}/", source_module);
        let suffix_end = format!("/{}", source_module);
        let mut candidates = Vec::new();

        for known in &self.known_files {
            if known.contains(&suffix) || known.ends_with(&suffix_end) {
                candidates.push(known.clone());
            }
        }

        candidates
    }

    /// Java/Kotlin: `"com.example.auth.AuthService"` → `com/example/auth/AuthService.java`.
    fn resolve_java(&self, source_module: &str) -> Vec<String> {
        let mut candidates = Vec::new();
        let path = source_module.replace('.', "/");
        candidates.push(format!("{}.java", path));
        candidates.push(format!("{}.kt", path));
        // Also try just the class name (last component)
        if let Some(class_name) = source_module.rsplit('.').next() {
            candidates.push(format!("{}.java", class_name));
            candidates.push(format!("{}.kt", class_name));
        }
        candidates
    }

    /// C#: `"MyApp.Auth.AuthService"` → `MyApp/Auth/AuthService.cs`.
    fn resolve_csharp(&self, source_module: &str) -> Vec<String> {
        let mut candidates = Vec::new();
        let path = source_module.replace('.', "/");
        candidates.push(format!("{}.cs", path));
        if let Some(class_name) = source_module.rsplit('.').next() {
            candidates.push(format!("{}.cs", class_name));
        }
        candidates
    }

    /// Ruby: `"./lib/auth"` → relative path with `.rb`.
    fn resolve_ruby(&self, source_module: &str, importing_file: &str) -> Vec<String> {
        let mut candidates = Vec::new();

        if source_module.starts_with('.') {
            let importer_dir = Path::new(importing_file).parent().unwrap_or(Path::new("."));
            let base = importer_dir.join(source_module);
            let base_str = normalize_path(&base);
            candidates.push(format!("{}.rb", base_str));
            candidates.push(base_str);
        } else {
            candidates.push(format!("{}.rb", source_module));
            candidates.push(source_module.to_string());
        }

        candidates
    }

    /// PHP: `"App\\Auth\\AuthService"` → `App/Auth/AuthService.php`.
    fn resolve_php(&self, source_module: &str) -> Vec<String> {
        let mut candidates = Vec::new();
        let path = source_module.replace('\\', "/");
        candidates.push(format!("{}.php", path));
        // Also try the last component
        if let Some(class_name) = source_module.rsplit('\\').next() {
            candidates.push(format!("{}.php", class_name));
        }
        candidates
    }

    /// Find the known file that matches a candidate path via suffix matching.
    ///
    /// Returns the original known file path (as stored in parsed files) so that
    /// it matches what the graph stores in node locations.
    ///
    /// Since parsed file paths may be absolute or relative, we use suffix matching:
    /// a known file `/home/user/project/src/auth/db.ts` matches candidate `src/auth/db.ts`.
    fn find_known_file(&self, candidate: &str) -> Option<String> {
        // Normalize the candidate
        let normalized = candidate
            .replace("\\", "/")
            .trim_start_matches("./")
            .to_string();

        for known in &self.known_files {
            let known_normalized = known.replace("\\", "/");
            if known_normalized == normalized {
                return Some(known.clone());
            }
            // Suffix match: known file ends with /candidate
            if known_normalized.ends_with(&format!("/{}", normalized)) {
                return Some(known.clone());
            }
            // Candidate ends with known (for when known is relative)
            if normalized.ends_with(&format!("/{}", known_normalized)) {
                return Some(known.clone());
            }
            // Direct suffix match
            if known_normalized.ends_with(&normalized) {
                // Ensure it's at a path boundary
                let prefix_len = known_normalized.len() - normalized.len();
                if prefix_len == 0
                    || known_normalized.as_bytes()[prefix_len - 1] == b'/'
                    || known_normalized.as_bytes()[prefix_len - 1] == b'\\'
                {
                    return Some(known.clone());
                }
            }
        }
        None
    }
}

/// Normalize a PathBuf to a string, cleaning up `./` and `..` where possible.
fn normalize_path(path: &Path) -> String {
    let mut components = Vec::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {} // skip .
            std::path::Component::ParentDir => {
                if !components.is_empty() {
                    components.pop();
                }
            }
            other => {
                components.push(other.as_os_str().to_string_lossy().to_string());
            }
        }
    }
    let result: PathBuf = components.iter().collect();
    result.to_string_lossy().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_resolver(files: &[&str]) -> ImportResolver {
        let known_files: HashSet<String> = files.iter().map(|s| s.to_string()).collect();
        ImportResolver { known_files }
    }

    // =========================================================================
    // JS/TS resolution
    // =========================================================================

    #[test]
    fn test_js_relative_import() {
        let resolver = make_resolver(&["src/utils/db.ts", "src/index.ts"]);
        let result = resolver.resolve("./utils/db", "src/index.ts", Language::TypeScript);
        assert!(
            result.contains(&"src/utils/db.ts".to_string()),
            "Should resolve ./utils/db to src/utils/db.ts, got: {:?}",
            result
        );
    }

    #[test]
    fn test_js_index_import() {
        let resolver = make_resolver(&["src/components/index.ts", "src/app.ts"]);
        let result = resolver.resolve("./components", "src/app.ts", Language::TypeScript);
        assert!(
            result.contains(&"src/components/index.ts".to_string()),
            "Should resolve ./components to src/components/index.ts, got: {:?}",
            result
        );
    }

    #[test]
    fn test_js_parent_relative_import() {
        let resolver = make_resolver(&["src/utils/helpers.ts", "src/features/auth/login.ts"]);
        let result = resolver.resolve(
            "../../utils/helpers",
            "src/features/auth/login.ts",
            Language::TypeScript,
        );
        assert!(
            result.contains(&"src/utils/helpers.ts".to_string()),
            "Should resolve ../../utils/helpers, got: {:?}",
            result
        );
    }

    #[test]
    fn test_js_no_match() {
        let resolver = make_resolver(&["src/utils/db.ts"]);
        let result = resolver.resolve("./nonexistent", "src/index.ts", Language::TypeScript);
        assert!(
            result.is_empty(),
            "Should return empty for nonexistent import"
        );
    }

    // =========================================================================
    // Python resolution
    // =========================================================================

    #[test]
    fn test_python_dotted_import() {
        let resolver = make_resolver(&["auth/middleware.py"]);
        let result = resolver.resolve("auth.middleware", "app.py", Language::Python);
        assert!(
            result.contains(&"auth/middleware.py".to_string()),
            "Should resolve auth.middleware to auth/middleware.py, got: {:?}",
            result
        );
    }

    #[test]
    fn test_python_package_import() {
        let resolver = make_resolver(&["auth/__init__.py"]);
        let result = resolver.resolve("auth", "app.py", Language::Python);
        assert!(
            result.contains(&"auth/__init__.py".to_string()),
            "Should resolve auth to auth/__init__.py, got: {:?}",
            result
        );
    }

    // =========================================================================
    // Rust resolution
    // =========================================================================

    #[test]
    fn test_rust_crate_import() {
        let resolver = make_resolver(&["src/auth/middleware.rs"]);
        let result = resolver.resolve("crate::auth::middleware", "src/main.rs", Language::Rust);
        assert!(
            result.contains(&"src/auth/middleware.rs".to_string()),
            "Should resolve crate::auth::middleware, got: {:?}",
            result
        );
    }

    #[test]
    fn test_rust_mod_rs() {
        let resolver = make_resolver(&["src/auth/mod.rs"]);
        let result = resolver.resolve("crate::auth", "src/main.rs", Language::Rust);
        assert!(
            result.contains(&"src/auth/mod.rs".to_string()),
            "Should resolve crate::auth to src/auth/mod.rs, got: {:?}",
            result
        );
    }

    // =========================================================================
    // Java resolution
    // =========================================================================

    #[test]
    fn test_java_fqn_import() {
        let resolver = make_resolver(&["com/example/auth/AuthService.java"]);
        let result = resolver.resolve(
            "com.example.auth.AuthService",
            "com/example/Main.java",
            Language::Java,
        );
        assert!(
            result.contains(&"com/example/auth/AuthService.java".to_string()),
            "Should resolve Java FQN, got: {:?}",
            result
        );
    }

    // =========================================================================
    // C# resolution
    // =========================================================================

    #[test]
    fn test_csharp_namespace_import() {
        let resolver = make_resolver(&["MyApp/Auth/AuthService.cs"]);
        let result = resolver.resolve(
            "MyApp.Auth.AuthService",
            "MyApp/Program.cs",
            Language::CSharp,
        );
        assert!(
            result.contains(&"MyApp/Auth/AuthService.cs".to_string()),
            "Should resolve C# namespace, got: {:?}",
            result
        );
    }

    // =========================================================================
    // PHP resolution
    // =========================================================================

    #[test]
    fn test_php_namespace_import() {
        let resolver = make_resolver(&["App/Auth/AuthService.php"]);
        let result = resolver.resolve("App\\Auth\\AuthService", "App/index.php", Language::PHP);
        assert!(
            result.contains(&"App/Auth/AuthService.php".to_string()),
            "Should resolve PHP namespace, got: {:?}",
            result
        );
    }

    // =========================================================================
    // Suffix matching
    // =========================================================================

    #[test]
    fn test_absolute_path_suffix_matching() {
        let resolver = make_resolver(&["/home/user/project/src/utils/db.ts"]);
        let result = resolver.resolve("./utils/db", "src/index.ts", Language::TypeScript);
        assert!(
            !result.is_empty(),
            "Should match absolute known paths via suffix, got: {:?}",
            result
        );
    }

    #[test]
    fn test_no_false_suffix_match() {
        // "db.ts" should not match "mydb.ts"
        let resolver = make_resolver(&["src/mydb.ts"]);
        let result = resolver.resolve("./db", "src/index.ts", Language::TypeScript);
        assert!(
            result.is_empty(),
            "Should not false-match mydb.ts for ./db import, got: {:?}",
            result
        );
    }
}
