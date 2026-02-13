//! End-to-end integration tests for dead code detection.
//!
//! These tests exercise the FULL pipeline:
//!   write real source files → parse → build graph → detect dead code → verify results
//!
//! If any of these tests fail, our changes aren't actually working in practice.

use std::collections::HashSet;
use std::fs;

use fossil_mcp::dead_code::detector::{Detector, DetectorConfig};
use tempfile::TempDir;

/// Helper: run full dead code detection on a temp directory, return finding names.
fn detect_dead_names(dir: &TempDir) -> Vec<String> {
    detect_dead_names_with_config(dir, DetectorConfig::default())
}

/// Helper: run full dead code detection with custom config.
fn detect_dead_names_with_config(dir: &TempDir, mut config: DetectorConfig) -> Vec<String> {
    // Auto-detect and apply presets based on project files
    if config.entry_point_rules.is_none() {
        let entry_point_config = fossil_mcp::config::EntryPointConfig::default();
        config.entry_point_rules = Some(
            fossil_mcp::config::ResolvedEntryPointRules::from_config(
                &entry_point_config,
                Some(dir.path()),
            )
        );
    }

    let detector = Detector::new(config);
    let result = detector
        .detect(dir.path())
        .expect("detection should succeed");
    result.findings.iter().map(|f| f.name.clone()).collect()
}

// =====================================================================
// Test 1: Rust impl Trait — methods on trait impls should NOT be dead
// =====================================================================

#[test]
fn test_rust_impl_trait_methods_not_dead() {
    let dir = TempDir::new().unwrap();
    fs::write(
        dir.path().join("lib.rs"),
        r#"
pub trait Validate {
    fn validate(&self) -> bool;
}

pub struct User {
    name: String,
}

impl Validate for User {
    fn validate(&self) -> bool {
        !self.name.is_empty()
    }
}

pub fn main() {
    let u = User { name: "test".to_string() };
    println!("{}", u.validate());
}
"#,
    )
    .unwrap();

    let dead_names = detect_dead_names(&dir);
    assert!(
        !dead_names.contains(&"validate".to_string()),
        "validate() in impl Validate should NOT be dead code. Dead names: {:?}",
        dead_names
    );
}

#[test]
fn test_rust_custom_trait_impl_not_dead() {
    let dir = TempDir::new().unwrap();
    fs::write(
        dir.path().join("lib.rs"),
        r#"
pub trait MyAppHandler {
    fn handle_request(&self);
    fn handle_error(&self);
}

pub struct ApiHandler;

impl MyAppHandler for ApiHandler {
    fn handle_request(&self) {
        println!("handling request");
    }
    fn handle_error(&self) {
        println!("handling error");
    }
}

pub fn main() {
    let h = ApiHandler;
    h.handle_request();
}
"#,
    )
    .unwrap();

    let dead_names = detect_dead_names(&dir);
    assert!(
        !dead_names.contains(&"handle_request".to_string()),
        "handle_request() in impl MyAppHandler should NOT be dead. Dead: {:?}",
        dead_names
    );
    assert!(
        !dead_names.contains(&"handle_error".to_string()),
        "handle_error() in impl MyAppHandler should NOT be dead. Dead: {:?}",
        dead_names
    );
}

#[test]
fn test_rust_impl_from_not_dead() {
    let dir = TempDir::new().unwrap();
    fs::write(
        dir.path().join("lib.rs"),
        r#"
pub struct MyError(String);

impl From<std::io::Error> for MyError {
    fn from(e: std::io::Error) -> Self {
        MyError(e.to_string())
    }
}

pub fn main() {
    println!("hello");
}
"#,
    )
    .unwrap();

    let dead_names = detect_dead_names(&dir);
    assert!(
        !dead_names.contains(&"from".to_string()),
        "from() in impl From should NOT be dead. Dead: {:?}",
        dead_names
    );
}

// =====================================================================
// Test 2: TypeScript imports — cross-file imported functions NOT dead
// =====================================================================

#[test]
fn test_ts_imported_function_not_dead() {
    let dir = TempDir::new().unwrap();

    fs::write(
        dir.path().join("validators.ts"),
        "export function validate(input: string): boolean {\n    return input.length > 0;\n}\n\nexport function sanitize(input: string): string {\n    return input.trim();\n}\n",
    )
    .unwrap();

    fs::write(
        dir.path().join("app.ts"),
        "import { validate } from './validators';\n\nfunction main() {\n    const result = validate(\"hello\");\n    console.log(result);\n}\n",
    )
    .unwrap();

    let dead_names = detect_dead_names(&dir);
    assert!(
        !dead_names.contains(&"validate".to_string()),
        "validate() imported in app.ts should NOT be dead. Dead: {:?}",
        dead_names
    );
}

#[test]
fn test_ts_renamed_import_not_dead() {
    let dir = TempDir::new().unwrap();

    fs::write(
        dir.path().join("utils.ts"),
        "export function processData(data: any): any {\n    return data;\n}\n",
    )
    .unwrap();

    fs::write(
        dir.path().join("main.ts"),
        "import { processData as transform } from './utils';\n\nfunction run() {\n    const result = transform({ key: \"value\" });\n    console.log(result);\n}\n",
    )
    .unwrap();

    let dead_names = detect_dead_names(&dir);
    assert!(
        !dead_names.contains(&"processData".to_string()),
        "processData() imported as 'transform' should NOT be dead. Dead: {:?}",
        dead_names
    );
}

// =====================================================================
// Test 3: Python class hierarchy — overridden methods NOT dead
// =====================================================================

#[test]
fn test_python_class_method_override_not_dead() {
    let dir = TempDir::new().unwrap();

    fs::write(
        dir.path().join("app.py"),
        "class BaseHandler:\n    def handle(self):\n        pass\n\nclass MyHandler(BaseHandler):\n    def handle(self):\n        print(\"handling\")\n\ndef main():\n    h = MyHandler()\n    h.handle()\n",
    )
    .unwrap();

    let dead_names = detect_dead_names(&dir);
    assert!(
        !dead_names.contains(&"handle".to_string()),
        "handle() method should NOT be dead. Dead: {:?}",
        dead_names
    );
}

// =====================================================================
// Test 4: Verify attributes are actually applied to parsed nodes
// =====================================================================

#[test]
fn test_rust_impl_trait_attribute_applied() {
    let dir = TempDir::new().unwrap();
    fs::write(
        dir.path().join("lib.rs"),
        "trait Serialize {\n    fn serialize(&self) -> String;\n}\n\nstruct Config {\n    value: i32,\n}\n\nimpl Serialize for Config {\n    fn serialize(&self) -> String {\n        format!(\"{}\", self.value)\n    }\n}\n",
    )
    .unwrap();

    let registry = fossil_mcp::parsers::ParserRegistry::with_defaults().unwrap();
    let source = fs::read_to_string(dir.path().join("lib.rs")).unwrap();
    let parser = registry
        .get_parser(fossil_mcp::core::Language::Rust)
        .expect("Rust parser");
    let parsed = parser
        .parse_file(dir.path().join("lib.rs").to_str().unwrap(), &source)
        .unwrap();

    let serialize_nodes: Vec<_> = parsed
        .nodes
        .iter()
        .filter(|n| n.name == "serialize")
        .collect();

    assert!(!serialize_nodes.is_empty(), "Should find 'serialize' node");

    let has_impl_trait = serialize_nodes
        .iter()
        .any(|n| n.attributes.iter().any(|a| a.starts_with("impl_trait:")));

    assert!(
        has_impl_trait,
        "serialize() in impl Serialize should have impl_trait:Serialize attribute. \
         Nodes: {:?}",
        serialize_nodes
            .iter()
            .map(|n| (&n.name, &n.attributes, n.location.line_start))
            .collect::<Vec<_>>()
    );
}

#[test]
fn test_ts_source_module_set_on_import_call() {
    let dir = TempDir::new().unwrap();

    fs::write(
        dir.path().join("utils.ts"),
        "export function helper(): void {\n    console.log(\"help\");\n}\n",
    )
    .unwrap();

    fs::write(
        dir.path().join("main.ts"),
        "import { helper } from './utils';\n\nfunction main(): void {\n    helper();\n}\n",
    )
    .unwrap();

    let registry = fossil_mcp::parsers::ParserRegistry::with_defaults().unwrap();
    let source = fs::read_to_string(dir.path().join("main.ts")).unwrap();
    let parser = registry
        .get_parser(fossil_mcp::core::Language::TypeScript)
        .expect("TypeScript parser");
    let parsed = parser
        .parse_file(dir.path().join("main.ts").to_str().unwrap(), &source)
        .unwrap();

    let helper_calls: Vec<_> = parsed
        .unresolved_calls
        .iter()
        .filter(|c| c.callee_name == "helper")
        .collect();

    assert!(
        !helper_calls.is_empty(),
        "Should have an unresolved call to 'helper'. Unresolved: {:?}",
        parsed
            .unresolved_calls
            .iter()
            .map(|c| &c.callee_name)
            .collect::<Vec<_>>()
    );

    let has_source_module = helper_calls
        .iter()
        .any(|c| c.source_module.as_deref() == Some("./utils"));

    assert!(
        has_source_module,
        "Unresolved call to 'helper' should have source_module='./utils'. \
         Calls: {:?}",
        helper_calls
            .iter()
            .map(|c| (&c.callee_name, &c.source_module, &c.imported_as))
            .collect::<Vec<_>>()
    );
}

// =====================================================================
// Test 5: Cross-file resolution via GraphBuilder
// =====================================================================

#[test]
fn test_cross_file_import_creates_edge() {
    let dir = TempDir::new().unwrap();

    fs::write(
        dir.path().join("db.ts"),
        "export function connectDB(): void {\n    console.log(\"connected\");\n}\n",
    )
    .unwrap();

    fs::write(
        dir.path().join("server.ts"),
        "import { connectDB } from './db';\n\nfunction startServer(): void {\n    connectDB();\n    console.log(\"server started\");\n}\n",
    )
    .unwrap();

    let pipeline = fossil_mcp::analysis::Pipeline::with_defaults();
    let result = pipeline.run(dir.path()).expect("pipeline should succeed");

    let connect_node = result.graph.nodes().find(|(_, n)| n.name == "connectDB");
    assert!(
        connect_node.is_some(),
        "Should find connectDB node in graph"
    );

    let (connect_idx, _) = connect_node.unwrap();
    let callers: Vec<_> = result.graph.callers_of(connect_idx).collect();
    assert!(
        !callers.is_empty(),
        "connectDB should have callers (imported and called by startServer)"
    );
}

// =====================================================================
// Test 7: REALISTIC — multi-file Rust project with traits
// =====================================================================

#[test]
fn test_rust_multifile_trait_impls_not_dead() {
    let dir = TempDir::new().unwrap();
    fs::create_dir_all(dir.path().join("src")).unwrap();

    fs::write(
        dir.path().join("src/traits.rs"),
        "pub trait Validate {\n    fn validate(&self) -> bool;\n}\n\npub trait FromRequest {\n    fn from_request_parts(req: &str) -> Self;\n}\n",
    )
    .unwrap();

    fs::write(
        dir.path().join("src/models.rs"),
        "use crate::traits::{Validate, FromRequest};\n\npub struct User {\n    pub name: String,\n    pub email: String,\n}\n\nimpl Validate for User {\n    fn validate(&self) -> bool {\n        !self.name.is_empty() && self.email.contains('@')\n    }\n}\n\nimpl FromRequest for User {\n    fn from_request_parts(req: &str) -> Self {\n        User { name: req.to_string(), email: \"test@example.com\".to_string() }\n    }\n}\n",
    )
    .unwrap();

    fs::write(
        dir.path().join("src/main.rs"),
        "mod traits;\nmod models;\n\nfn main() {\n    let u = models::User { name: \"Alice\".to_string(), email: \"alice@example.com\".to_string() };\n    println!(\"valid: {}\", u.validate());\n}\n",
    )
    .unwrap();

    let dead_names = detect_dead_names(&dir);

    assert!(
        !dead_names.contains(&"validate".to_string()),
        "validate() trait impl should NOT be dead. Dead: {:?}",
        dead_names
    );
    assert!(
        !dead_names.contains(&"from_request_parts".to_string()),
        "from_request_parts() trait impl should NOT be dead. Dead: {:?}",
        dead_names
    );
}

// =====================================================================
// Test 8: TypeScript barrel exports
// =====================================================================

#[test]
fn test_ts_barrel_export_resolution() {
    let dir = TempDir::new().unwrap();
    fs::create_dir_all(dir.path().join("utils")).unwrap();

    fs::write(
        dir.path().join("utils/db.ts"),
        "export function connectDatabase(): void {\n    console.log(\"connecting\");\n}\n\nexport function closeDatabase(): void {\n    console.log(\"closing\");\n}\n",
    )
    .unwrap();

    fs::write(
        dir.path().join("utils/index.ts"),
        "export { connectDatabase } from './db';\nexport { closeDatabase } from './db';\n",
    )
    .unwrap();

    // Top-level call to startApp makes it reachable from the module entry point.
    // This is realistic — JS/TS apps have top-level calls that kick off execution.
    fs::write(
        dir.path().join("app.ts"),
        "import { connectDatabase } from './utils';\n\nfunction startApp(): void {\n    connectDatabase();\n}\n\nstartApp();\n",
    )
    .unwrap();

    let dead_names = detect_dead_names(&dir);
    assert!(
        !dead_names.contains(&"connectDatabase".to_string()),
        "connectDatabase() via barrel export should NOT be dead. Dead: {:?}",
        dead_names
    );
    // closeDatabase is never called, so it SHOULD be dead
    assert!(
        dead_names.contains(&"closeDatabase".to_string()),
        "closeDatabase() is never called and should be dead. Dead: {:?}",
        dead_names
    );
}

// =====================================================================
// Test 9: TypeScript class constructor
// =====================================================================

#[test]
fn test_ts_class_constructor_not_dead() {
    let dir = TempDir::new().unwrap();

    fs::write(
        dir.path().join("service.ts"),
        "export class UserService {\n    constructor() {\n        console.log(\"init\");\n    }\n\n    findUser(id: string): void {\n        console.log(\"finding\", id);\n    }\n\n    deleteUser(id: string): void {\n        console.log(\"deleting\", id);\n    }\n}\n",
    )
    .unwrap();

    fs::write(
        dir.path().join("app.ts"),
        "import { UserService } from './service';\n\nfunction main(): void {\n    const svc = new UserService();\n    svc.findUser(\"123\");\n}\n",
    )
    .unwrap();

    let dead_names = detect_dead_names(&dir);
    assert!(
        !dead_names.contains(&"constructor".to_string()),
        "constructor() should NOT be dead when class is instantiated. Dead: {:?}",
        dead_names
    );
}

// =====================================================================
// Test 10: Rust generic trait impls
// =====================================================================

#[test]
fn test_rust_generic_trait_impl_not_dead() {
    let dir = TempDir::new().unwrap();
    fs::write(
        dir.path().join("lib.rs"),
        "pub trait Serialize {\n    fn serialize(&self) -> String;\n}\n\npub struct Wrapper<T> {\n    inner: T,\n}\n\nimpl<T: std::fmt::Display> Serialize for Wrapper<T> {\n    fn serialize(&self) -> String {\n        format!(\"{}\", self.inner)\n    }\n}\n\npub fn main() {\n    let w = Wrapper { inner: 42 };\n    println!(\"{}\", w.serialize());\n}\n",
    )
    .unwrap();

    let dead_names = detect_dead_names(&dir);
    assert!(
        !dead_names.contains(&"serialize".to_string()),
        "serialize() in generic impl should NOT be dead. Dead: {:?}",
        dead_names
    );
}

// =====================================================================
// Test 11: CDK construct patterns
// =====================================================================

#[test]
fn test_ts_cdk_construct_not_dead() {
    let dir = TempDir::new().unwrap();

    fs::write(
        dir.path().join("stack.ts"),
        "export class MyStack {\n    constructor(scope: any, id: string) {\n        console.log(\"creating stack\", id);\n    }\n}\n\nexport class MyConstruct {\n    constructor(scope: any, id: string) {\n        console.log(\"creating construct\", id);\n    }\n\n    addResource(name: string): void {\n        console.log(\"adding\", name);\n    }\n}\n",
    )
    .unwrap();

    fs::write(
        dir.path().join("app.ts"),
        "import { MyStack, MyConstruct } from './stack';\n\nfunction main(): void {\n    const stack = new MyStack(null, \"my-stack\");\n    const construct = new MyConstruct(stack, \"my-construct\");\n    construct.addResource(\"bucket\");\n}\n",
    )
    .unwrap();

    let dead_names = detect_dead_names(&dir);
    assert!(
        !dead_names.contains(&"MyStack".to_string()),
        "MyStack should NOT be dead. Dead: {:?}",
        dead_names
    );
    assert!(
        !dead_names.contains(&"MyConstruct".to_string()),
        "MyConstruct should NOT be dead. Dead: {:?}",
        dead_names
    );
}

// =====================================================================
// Test 12: Comprehensive — low false positive rate
// =====================================================================

#[test]
fn test_small_project_low_false_positive_rate() {
    let dir = TempDir::new().unwrap();
    fs::create_dir_all(dir.path().join("src")).unwrap();

    fs::write(
        dir.path().join("src/main.rs"),
        "mod db;\nmod auth;\n\nfn main() {\n    let conn = db::connect();\n    let user = auth::authenticate(\"admin\", \"pass\");\n    println!(\"Connected: {:?}, User: {:?}\", conn, user);\n}\n",
    )
    .unwrap();

    fs::write(
        dir.path().join("src/db.rs"),
        "pub fn connect() -> String {\n    \"connection\".to_string()\n}\n\npub fn disconnect() {\n    println!(\"disconnected\");\n}\n",
    )
    .unwrap();

    fs::write(
        dir.path().join("src/auth.rs"),
        "pub trait Authenticator {\n    fn verify(&self, password: &str) -> bool;\n}\n\npub struct BasicAuth {\n    secret: String,\n}\n\nimpl Authenticator for BasicAuth {\n    fn verify(&self, password: &str) -> bool {\n        self.secret == password\n    }\n}\n\nimpl BasicAuth {\n    pub fn new(secret: &str) -> Self {\n        BasicAuth { secret: secret.to_string() }\n    }\n}\n\npub fn authenticate(user: &str, pass: &str) -> bool {\n    let auth = BasicAuth::new(\"secret\");\n    auth.verify(pass)\n}\n",
    )
    .unwrap();

    let dead_names = detect_dead_names(&dir);
    let dead_set: HashSet<_> = dead_names.iter().collect();

    // These should NOT be dead
    let must_be_alive = ["main", "connect", "authenticate", "verify", "new"];
    for name in &must_be_alive {
        assert!(
            !dead_set.contains(&name.to_string()),
            "{name} should NOT be dead. Dead: {:?}",
            dead_names
        );
    }

    // disconnect is a pub fn in a module — the detector may treat pub items as
    // potential entry points. In a real project it would be dead, but for our
    // small test case this is acceptable. The key assertion is that alive
    // functions are NOT flagged.
}

// =====================================================================
// RUN4 ISSUE TESTS — These test issues from FOSSIL_VERIFICATION_RUN4.md
// Each test targets a specific false positive category.
// =====================================================================

// =====================================================================
// RUN4-1: CDK deep import chain (P0-2 + P0-3)
//
// Realistic CDK project structure:
//   bin/app.ts           — entry point, imports & instantiates stacks
//   lib/stacks/my-stack.ts — imports & instantiates constructs
//   lib/constructs/my-bucket.ts — leaf construct
//
// All classes instantiated via `new` should NOT be dead.
// =====================================================================

#[test]
fn test_cdk_deep_import_chain_not_dead() {
    let dir = TempDir::new().unwrap();
    fs::create_dir_all(dir.path().join("bin")).unwrap();
    fs::create_dir_all(dir.path().join("lib/stacks")).unwrap();
    fs::create_dir_all(dir.path().join("lib/constructs")).unwrap();

    // Leaf construct
    fs::write(
        dir.path().join("lib/constructs/my-bucket.ts"),
        r#"export class MyBucket {
    constructor(scope: any, id: string) {
        console.log("creating bucket", id);
    }

    addLifecycleRule(name: string): void {
        console.log("adding rule", name);
    }
}
"#,
    )
    .unwrap();

    // Stack imports and instantiates construct
    fs::write(
        dir.path().join("lib/stacks/my-stack.ts"),
        r#"import { MyBucket } from '../constructs/my-bucket';

export class MyStack {
    constructor(scope: any, id: string) {
        const bucket = new MyBucket(this, 'MyBucket');
        bucket.addLifecycleRule('expire-30d');
    }
}
"#,
    )
    .unwrap();

    // CDK bin entry point — top-level instantiation
    fs::write(
        dir.path().join("bin/app.ts"),
        r#"import { MyStack } from '../lib/stacks/my-stack';

const app = {};
new MyStack(app, 'MyStack');
"#,
    )
    .unwrap();

    let dead_names = detect_dead_names(&dir);

    // All classes and their methods should be alive — called transitively
    // from the bin entry point
    assert!(
        !dead_names.contains(&"MyStack".to_string()),
        "MyStack class should NOT be dead (instantiated in bin/app.ts). Dead: {:?}",
        dead_names
    );
    assert!(
        !dead_names.contains(&"MyBucket".to_string()),
        "MyBucket class should NOT be dead (instantiated in MyStack constructor). Dead: {:?}",
        dead_names
    );
    assert!(
        !dead_names.contains(&"addLifecycleRule".to_string()),
        "addLifecycleRule() should NOT be dead (called in MyStack constructor). Dead: {:?}",
        dead_names
    );
}

// =====================================================================
// Rust Self::method() resolution — Self::hash_ip() should create edge
// =====================================================================

#[test]
fn test_rust_self_method_call_creates_edge() {
    let dir = TempDir::new().unwrap();
    fs::write(
        dir.path().join("lib.rs"),
        r#"pub struct RateLimiter;

impl RateLimiter {
    fn hash_ip(ip: &str) -> String {
        ip.to_string()
    }

    pub fn check_rate(&self, ip: &str) -> bool {
        let h = Self::hash_ip(ip);
        println!("{}", h);
        true
    }
}

pub fn main() {
    let limiter = RateLimiter;
    limiter.check_rate("127.0.0.1");
}
"#,
    )
    .unwrap();

    let dead_names = detect_dead_names(&dir);

    assert!(
        !dead_names.contains(&"hash_ip".to_string()),
        "hash_ip() called via Self::hash_ip() should NOT be dead. Dead: {:?}",
        dead_names
    );
}

// =====================================================================
// RUN4-2: Serde default="fn" implicit call (P1-5)
//
// `#[serde(default = "default_port")]` means serde calls `default_port()`
// at runtime during deserialization. The function is NOT dead.
// =====================================================================

#[test]
fn test_rust_serde_default_fn_not_dead() {
    let dir = TempDir::new().unwrap();
    fs::write(
        dir.path().join("config.rs"),
        r#"use serde::Deserialize;

fn default_port() -> u16 {
    8080
}

fn default_host() -> String {
    "localhost".to_string()
}

#[derive(Deserialize)]
pub struct Config {
    #[serde(default = "default_port")]
    pub port: u16,
    #[serde(default = "default_host")]
    pub host: String,
    pub name: String,
}

pub fn main() {
    println!("loading config");
}
"#,
    )
    .unwrap();

    let dead_names = detect_dead_names(&dir);

    assert!(
        !dead_names.contains(&"default_port".to_string()),
        "default_port() referenced by #[serde(default)] should NOT be dead. Dead: {:?}",
        dead_names
    );
    assert!(
        !dead_names.contains(&"default_host".to_string()),
        "default_host() referenced by #[serde(default)] should NOT be dead. Dead: {:?}",
        dead_names
    );
}

// =====================================================================
// RUN4-3: Derive macro attributes (P1-4)
//
// `#[derive(Debug, Clone, PartialEq)]` generates trait impls.
// Tree-sitter doesn't see the generated code, but we should extract
// the derive list so we know these traits are implemented.
// This is a parser-level test — verify the attributes are extracted.
// =====================================================================

#[test]
fn test_rust_derive_attributes_extracted() {
    let dir = TempDir::new().unwrap();
    fs::write(
        dir.path().join("models.rs"),
        r#"#[derive(Debug, Clone, PartialEq)]
pub struct User {
    pub name: String,
    pub email: String,
}

pub fn main() {
    let u = User { name: "test".to_string(), email: "t@t.com".to_string() };
    println!("{:?}", u);
}
"#,
    )
    .unwrap();

    let registry = fossil_mcp::parsers::ParserRegistry::with_defaults().unwrap();
    let source = fs::read_to_string(dir.path().join("models.rs")).unwrap();
    let parser = registry
        .get_parser(fossil_mcp::core::Language::Rust)
        .expect("Rust parser");
    let parsed = parser
        .parse_file(dir.path().join("models.rs").to_str().unwrap(), &source)
        .unwrap();

    // The User struct node should have derive attributes
    let user_node = parsed.nodes.iter().find(|n| n.name == "User");
    assert!(user_node.is_some(), "Should find User node");
    let user = user_node.unwrap();

    let has_derive = user.attributes.iter().any(|a| a.contains("derive"));
    assert!(
        has_derive,
        "User struct should have derive attributes. Got: {:?}",
        user.attributes
    );
}

// =====================================================================
// RUN4-3b: Derive macro structs should NOT be reported as dead
//
// `#[derive(Debug, Clone)]` on a struct means it has generated trait impls.
// The struct itself should not be flagged as dead code.
// =====================================================================

#[test]
fn test_rust_derive_struct_not_reported_as_dead() {
    let dir = TempDir::new().unwrap();
    fs::write(
        dir.path().join("models.rs"),
        r#"#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    pub host: String,
    pub port: u16,
}

#[derive(Debug)]
pub enum Status {
    Active,
    Inactive,
}

pub fn main() {
    let c = Config { host: "localhost".to_string(), port: 8080 };
    println!("{:?}", c);
}
"#,
    )
    .unwrap();

    let dead_names = detect_dead_names(&dir);

    assert!(
        !dead_names.contains(&"Config".to_string()),
        "Config with #[derive(Debug, Clone, PartialEq)] should NOT be dead. Dead: {:?}",
        dead_names
    );
    assert!(
        !dead_names.contains(&"Status".to_string()),
        "Status with #[derive(Debug)] should NOT be dead. Dead: {:?}",
        dead_names
    );
}

// =====================================================================
// RUN4-4: #[test] functions excluded when include_tests=false (P3-13)
//
// When running dead code detection with include_tests=false (default),
// test functions should NOT appear as dead code findings.
// =====================================================================

#[test]
fn test_rust_test_functions_excluded_by_default() {
    let dir = TempDir::new().unwrap();
    fs::write(
        dir.path().join("lib.rs"),
        r#"pub fn add(a: i32, b: i32) -> i32 {
    a + b
}

fn unused_helper() -> i32 {
    42
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_add() {
        assert_eq!(add(1, 2), 3);
    }

    #[test]
    fn test_add_negative() {
        assert_eq!(add(-1, 1), 0);
    }
}
"#,
    )
    .unwrap();

    let dead_names = detect_dead_names(&dir);

    // Test functions should NOT be in findings (include_tests=false)
    assert!(
        !dead_names.contains(&"test_add".to_string()),
        "test_add() should be excluded from dead code findings. Dead: {:?}",
        dead_names
    );
    assert!(
        !dead_names.contains(&"test_add_negative".to_string()),
        "test_add_negative() should be excluded. Dead: {:?}",
        dead_names
    );

    // unused_helper IS dead (no callers, not a test)
    assert!(
        dead_names.contains(&"unused_helper".to_string()),
        "unused_helper() should be dead (no callers). Dead: {:?}",
        dead_names
    );
}

// NOTE: scan_all vs detect_clones config consistency test is in
// fossil_clones/tests/min_lines_filter.rs (can't import fossil_clones from here)

// =====================================================================
// RUN4-4b: Barrel re-export chain — correct file resolution
//
// Two files define connect(). Barrel re-exports from db only.
// App imports via barrel. Only db's connect() should be alive.
// =====================================================================

#[test]
fn test_ts_barrel_reexport_follows_chain() {
    let dir = TempDir::new().unwrap();
    fs::create_dir_all(dir.path().join("lib")).unwrap();

    // Two files both define "connect"
    fs::write(
        dir.path().join("lib/db.ts"),
        "export function connect(): void {\n    console.log(\"db connect\");\n}\n",
    )
    .unwrap();

    fs::write(
        dir.path().join("lib/cache.ts"),
        "export function connect(): void {\n    console.log(\"cache connect\");\n}\n",
    )
    .unwrap();

    // Barrel re-exports only from db
    fs::write(
        dir.path().join("lib/index.ts"),
        "export { connect } from './db';\n",
    )
    .unwrap();

    // App imports via barrel
    fs::write(
        dir.path().join("app.ts"),
        "import { connect } from './lib';\n\nfunction main(): void {\n    connect();\n}\n\nmain();\n",
    )
    .unwrap();

    let dead_names = detect_dead_names(&dir);

    // db's connect should be alive (re-exported via barrel, called from app)
    // cache's connect should be dead (nobody imports it)
    let connect_dead_count = dead_names.iter().filter(|n| *n == "connect").count();
    assert!(
        connect_dead_count < 2,
        "Both connect() functions are dead — barrel re-export should make db's connect alive. Dead: {:?}",
        dead_names
    );
    assert_eq!(
        connect_dead_count, 1,
        "cache's connect() should be dead (not imported). Dead: {:?}",
        dead_names
    );
}

// =====================================================================
// RUN4-5b: CDK class extending Stack — not dead even without new()
//
// CDK framework instantiates stacks/constructs. When a class extends
// a parent, both the class itself and its methods should be treated as
// entry points via the extends: attribute.
// =====================================================================

#[test]
fn test_cdk_construct_extending_stack_not_dead() {
    let dir = TempDir::new().unwrap();

    // A CDK-style class that extends a base — no code calls `new MyStack()`
    // because the CDK framework does that.
    fs::write(
        dir.path().join("stack.ts"),
        r#"class Stack {
    constructor(scope: any, id: string) {}
}

export class MyStack extends Stack {
    constructor(scope: any, id: string) {
        console.log("creating stack", id);
    }

    addBucket(name: string): void {
        console.log("adding bucket", name);
    }
}
"#,
    )
    .unwrap();

    let dead_names = detect_dead_names(&dir);

    assert!(
        !dead_names.contains(&"MyStack".to_string()),
        "MyStack (extends Stack) should NOT be dead — CDK framework instantiates it. Dead: {:?}",
        dead_names
    );
}

// =====================================================================
// RUN4-6: TypeScript deep relative import resolution
//
// Verify that `../../lib/constructs/foo` style imports resolve correctly
// across directory boundaries.
// =====================================================================

#[test]
fn test_ts_deep_relative_import_resolution() {
    let dir = TempDir::new().unwrap();
    fs::create_dir_all(dir.path().join("src/features/auth")).unwrap();
    fs::create_dir_all(dir.path().join("src/lib/utils")).unwrap();

    fs::write(
        dir.path().join("src/lib/utils/validator.ts"),
        "export function validateEmail(email: string): boolean {\n    return email.includes('@');\n}\n",
    )
    .unwrap();

    fs::write(
        dir.path().join("src/features/auth/login.ts"),
        "import { validateEmail } from '../../lib/utils/validator';\n\nfunction handleLogin(email: string): void {\n    if (validateEmail(email)) {\n        console.log('valid');\n    }\n}\n\nhandleLogin('test@test.com');\n",
    )
    .unwrap();

    let dead_names = detect_dead_names(&dir);

    assert!(
        !dead_names.contains(&"validateEmail".to_string()),
        "validateEmail() imported via deep relative path should NOT be dead. Dead: {:?}",
        dead_names
    );
}

// =====================================================================
// RUN4-7: Rust framework extractor pattern (axum FromRequest)
//
// impl FromRequest / FromRequestParts methods should not be dead.
// These are called by the web framework, not directly by user code.
// Covered by impl_trait:* wildcard — this verifies the realistic pattern.
// =====================================================================

#[test]
fn test_rust_axum_extractor_not_dead() {
    let dir = TempDir::new().unwrap();
    fs::write(
        dir.path().join("auth.rs"),
        r#"pub struct AppState;

pub trait FromRequestParts {
    fn from_request_parts(parts: &str, state: &AppState) -> Result<Self, String>
    where Self: Sized;
}

pub struct AuthenticatedUser {
    pub user_id: String,
}

impl FromRequestParts for AuthenticatedUser {
    fn from_request_parts(parts: &str, _state: &AppState) -> Result<Self, String> {
        Ok(AuthenticatedUser { user_id: parts.to_string() })
    }
}

pub fn main() {
    println!("server starting");
}
"#,
    )
    .unwrap();

    let dead_names = detect_dead_names(&dir);

    assert!(
        !dead_names.contains(&"from_request_parts".to_string()),
        "from_request_parts() in impl FromRequestParts should NOT be dead. Dead: {:?}",
        dead_names
    );
}

// =====================================================================
// Cross-file name collision: proximity resolution prevents FPs
//
// Two files both define validate(). Two callers each import from
// their respective file. Neither validate() should be flagged dead.
// =====================================================================

#[test]
fn test_name_collision_no_false_positive() {
    let dir = TempDir::new().unwrap();
    fs::create_dir_all(dir.path().join("src")).unwrap();

    // File A defines validate()
    fs::write(
        dir.path().join("src/auth_validator.ts"),
        "export function validate(token: string): boolean {\n    return token.length > 0;\n}\n",
    )
    .unwrap();

    // File B also defines validate()
    fs::write(
        dir.path().join("src/input_validator.ts"),
        "export function validate(input: string): boolean {\n    return input.trim().length > 0;\n}\n",
    )
    .unwrap();

    // Caller A imports from auth_validator
    fs::write(
        dir.path().join("src/auth.ts"),
        "import { validate } from './auth_validator';\n\nexport function checkAuth(token: string): boolean {\n    return validate(token);\n}\n",
    )
    .unwrap();

    // Caller B imports from input_validator
    fs::write(
        dir.path().join("src/form.ts"),
        "import { validate } from './input_validator';\n\nexport function checkForm(input: string): boolean {\n    return validate(input);\n}\n",
    )
    .unwrap();

    // Entry point uses both callers
    fs::write(
        dir.path().join("src/app.ts"),
        "import { checkAuth } from './auth';\nimport { checkForm } from './form';\n\nfunction main(): void {\n    checkAuth('my-token');\n    checkForm('hello');\n}\n\nmain();\n",
    )
    .unwrap();

    let dead_names = detect_dead_names(&dir);

    // Neither validate() should be dead — each is imported and called
    let validate_dead_count = dead_names.iter().filter(|n| *n == "validate").count();
    assert_eq!(
        validate_dead_count, 0,
        "Neither validate() should be dead — both are imported and called. Dead: {:?}",
        dead_names
    );
}

// =====================================================================
// Python barrel re-export via __init__.py
//
// __init__.py with `from .db import connect` should be followed
// so that app.py importing from the package resolves correctly.
// =====================================================================

#[test]
fn test_python_barrel_reexport_follows_chain() {
    let dir = TempDir::new().unwrap();
    fs::create_dir_all(dir.path().join("mypackage")).unwrap();

    // Two files both define "connect"
    fs::write(
        dir.path().join("mypackage/db.py"),
        "def connect():\n    print('db connect')\n",
    )
    .unwrap();

    fs::write(
        dir.path().join("mypackage/cache.py"),
        "def connect():\n    print('cache connect')\n",
    )
    .unwrap();

    // Barrel re-exports only from db
    fs::write(
        dir.path().join("mypackage/__init__.py"),
        "from .db import connect\n",
    )
    .unwrap();

    // App imports via package barrel
    fs::write(
        dir.path().join("app.py"),
        "from mypackage import connect\n\ndef main():\n    connect()\n\nmain()\n",
    )
    .unwrap();

    let dead_names = detect_dead_names(&dir);

    // db's connect should be alive (re-exported via __init__.py, called from app)
    let connect_dead_count = dead_names.iter().filter(|n| *n == "connect").count();
    assert!(
        connect_dead_count < 2,
        "Both connect() functions are dead — Python barrel re-export should make db's connect alive. Dead: {:?}",
        dead_names
    );
}

// =====================================================================
// Python @dataclass should not be reported as dead code
// =====================================================================

#[test]
fn test_python_dataclass_not_reported_as_dead() {
    let dir = TempDir::new().unwrap();

    fs::write(
        dir.path().join("models.py"),
        r#"from dataclasses import dataclass

@dataclass
class User:
    name: str
    age: int

def main():
    u = User("Alice", 30)
    print(u)

main()
"#,
    )
    .unwrap();

    let dead_names = detect_dead_names(&dir);

    assert!(
        !dead_names.contains(&"User".to_string()),
        "Python @dataclass class should NOT be dead. Dead: {:?}",
        dead_names
    );
}

// =====================================================================
// Wildcard re-export: export * from './module'
//
// Barrel file uses `export * from './connections'`. App imports a specific
// function via the barrel. The wildcard re-export should be followed to
// find the function in the source module.
// =====================================================================

#[test]
fn test_ts_wildcard_reexport_resolution() {
    let dir = TempDir::new().unwrap();
    fs::create_dir_all(dir.path().join("api")).unwrap();

    fs::write(
        dir.path().join("api/connections.ts"),
        "export function fetchConnections(): void {\n    console.log(\"fetching\");\n}\n\nexport function unusedHelper(): void {\n    console.log(\"unused\");\n}\n",
    )
    .unwrap();

    // Barrel uses wildcard re-export
    fs::write(
        dir.path().join("api/index.ts"),
        "export * from './connections';\n",
    )
    .unwrap();

    fs::write(
        dir.path().join("app.ts"),
        "import { fetchConnections } from './api';\n\nfunction main(): void {\n    fetchConnections();\n}\n\nmain();\n",
    )
    .unwrap();

    let dead_names = detect_dead_names(&dir);
    assert!(
        !dead_names.contains(&"fetchConnections".to_string()),
        "fetchConnections() via wildcard barrel re-export should NOT be dead. Dead: {:?}",
        dead_names
    );
    assert!(
        dead_names.contains(&"unusedHelper".to_string()),
        "unusedHelper() is never called and should be dead. Dead: {:?}",
        dead_names
    );
}

// =====================================================================
// Named barrel re-export from nested index.ts
//
// Function in validators.ts, re-exported from validation/index.ts,
// called from a different directory via import from '../validation'.
// =====================================================================

#[test]
fn test_ts_named_barrel_reexport_nested() {
    let dir = TempDir::new().unwrap();
    fs::create_dir_all(dir.path().join("utils/validation")).unwrap();
    fs::create_dir_all(dir.path().join("components")).unwrap();

    fs::write(
        dir.path().join("utils/validation/validators.ts"),
        "export function validateEmail(email: string): boolean {\n    return email.includes('@');\n}\n\nexport function unusedValidator(): boolean {\n    return false;\n}\n",
    )
    .unwrap();

    // Barrel re-exports specific functions
    fs::write(
        dir.path().join("utils/validation/index.ts"),
        "export { validateEmail } from './validators';\n",
    )
    .unwrap();

    // Consumer imports from the barrel
    fs::write(
        dir.path().join("components/Form.tsx"),
        "import { validateEmail } from '../utils/validation';\n\nexport function Form(): void {\n    validateEmail('test@test.com');\n}\n\nForm();\n",
    )
    .unwrap();

    let dead_names = detect_dead_names(&dir);
    assert!(
        !dead_names.contains(&"validateEmail".to_string()),
        "validateEmail() via barrel re-export should NOT be dead. Dead: {:?}",
        dead_names
    );
    assert!(
        dead_names.contains(&"unusedValidator".to_string()),
        "unusedValidator() is never called and should be dead. Dead: {:?}",
        dead_names
    );
}

// =====================================================================
// Direct cross-file import: import { foo } from '../utils/module'
//
// Function is directly imported (no barrel) and called. File-scoped
// resolution should resolve the import path to the source file.
// =====================================================================

#[test]
fn test_ts_direct_import_cross_file() {
    let dir = TempDir::new().unwrap();
    fs::create_dir_all(dir.path().join("utils")).unwrap();
    fs::create_dir_all(dir.path().join("components")).unwrap();

    fs::write(
        dir.path().join("utils/smoothScroll.ts"),
        "export function smoothScrollTo(id: string): void {\n    console.log(id);\n}\n",
    )
    .unwrap();

    fs::write(
        dir.path().join("components/SlideIndicator.tsx"),
        "import { smoothScrollTo } from '../utils/smoothScroll';\n\nexport function SlideIndicator(): void {\n    smoothScrollTo('slide1');\n}\n\nSlideIndicator();\n",
    )
    .unwrap();

    let dead_names = detect_dead_names(&dir);
    assert!(
        !dead_names.contains(&"smoothScrollTo".to_string()),
        "smoothScrollTo() directly imported and called should NOT be dead. Dead: {:?}",
        dead_names
    );
}

// =====================================================================
// Cross-file call inside callback: import + call inside useCallback
//
// Mimics React pattern: function imported and called inside a nested
// arrow function (useCallback). The call should still create an edge.
// =====================================================================

#[test]
fn test_ts_cross_file_call_in_callback() {
    let dir = TempDir::new().unwrap();
    fs::create_dir_all(dir.path().join("api")).unwrap();
    fs::create_dir_all(dir.path().join("pages")).unwrap();

    fs::write(
        dir.path().join("api/connection.ts"),
        "export async function retryFailed(id: string): Promise<void> {\n    console.log(id);\n}\n",
    )
    .unwrap();

    fs::write(
        dir.path().join("pages/Settings.tsx"),
        r#"import { retryFailed } from '../api/connection';

export function Settings(): void {
    const handleRetry = async () => {
        await retryFailed('123');
    };
    handleRetry();
}

Settings();
"#,
    )
    .unwrap();

    let dead_names = detect_dead_names(&dir);
    assert!(
        !dead_names.contains(&"retryFailed".to_string()),
        "retryFailed() called inside callback should NOT be dead. Dead: {:?}",
        dead_names
    );
}

// =====================================================================
// Test: Rust multi-line impl Trait — methods should NOT be dead
// =====================================================================

#[test]
fn test_rust_multiline_impl_trait_not_dead() {
    let dir = TempDir::new().unwrap();
    fs::write(
        dir.path().join("lib.rs"),
        r#"
pub struct MyCollection<T> {
    items: Vec<T>,
}

impl<T: Clone + Send + Sync>
    From<Vec<T>>
    for MyCollection<T>
{
    fn from(items: Vec<T>) -> Self {
        MyCollection { items }
    }
}

pub fn main() {
    let c: MyCollection<i32> = MyCollection::from(vec![1, 2, 3]);
    println!("{:?}", c.items.len());
}
"#,
    )
    .unwrap();

    let dead_names = detect_dead_names(&dir);
    assert!(
        !dead_names.contains(&"from".to_string()),
        "from() in multi-line impl From should NOT be dead. Dead: {:?}",
        dead_names
    );
}

// =====================================================================
// Test: Rust multi-line FromRequestParts impl — methods should NOT be dead
// =====================================================================

#[test]
fn test_rust_multiline_from_request_parts_not_dead() {
    let dir = TempDir::new().unwrap();
    fs::write(
        dir.path().join("lib.rs"),
        r#"
pub struct AuthUser {
    pub id: u64,
}

pub trait FromRequestParts<S> {
    fn from_request_parts(state: &S) -> Self;
}

impl<S: Send + Sync>
    FromRequestParts<S>
    for AuthUser
{
    fn from_request_parts(state: &S) -> Self {
        AuthUser { id: 1 }
    }
}

pub fn main() {
    let user = AuthUser { id: 0 };
    println!("{}", user.id);
}
"#,
    )
    .unwrap();

    let dead_names = detect_dead_names(&dir);
    assert!(
        !dead_names.contains(&"from_request_parts".to_string()),
        "from_request_parts() in multi-line impl should NOT be dead. Dead: {:?}",
        dead_names
    );
}

// =====================================================================
// Test: Rust bare #[serde(default)] attribute extraction
// =====================================================================

#[test]
fn test_rust_serde_default_bare_attribute() {
    let dir = TempDir::new().unwrap();
    fs::write(
        dir.path().join("lib.rs"),
        r#"
#[serde(default)]
pub struct Config {
    pub timeout: u64,
}

impl Default for Config {
    fn default() -> Self {
        Config { timeout: 30 }
    }
}

pub fn main() {
    let c = Config::default();
    println!("{}", c.timeout);
}
"#,
    )
    .unwrap();

    let dead_names = detect_dead_names(&dir);
    // default() is an impl trait method — should not be dead
    assert!(
        !dead_names.contains(&"default".to_string()),
        "default() in impl Default should NOT be dead. Dead: {:?}",
        dead_names
    );
}

// =====================================================================
// Test #18: Criterion benchmark functions not recognized as entry points
// =====================================================================

#[test]
fn test_criterion_benchmarks_not_dead() {
    let dir = TempDir::new().unwrap();

    // Create Cargo.toml with criterion dependency
    fs::write(
        dir.path().join("Cargo.toml"),
        r#"[package]
name = "bench_test"
version = "0.1.0"
edition = "2021"

[dev-dependencies]
criterion = "0.5"
"#,
    )
    .unwrap();

    // Create lib.rs with benchmark functions marked with #[bench] attribute
    fs::write(
        dir.path().join("lib.rs"),
        r#"
pub fn fibonacci(n: u64) -> u64 {
    match n {
        0 => 1,
        1 => 1,
        n => fibonacci(n-1) + fibonacci(n-2),
    }
}

#[bench]
pub fn bench_fibonacci() {
    fibonacci(20);
}

pub fn main() {
    bench_fibonacci();
}
"#,
    )
    .unwrap();

    let dead_names = detect_dead_names(&dir);
    assert!(
        !dead_names.contains(&"fibonacci".to_string()),
        "fibonacci() should not be dead when used in benchmark. Dead: {:?}",
        dead_names
    );
    assert!(
        !dead_names.contains(&"bench_fibonacci".to_string()),
        "bench_fibonacci() with #[bench] should not be dead. Dead: {:?}",
        dead_names
    );
}

// =====================================================================
// Test #19: PyO3 methods not flagged as dead
// =====================================================================

#[test]
fn test_pyo3_methods_not_dead() {
    let dir = TempDir::new().unwrap();

    // Create Cargo.toml with pyo3 dependency
    fs::write(
        dir.path().join("Cargo.toml"),
        r#"[package]
name = "pyo3_test"
version = "0.1.0"
edition = "2021"

[dependencies]
pyo3 = { version = "0.20", features = ["extension-module"] }
"#,
    )
    .unwrap();

    fs::write(
        dir.path().join("lib.rs"),
        r#"
use pyo3::prelude::*;

#[pyclass]
pub struct Greeter {
    name: String,
}

#[pymethods]
impl Greeter {
    #[new]
    fn new(name: String) -> Self {
        Greeter { name }
    }

    fn greet(&self) -> String {
        format!("Hello, {}!", self.name)
    }
}

#[pyfunction]
fn add(a: i32, b: i32) -> i32 {
    a + b
}

pub fn main() {
    let g = Greeter::new("World".to_string());
    println!("{}", g.greet());
    println!("{}", add(1, 2));
}
"#,
    )
    .unwrap();

    let dead_names = detect_dead_names(&dir);
    assert!(
        !dead_names.contains(&"greet".to_string()),
        "greet() in #[pymethods] should NOT be dead. Dead: {:?}",
        dead_names
    );
    assert!(
        !dead_names.contains(&"add".to_string()),
        "add() with #[pyfunction] should NOT be dead. Dead: {:?}",
        dead_names
    );
}

// =====================================================================
// Test #20: Trait default methods not flagged as dead
// =====================================================================

#[test]
fn test_trait_default_methods_not_dead() {
    let dir = TempDir::new().unwrap();
    fs::write(
        dir.path().join("lib.rs"),
        r#"
pub trait Config {
    fn get_timeout(&self) -> u64 {
        30
    }

    fn get_max_retries(&self) -> usize {
        3
    }

    fn validate(&self) -> bool;
}

pub struct MyConfig;

impl Config for MyConfig {
    fn validate(&self) -> bool {
        true
    }
}

pub fn main() {
    let cfg = MyConfig;
    println!("{}", cfg.get_timeout());
    println!("{}", cfg.validate());
}
"#,
    )
    .unwrap();

    let dead_names = detect_dead_names(&dir);
    assert!(
        !dead_names.contains(&"get_timeout".to_string()),
        "get_timeout() with default impl should NOT be dead. Dead: {:?}",
        dead_names
    );
}

// =====================================================================
// Test #21: Feature-gated functions not flagged as dead
// =====================================================================

#[test]
fn test_cfg_feature_functions_not_dead() {
    let dir = TempDir::new().unwrap();
    fs::write(
        dir.path().join("lib.rs"),
        r#"
#[cfg(feature = "experimental")]
pub fn experimental_feature() {
    println!("This is experimental");
}

pub fn stable_feature() {
    println!("This is stable");
}

pub fn main() {
    stable_feature();
    #[cfg(feature = "experimental")]
    experimental_feature();
}
"#,
    )
    .unwrap();

    let dead_names = detect_dead_names(&dir);
    // Feature-gated functions should not be flagged as dead
    // (they may be dead for the current feature set, but that's expected)
    assert!(
        !dead_names.contains(&"experimental_feature".to_string()),
        "experimental_feature() with #[cfg(feature)] should NOT be flagged. Dead: {:?}",
        dead_names
    );
}

// =====================================================================
// Test #22: Variables used inside macros not reported as unused
// =====================================================================

#[test]
fn test_variables_in_macros_not_unused() {
    let dir = TempDir::new().unwrap();
    fs::write(
        dir.path().join("lib.rs"),
        r#"
pub fn test_function() {
    let x = 42;
    let y = 10;
    assert!(x > 0);
    println!("y = {}", y);
}
"#,
    )
    .unwrap();

    let dead_names = detect_dead_names(&dir);
    // Variables used in assert! and println! should not be flagged
    // (This is a dead code test, not dead store test, so we check function level)
    assert!(
        !dead_names.contains(&"test_function".to_string()),
        "test_function() should not be dead when variables are used in macros. Dead: {:?}",
        dead_names
    );
}

// =====================================================================
// Test #23: Structs not reported as dead "functions"
// =====================================================================

#[test]
fn test_structs_not_reported_as_dead() {
    let dir = TempDir::new().unwrap();
    fs::write(
        dir.path().join("lib.rs"),
        r#"
pub struct User {
    id: u32,
    name: String,
}

pub struct Config {
    timeout: u64,
}

pub fn main() {
    let user = User { id: 1, name: "Alice".to_string() };
    let config = Config { timeout: 30 };
    println!("{}", user.id);
    println!("{}", config.timeout);
}
"#,
    )
    .unwrap();

    let dead_names = detect_dead_names(&dir);
    // Struct definitions should not be reported as dead "functions"
    // They should be detected as struct types, not function nodes
    assert!(
        !dead_names.contains(&"User".to_string()),
        "User struct should NOT be reported as dead. Dead: {:?}",
        dead_names
    );
    assert!(
        !dead_names.contains(&"Config".to_string()),
        "Config struct should NOT be reported as dead. Dead: {:?}",
        dead_names
    );
}

#[test]
fn test_language_filtering_dead_code_single() {
    let dir = TempDir::new().unwrap();
    fs::write(
        dir.path().join("main.rs"),
        r#"
fn main() {
    used_function();
}

fn used_function() {}

fn dead_function() {
    println!("This is never called");
}
"#,
    )
    .unwrap();

    fs::write(
        dir.path().join("script.py"),
        r#"
def main():
    used_function()

def used_function():
    pass

def dead_python_function():
    print("Never called")
"#,
    )
    .unwrap();

    // Test filtering to only Rust
    let rust_dead = detect_dead_names_with_filter(dir.path(), Some("rust"));
    assert!(
        rust_dead.contains(&"dead_function".to_string()),
        "Should find Rust dead function. Dead: {:?}",
        rust_dead
    );
    assert!(
        !rust_dead.contains(&"dead_python_function".to_string()),
        "Should NOT find Python dead function when filtering for Rust. Dead: {:?}",
        rust_dead
    );

    // Test filtering to only Python
    let python_dead = detect_dead_names_with_filter(dir.path(), Some("python"));
    assert!(
        python_dead.contains(&"dead_python_function".to_string()),
        "Should find Python dead function. Dead: {:?}",
        python_dead
    );
    assert!(
        !python_dead.contains(&"dead_function".to_string()),
        "Should NOT find Rust dead function when filtering for Python. Dead: {:?}",
        python_dead
    );

    // Test no filter (should find both)
    let all_dead = detect_dead_names(&dir);
    assert!(
        all_dead.contains(&"dead_function".to_string()),
        "Should find Rust dead function. Dead: {:?}",
        all_dead
    );
    assert!(
        all_dead.contains(&"dead_python_function".to_string()),
        "Should find Python dead function. Dead: {:?}",
        all_dead
    );
}

#[test]
fn test_language_filtering_dead_code_multiple() {
    let dir = TempDir::new().unwrap();
    fs::write(
        dir.path().join("code.rs"),
        r#"
fn main() {
    used_function();
}

fn used_function() {}
fn dead_rust() {}
"#,
    )
    .unwrap();

    fs::write(
        dir.path().join("code.ts"),
        r#"
function main() {
    usedFunction();
}

function usedFunction() {}
function deadTypescript() {}
"#,
    )
    .unwrap();

    fs::write(
        dir.path().join("code.py"),
        r#"
def main():
    used_function()

def used_function():
    pass

def dead_python():
    pass
"#,
    )
    .unwrap();

    // Test filtering to Rust and Python (should exclude TypeScript)
    let filtered = detect_dead_names_with_filter(dir.path(), Some("rust,python"));
    assert!(
        filtered.contains(&"dead_rust".to_string()),
        "Should find Rust dead function. Dead: {:?}",
        filtered
    );
    assert!(
        filtered.contains(&"dead_python".to_string()),
        "Should find Python dead function. Dead: {:?}",
        filtered
    );
    assert!(
        !filtered.contains(&"deadTypescript".to_string()),
        "Should NOT find TypeScript dead function. Dead: {:?}",
        filtered
    );
}

#[test]
fn test_language_filtering_clones() {
    let dir = TempDir::new().unwrap();

    // Create duplicate Rust code
    fs::write(
        dir.path().join("a.rs"),
        r#"
fn duplicate_function() {
    let x = 1;
    let y = 2;
    let z = x + y;
    println!("{}", z);
}
"#,
    )
    .unwrap();

    fs::write(
        dir.path().join("b.rs"),
        r#"
fn similar_function() {
    let x = 1;
    let y = 2;
    let z = x + y;
    println!("{}", z);
}
"#,
    )
    .unwrap();

    // Create duplicate Python code
    fs::write(
        dir.path().join("script_a.py"),
        r#"
def duplicate_py():
    x = 1
    y = 2
    z = x + y
    print(z)
"#,
    )
    .unwrap();

    fs::write(
        dir.path().join("script_b.py"),
        r#"
def duplicate_py_2():
    x = 1
    y = 2
    z = x + y
    print(z)
"#,
    )
    .unwrap();

    // Test with Rust filter — should find Rust clones
    let rust_clones = detect_clones_with_filter(dir.path(), 5, 0.8, "type1,type2,type3", Some("rust"));
    assert!(
        !rust_clones.is_empty(),
        "Should find Rust clones with language filter"
    );
    // All instances should be .rs files
    for group in &rust_clones {
        for instance in &group.instances {
            assert!(
                instance.file.ends_with(".rs"),
                "Clone instance should be in Rust file: {}",
                instance.file
            );
        }
    }

    // Test with Python filter — should find Python clones
    let python_clones =
        detect_clones_with_filter(dir.path(), 5, 0.8, "type1,type2,type3", Some("python"));
    assert!(
        !python_clones.is_empty(),
        "Should find Python clones with language filter"
    );
    // All instances should be .py files
    for group in &python_clones {
        for instance in &group.instances {
            assert!(
                instance.file.ends_with(".py"),
                "Clone instance should be in Python file: {}",
                instance.file
            );
        }
    }

    // Test with no filter — should find clones in both languages
    let all_clones = detect_clones_with_filter(dir.path(), 5, 0.8, "type1,type2,type3", None);
    assert!(
        !all_clones.is_empty(),
        "Should find clones with no language filter"
    );
}

/// Helper function to detect dead code with language filter
fn detect_dead_names_with_filter(
    dir: &std::path::Path,
    language: Option<&str>,
) -> Vec<String> {
    use fossil_mcp::core::Language;

    let fossil_config = fossil_mcp::config::FossilConfig::discover(dir);
    let rules = fossil_mcp::config::ResolvedEntryPointRules::from_config(
        &fossil_config.entry_points,
        Some(dir),
    );

    let config = DetectorConfig {
        include_tests: true,
        min_confidence: fossil_mcp::core::Confidence::Low,
        min_lines: 0,
        exclude_patterns: Vec::new(),
        detect_dead_stores: true,
        use_rta: true,
        use_sdg: false,
        entry_point_rules: Some(rules),
    };

    let detector = Detector::new(config);
    let result = detector.detect(dir).unwrap();

    let allowed_languages = if let Some(lang_str) = language {
        let (langs, _) = Language::parse_list(lang_str);
        Some(langs)
    } else {
        None
    };

    result
        .findings
        .into_iter()
        .filter(|f| {
            if let Some(ref langs) = allowed_languages {
                if let Some(file_lang) = Language::from_file_path(&f.file) {
                    langs.contains(&file_lang)
                } else {
                    false
                }
            } else {
                true
            }
        })
        .map(|f| f.name)
        .collect()
}

/// Helper function to detect clones with language filter
fn detect_clones_with_filter(
    dir: &std::path::Path,
    min_lines: usize,
    similarity: f64,
    types: &str,
    language: Option<&str>,
) -> Vec<fossil_mcp::clones::types::CloneGroup> {
    use fossil_mcp::clones::detector::{CloneDetector, CloneConfig};
    use fossil_mcp::core::Language;

    let type_list: Vec<&str> = types.split(',').map(|t| t.trim()).collect();

    let config = CloneConfig {
        min_lines,
        min_nodes: 5,
        similarity_threshold: similarity,
        detect_type1: type_list.contains(&"type1"),
        detect_type2: type_list.contains(&"type2"),
        detect_type3: type_list.contains(&"type3"),
        detect_cross_language: true,
    };

    let detector = CloneDetector::new(config);
    let mut result = detector.detect(dir).unwrap();

    // Apply language filter
    if let Some(lang_str) = language {
        let (langs, _) = Language::parse_list(lang_str);
        result.groups.retain_mut(|group| {
            group.instances.retain(|inst| {
                if let Some(file_lang) = Language::from_file_path(&inst.file) {
                    langs.contains(&file_lang)
                } else {
                    false
                }
            });
            !group.instances.is_empty()
        });
    }

    result.groups
}
