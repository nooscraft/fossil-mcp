//! File scanner: discovers source files respecting .gitignore and exclude patterns.

use std::path::{Path, PathBuf};

use crate::core::Language;
use crate::parsers::ParserRegistry;
use ignore::WalkBuilder;

/// Discovered source file.
#[derive(Debug, Clone)]
pub struct SourceFile {
    pub path: PathBuf,
    pub language: Language,
    pub size_bytes: u64,
}

/// Discovers source files in a directory tree.
pub struct FileScanner {
    exclude_patterns: Vec<String>,
    max_file_size: u64,
    follow_symlinks: bool,
}

/// Structural markers that identify directories to skip.
/// Checked on every directory entry during the walk. If a marker file/dir
/// exists inside a directory, the entire subtree is excluded.
const VENV_MARKER: &str = "pyvenv.cfg";
const CONDA_MARKER: &str = "conda-meta";

impl FileScanner {
    pub fn new() -> Self {
        Self {
            exclude_patterns: vec![
                "node_modules".to_string(),
                ".git".to_string(),
                "target".to_string(),
                "__pycache__".to_string(),
                "vendor".to_string(),
                "dist".to_string(),
                "build".to_string(),
                ".next".to_string(),
                ".tox".to_string(),
            ],
            max_file_size: 1024 * 1024, // 1MB default
            follow_symlinks: false,
        }
    }

    pub fn with_exclude_patterns(mut self, patterns: Vec<String>) -> Self {
        self.exclude_patterns = patterns;
        self
    }

    pub fn with_max_file_size(mut self, size: u64) -> Self {
        self.max_file_size = size;
        self
    }

    pub fn with_follow_symlinks(mut self, follow: bool) -> Self {
        self.follow_symlinks = follow;
        self
    }

    /// Scan a directory for source files.
    pub fn scan(&self, root: &Path) -> Result<Vec<SourceFile>, crate::core::Error> {
        let registry = ParserRegistry::with_defaults()?;
        let mut files = Vec::new();

        let exclude = self.exclude_patterns.clone();
        let walker = WalkBuilder::new(root)
            .follow_links(self.follow_symlinks)
            .git_ignore(true)
            .git_global(true)
            .git_exclude(true)
            .hidden(true)
            // filter_entry prunes entire subtrees when it returns false for
            // a directory, so we never descend into node_modules, venvs, etc.
            .filter_entry(move |entry| {
                let path = entry.path();
                if let Some(file_name) = path.file_name().and_then(|n| n.to_str()) {
                    // Name-based exclusion (exact component match)
                    if exclude.iter().any(|p| p == file_name) {
                        return false;
                    }
                }
                // Structural detection: skip Python virtualenvs and conda envs
                // regardless of their directory name.
                if entry.file_type().is_some_and(|ft| ft.is_dir())
                    && (path.join(VENV_MARKER).exists() || path.join(CONDA_MARKER).exists())
                {
                    return false;
                }
                true
            })
            .build();

        for entry in walker {
            let entry =
                entry.map_err(|e| crate::core::Error::analysis(format!("Walk error: {e}")))?;

            let path = entry.path();

            // Skip directories
            if path.is_dir() {
                continue;
            }

            // Check file size
            let metadata = std::fs::metadata(path).ok();
            let size = metadata.as_ref().map_or(0, |m| m.len());
            if size > self.max_file_size || size == 0 {
                continue;
            }

            // Check if we have a parser for this extension
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");

            if !registry.supports_extension(ext) {
                continue;
            }

            let language = Language::from_extension(ext);
            if let Some(lang) = language {
                files.push(SourceFile {
                    path: path.to_path_buf(),
                    language: lang,
                    size_bytes: size,
                });
            }
        }

        // Sort by path for deterministic output
        files.sort_by(|a, b| a.path.cmp(&b.path));

        Ok(files)
    }

    /// Scan and group files by language.
    pub fn scan_grouped(
        &self,
        root: &Path,
    ) -> Result<std::collections::HashMap<Language, Vec<SourceFile>>, crate::core::Error> {
        let files = self.scan(root)?;
        let mut grouped = std::collections::HashMap::new();
        for file in files {
            grouped
                .entry(file.language)
                .or_insert_with(Vec::new)
                .push(file);
        }
        Ok(grouped)
    }
}

impl Default for FileScanner {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_scan_finds_python_files() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("main.py"), "def main(): pass").unwrap();
        fs::write(dir.path().join("helper.py"), "def helper(): pass").unwrap();
        fs::write(dir.path().join("README.md"), "# readme").unwrap();

        let scanner = FileScanner::new();
        let files = scanner.scan(dir.path()).unwrap();

        assert_eq!(files.len(), 2);
        assert!(files.iter().all(|f| f.language == Language::Python));
    }

    #[test]
    fn test_scan_excludes_node_modules() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("app.js"), "function app() {}").unwrap();
        let nm = dir.path().join("node_modules");
        fs::create_dir(&nm).unwrap();
        fs::write(nm.join("dep.js"), "function dep() {}").unwrap();

        let scanner = FileScanner::new();
        let files = scanner.scan(dir.path()).unwrap();

        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path.file_name().unwrap(), "app.js");
    }

    #[test]
    fn test_scan_grouped() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("main.py"), "def main(): pass").unwrap();
        fs::write(dir.path().join("app.js"), "function app() {}").unwrap();

        let scanner = FileScanner::new();
        let grouped = scanner.scan_grouped(dir.path()).unwrap();

        assert!(grouped.contains_key(&Language::Python));
        assert!(grouped.contains_key(&Language::JavaScript));
    }

    #[test]
    fn test_scan_excludes_venv_by_marker() {
        // A directory with pyvenv.cfg should be excluded regardless of name
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("app.py"), "def app(): pass").unwrap();

        let custom_env = dir.path().join("myenv");
        fs::create_dir(&custom_env).unwrap();
        // pyvenv.cfg is the structural marker for Python virtual environments
        fs::write(custom_env.join("pyvenv.cfg"), "home = /usr/bin").unwrap();
        let lib_dir = custom_env.join("lib");
        fs::create_dir(&lib_dir).unwrap();
        fs::write(lib_dir.join("helpers.py"), "def helper(): pass").unwrap();

        let scanner = FileScanner::new();
        let files = scanner.scan(dir.path()).unwrap();

        assert_eq!(
            files.len(),
            1,
            "Only app.py should be found, got: {:?}",
            files
        );
        assert_eq!(files[0].path.file_name().unwrap(), "app.py");
    }

    #[test]
    fn test_scan_excludes_conda_env_by_marker() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("main.py"), "def main(): pass").unwrap();

        let conda_env = dir.path().join("conda_env");
        fs::create_dir(&conda_env).unwrap();
        // conda-meta directory is the structural marker for conda environments
        fs::create_dir(conda_env.join("conda-meta")).unwrap();
        let lib_dir = conda_env.join("lib");
        fs::create_dir(&lib_dir).unwrap();
        fs::write(lib_dir.join("utils.py"), "def util(): pass").unwrap();

        let scanner = FileScanner::new();
        let files = scanner.scan(dir.path()).unwrap();

        assert_eq!(
            files.len(),
            1,
            "Only main.py should be found, got: {:?}",
            files
        );
    }
}
