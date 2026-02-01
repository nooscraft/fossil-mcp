//! Integration test: min_lines parameter must filter out short clones.
//!
//! When min_lines=20, NO clone instance should have fewer than 20 lines.

use std::fs;
use tempfile::TempDir;

#[test]
fn test_min_lines_filters_short_function_clones() {
    let dir = TempDir::new().unwrap();

    // Short functions (3 lines) — should be filtered when min_lines=20
    let short_fn = "function shortFn() {\n    return 42;\n}\n";

    // Long functions (25 lines) — should pass the min_lines=20 filter
    let long_fn_body: String = (0..23)
        .map(|i| format!("    const x{i} = {i} * 2;"))
        .collect::<Vec<_>>()
        .join("\n");
    let long_fn = format!("function longFn() {{\n{long_fn_body}\n    return x0;\n}}\n");

    fs::write(dir.path().join("a.ts"), format!("{short_fn}\n{long_fn}")).unwrap();
    fs::write(dir.path().join("b.ts"), format!("{short_fn}\n{long_fn}")).unwrap();

    let config = fossil_mcp::clones::detector::CloneConfig {
        min_lines: 20,
        min_nodes: 1,
        similarity_threshold: 0.5,
        detect_type1: true,
        detect_type2: true,
        detect_type3: true,
        detect_cross_language: false,
    };

    let detector = fossil_mcp::clones::CloneDetector::new(config);

    let files: Vec<(String, String)> = vec![
        (
            dir.path().join("a.ts").to_str().unwrap().to_string(),
            fs::read_to_string(dir.path().join("a.ts")).unwrap(),
        ),
        (
            dir.path().join("b.ts").to_str().unwrap().to_string(),
            fs::read_to_string(dir.path().join("b.ts")).unwrap(),
        ),
    ];

    let result = detector.detect_in_sources(&files);

    // Every clone instance must have >= min_lines lines
    for group in &result.groups {
        for inst in &group.instances {
            let lines = inst.end_line.saturating_sub(inst.start_line) + 1;
            assert!(
                lines >= 20,
                "Clone instance should have >= 20 lines (min_lines=20), \
                 but found {lines} lines in {}:{}-{}. \
                 Clone type: {:?}, similarity: {:.2}",
                inst.file,
                inst.start_line,
                inst.end_line,
                group.clone_type,
                group.similarity,
            );
        }
    }
}

#[test]
fn test_min_lines_does_not_filter_large_clones() {
    let dir = TempDir::new().unwrap();

    // Two identical 30-line functions
    let body: String = (0..28)
        .map(|i| format!("    const val{i} = {i} + Math.random();"))
        .collect::<Vec<_>>()
        .join("\n");
    let func = format!("function bigFunc() {{\n{body}\n    return val0;\n}}\n");

    fs::write(dir.path().join("x.ts"), &func).unwrap();
    fs::write(dir.path().join("y.ts"), &func).unwrap();

    let config = fossil_mcp::clones::detector::CloneConfig {
        min_lines: 20,
        min_nodes: 1,
        similarity_threshold: 0.5,
        detect_type1: true,
        detect_type2: true,
        detect_type3: true,
        detect_cross_language: false,
    };

    let detector = fossil_mcp::clones::CloneDetector::new(config);
    let files: Vec<(String, String)> = vec![
        (
            dir.path().join("x.ts").to_str().unwrap().to_string(),
            fs::read_to_string(dir.path().join("x.ts")).unwrap(),
        ),
        (
            dir.path().join("y.ts").to_str().unwrap().to_string(),
            fs::read_to_string(dir.path().join("y.ts")).unwrap(),
        ),
    ];

    let result = detector.detect_in_sources(&files);

    // Should find at least one clone group for the 30-line functions
    assert!(
        !result.groups.is_empty(),
        "Should detect clones for 30-line identical functions with min_lines=20"
    );
}

// =====================================================================
// RUN4-E: Trivial getter clones should be filtered out
//
// Many 3-line getter functions (return this.field) across files should
// NOT produce clone groups — they're trivial boilerplate.
// =====================================================================

#[test]
fn test_trivial_getter_clones_filtered() {
    let dir = TempDir::new().unwrap();

    // Many trivial 3-line getter functions across two files
    let source_a = r#"
function getName() {
    return this.name;
}

function getAge() {
    return this.age;
}

function getEmail() {
    return this.email;
}

function getPhone() {
    return this.phone;
}
"#;

    let source_b = r#"
function getName() {
    return this.name;
}

function getAge() {
    return this.age;
}

function getEmail() {
    return this.email;
}

function getPhone() {
    return this.phone;
}
"#;

    fs::write(dir.path().join("a.js"), source_a).unwrap();
    fs::write(dir.path().join("b.js"), source_b).unwrap();

    let config = fossil_mcp::clones::detector::CloneConfig {
        min_lines: 3,
        min_nodes: 1,
        similarity_threshold: 0.5,
        detect_type1: true,
        detect_type2: true,
        detect_type3: true,
        detect_cross_language: false,
    };

    let detector = fossil_mcp::clones::CloneDetector::new(config);
    let files: Vec<(String, String)> = vec![
        (
            dir.path().join("a.js").to_str().unwrap().to_string(),
            fs::read_to_string(dir.path().join("a.js")).unwrap(),
        ),
        (
            dir.path().join("b.js").to_str().unwrap().to_string(),
            fs::read_to_string(dir.path().join("b.js")).unwrap(),
        ),
    ];

    let result = detector.detect_in_sources(&files);

    // Trivial getter clones (3 lines, single return statement) should be filtered
    for group in &result.groups {
        let all_trivial = group
            .instances
            .iter()
            .all(|inst| inst.end_line.saturating_sub(inst.start_line) < 5);
        assert!(
            !all_trivial,
            "Trivial getter clone group should be filtered out. Got group with instances: {:?}",
            group
                .instances
                .iter()
                .map(|i| (&i.file, i.start_line, i.end_line))
                .collect::<Vec<_>>()
        );
    }
}

// =====================================================================
// RUN4-5: scan_all vs detect_clones config consistency
//
// scan_all uses CloneDetector::with_defaults() which should match the
// detect_clones defaults (similarity_threshold=0.8, min_lines=6).
// =====================================================================

#[test]
fn test_clone_defaults_match_detect_clones_tool() {
    let default_config = fossil_mcp::clones::detector::CloneConfig::default();

    assert!(
        (default_config.similarity_threshold - 0.8).abs() < f64::EPSILON,
        "Default CloneConfig similarity_threshold should be 0.8 to match detect_clones tool. \
         Got: {}. scan_all uses with_defaults() which must agree with detect_clones defaults.",
        default_config.similarity_threshold
    );

    assert_eq!(
        default_config.min_lines, 6,
        "Default CloneConfig min_lines should be 6 to match detect_clones tool. Got: {}",
        default_config.min_lines
    );
}
