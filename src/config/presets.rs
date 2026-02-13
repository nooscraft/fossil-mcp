//! Embedded preset definitions for framework-specific entry point detection.
//!
//! Each preset defines which attributes/decorators mark functions as entry points
//! for a specific framework. Presets can be auto-detected from project files.

use std::path::Path;

/// A framework detection preset.
pub struct Preset {
    /// Preset identifier (e.g. "spring", "nestjs").
    pub name: &'static str,
    /// Files whose presence triggers auto-detection.
    pub detect_files: &'static [&'static str],
    /// Package/dependency names that trigger auto-detection
    /// (checked in package.json, pom.xml, Cargo.toml, etc.).
    pub detect_deps: &'static [&'static str],
    /// Attribute patterns that mark entry points for this framework.
    pub entry_attributes: &'static [&'static str],
    /// Function name patterns that are always entry points for this framework.
    pub entry_functions: &'static [&'static str],
    /// Lifecycle methods always alive inside classes (e.g., React `render`, Vue `mounted`).
    pub lifecycle_methods: &'static [&'static str],
}

/// All available presets.
static PRESETS: &[Preset] = &[
    Preset {
        name: "rust",
        detect_files: &["Cargo.toml"],
        detect_deps: &[],
        entry_attributes: &[
            "impl_trait:",
            "derive:",
            "serde_default:",
            "serde_serialize_with:",
            "serde_deserialize_with:",
            "implements:",
        ],
        entry_functions: &[],
        lifecycle_methods: &[],
    },
    Preset {
        name: "spring",
        detect_files: &["pom.xml", "build.gradle", "build.gradle.kts"],
        detect_deps: &["spring-boot", "spring-framework", "org.springframework"],
        entry_attributes: &[
            "Bean",
            "Controller",
            "RestController",
            "Service",
            "Component",
            "Repository",
            "Configuration",
            "Scheduled",
            "PostConstruct",
            "PreDestroy",
            "RequestMapping",
            "GetMapping",
            "PostMapping",
            "PutMapping",
            "DeleteMapping",
            "PatchMapping",
            "EventListener",
            "Async",
            "Transactional",
        ],
        entry_functions: &[],
        lifecycle_methods: &[],
    },
    Preset {
        name: "nestjs",
        detect_files: &[],
        detect_deps: &["@nestjs/core", "@nestjs/common"],
        entry_attributes: &[
            "route",
            "component",
            "Controller",
            "Injectable",
            "Module",
            "Pipe",
            "Guard",
            "Interceptor",
            "Middleware",
            "Get",
            "Post",
            "Put",
            "Delete",
            "Patch",
            "Head",
            "Options",
            "All",
        ],
        entry_functions: &[],
        lifecycle_methods: &[
            "onModuleInit",
            "onModuleDestroy",
            "onApplicationBootstrap",
            "onApplicationShutdown",
        ],
    },
    Preset {
        name: "express",
        detect_files: &[],
        detect_deps: &["express"],
        entry_attributes: &["route", "handler", "endpoint"],
        entry_functions: &[],
        lifecycle_methods: &[],
    },
    Preset {
        name: "django",
        detect_files: &["manage.py"],
        detect_deps: &["django", "Django"],
        entry_attributes: &[
            "route",
            "login_required",
            "permission_required",
            "csrf_exempt",
            "require_http_methods",
        ],
        entry_functions: &[],
        lifecycle_methods: &["setUp", "tearDown", "setUpClass", "tearDownClass"],
    },
    Preset {
        name: "flask",
        detect_files: &[],
        detect_deps: &["flask", "Flask"],
        entry_attributes: &["route", "before_request", "after_request", "errorhandler"],
        entry_functions: &[],
        lifecycle_methods: &[],
    },
    Preset {
        name: "fastapi",
        detect_files: &[],
        detect_deps: &["fastapi", "FastAPI"],
        entry_attributes: &["route", "Depends"],
        entry_functions: &[],
        lifecycle_methods: &[],
    },
    Preset {
        name: "cdk",
        detect_files: &["cdk.json"],
        detect_deps: &["aws-cdk-lib", "@aws-cdk/core"],
        entry_attributes: &["component"],
        entry_functions: &[],
        lifecycle_methods: &[],
    },
    Preset {
        name: "aspnet",
        detect_files: &[],
        detect_deps: &["Microsoft.AspNetCore"],
        entry_attributes: &[
            "HttpGet",
            "HttpPost",
            "HttpPut",
            "HttpDelete",
            "ApiController",
            "Authorize",
            "AllowAnonymous",
        ],
        entry_functions: &[],
        lifecycle_methods: &[],
    },
    Preset {
        name: "lombok",
        detect_files: &[],
        detect_deps: &["lombok", "org.projectlombok"],
        entry_attributes: &[
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
        ],
        entry_functions: &[],
        lifecycle_methods: &[],
    },
    Preset {
        name: "jpa",
        detect_files: &[],
        detect_deps: &["javax.persistence", "jakarta.persistence", "hibernate"],
        entry_attributes: &[
            "Entity",
            "Table",
            "MappedSuperclass",
            "Embeddable",
            "Column",
            "Id",
            "GeneratedValue",
        ],
        entry_functions: &[],
        lifecycle_methods: &[],
    },
    // React lifecycle methods
    Preset {
        name: "react",
        detect_files: &[],
        detect_deps: &["react", "react-dom", "next"],
        entry_attributes: &[],
        entry_functions: &["App"],
        lifecycle_methods: &[
            "render",
            "componentDidMount",
            "componentDidUpdate",
            "componentWillUnmount",
            "componentDidCatch",
            "getDerivedStateFromError",
            "shouldComponentUpdate",
            "getSnapshotBeforeUpdate",
            "getStaticProps",
            "getServerSideProps",
            "getStaticPaths",
        ],
    },
    // Vue lifecycle hooks
    Preset {
        name: "vue",
        detect_files: &[],
        detect_deps: &["vue", "nuxt"],
        entry_attributes: &[],
        entry_functions: &[],
        lifecycle_methods: &[
            "mounted",
            "created",
            "beforeDestroy",
            "destroyed",
            "beforeMount",
            "beforeCreate",
            "updated",
            "beforeUpdate",
            "activated",
            "deactivated",
            "setup",
        ],
    },
    // Angular lifecycle hooks
    Preset {
        name: "angular",
        detect_files: &["angular.json"],
        detect_deps: &["@angular/core"],
        entry_attributes: &["Component", "Injectable", "NgModule", "Directive", "Pipe"],
        entry_functions: &[],
        lifecycle_methods: &[
            "ngOnInit",
            "ngOnDestroy",
            "ngOnChanges",
            "ngAfterViewInit",
            "ngAfterContentInit",
            "ngAfterViewChecked",
            "ngAfterContentChecked",
            "ngDoCheck",
        ],
    },
    // Rust Axum/Actix/Rocket web frameworks
    Preset {
        name: "axum",
        detect_files: &[],
        detect_deps: &["axum"],
        entry_attributes: &["tokio::main", "debug_handler"],
        entry_functions: &["router", "app"],
        lifecycle_methods: &[],
    },
    Preset {
        name: "actix",
        detect_files: &[],
        detect_deps: &["actix-web"],
        entry_attributes: &["actix_web::main", "get", "post", "put", "delete"],
        entry_functions: &[],
        lifecycle_methods: &[],
    },
    // Rust benchmarking framework
    Preset {
        name: "criterion",
        detect_files: &[],
        detect_deps: &["criterion"],
        entry_attributes: &["bench"],
        entry_functions: &["criterion_main", "criterion_group"],
        lifecycle_methods: &[],
    },
    // Python FFI bindings framework
    Preset {
        name: "pyo3",
        detect_files: &[],
        detect_deps: &["pyo3"],
        entry_attributes: &["pymethods", "pyfunction", "pyclass"],
        entry_functions: &[],
        lifecycle_methods: &[],
    },
];

/// Look up a preset by name.
pub fn get_preset(name: &str) -> Option<&'static Preset> {
    PRESETS.iter().find(|p| p.name == name)
}

/// Auto-detect which presets apply to a project.
pub fn auto_detect_presets(root: &Path) -> Vec<String> {
    let mut active = Vec::new();

    for preset in PRESETS {
        // Check for marker files
        for detect_file in preset.detect_files {
            if root.join(detect_file).exists() {
                active.push(preset.name.to_string());
                break;
            }
        }
        if active.last().map(|s| s.as_str()) == Some(preset.name) {
            continue;
        }

        // Check package.json for JS/TS deps
        if !preset.detect_deps.is_empty() {
            if let Some(deps) = read_package_json_deps(root) {
                if preset
                    .detect_deps
                    .iter()
                    .any(|d| deps.contains(&d.to_string()))
                {
                    active.push(preset.name.to_string());
                    continue;
                }
            }
        }
    }

    active
}

/// Read dependency names from package.json (if it exists).
fn read_package_json_deps(root: &Path) -> Option<Vec<String>> {
    let pkg_path = root.join("package.json");
    let content = std::fs::read_to_string(pkg_path).ok()?;
    let json: serde_json::Value = serde_json::from_str(&content).ok()?;

    let mut deps = Vec::new();
    for key in &["dependencies", "devDependencies", "peerDependencies"] {
        if let Some(obj) = json.get(key).and_then(|v| v.as_object()) {
            for dep_name in obj.keys() {
                deps.push(dep_name.clone());
            }
        }
    }
    Some(deps)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_preset_spring() {
        let preset = get_preset("spring");
        assert!(preset.is_some());
        assert!(preset.unwrap().entry_attributes.contains(&"Bean"));
    }

    #[test]
    fn test_get_preset_unknown() {
        assert!(get_preset("nonexistent").is_none());
    }

    #[test]
    fn test_auto_detect_rust() {
        use std::io::Write;
        let dir = tempfile::TempDir::new().unwrap();
        let cargo_path = dir.path().join("Cargo.toml");
        let mut f = std::fs::File::create(&cargo_path).unwrap();
        writeln!(f, "[package]\nname = \"test\"").unwrap();

        let presets = auto_detect_presets(dir.path());
        assert!(
            presets.contains(&"rust".to_string()),
            "Should detect rust preset, got: {:?}",
            presets
        );
    }

    #[test]
    fn test_auto_detect_nestjs() {
        use std::io::Write;
        let dir = tempfile::TempDir::new().unwrap();
        let pkg_path = dir.path().join("package.json");
        let mut f = std::fs::File::create(&pkg_path).unwrap();
        writeln!(f, r#"{{"dependencies": {{"@nestjs/core": "^10.0.0"}}}}"#).unwrap();

        let presets = auto_detect_presets(dir.path());
        assert!(
            presets.contains(&"nestjs".to_string()),
            "Should detect nestjs preset, got: {:?}",
            presets
        );
    }
}
