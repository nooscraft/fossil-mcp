//! BDD-based context-sensitive dead code detection.
//!
//! Behavior-Driven Detection (BDD) uses common behavioral patterns to identify
//! functions that appear dead but are actually alive due to:
//! - Callback registration (setTimeout, addEventListener, etc.)
//! - Dynamic dispatch (reflection, plugin systems, factories)
//! - Middleware/decorator patterns
//! - Setup/teardown lifecycle methods
//! - Configuration-driven selection
//! - Lazy initialization

#![allow(non_snake_case)]

use crate::core::CodeNode;
use regex::Regex;
use std::sync::OnceLock;

#[cfg(test)]
use crate::core::NodeKind;

/// Behavior markers that indicate a function is actually in use despite
/// appearing unreachable in static analysis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BehaviorMarker {
    /// Function is passed as callback (setTimeout, fetch, promise.then, etc.)
    CallbackHandler,
    /// Function is middleware (Express, Django, etc.)
    Middleware,
    /// Function is lifecycle method (setUp, tearDown, beforeEach, etc.)
    LifecycleMethod,
    /// Function is registered in a registry/plugin system
    PluginRegistration,
    /// Function is used via lazy loading/factory pattern
    LazyLoading,
    /// Function is exported for external consumption
    PublicExport,
    /// Function is used in dynamic dispatch (reflection, factory selection)
    DynamicDispatch,
    /// Function matches common event handler pattern
    EventHandler,
    /// Function is used as a constructor in factory
    FactoryMethod,
    /// Function is used in configuration/dependency injection
    ConfigDriven,
}

/// Detects behavior markers that indicate code is alive despite appearing dead.
pub struct BddContextDetector;

impl BddContextDetector {
    /// Check if a node has any behavior markers indicating it's actually alive.
    pub fn detect_markers(node: &CodeNode) -> Vec<BehaviorMarker> {
        let mut markers = Vec::new();

        // Check callback handler patterns
        if Self::is_callback_handler(node) {
            markers.push(BehaviorMarker::CallbackHandler);
        }

        // Check middleware patterns
        if Self::is_middleware(node) {
            markers.push(BehaviorMarker::Middleware);
        }

        // Check lifecycle methods
        if Self::is_lifecycle_method(node) {
            markers.push(BehaviorMarker::LifecycleMethod);
        }

        // Check event handler patterns
        if Self::is_event_handler(node) {
            markers.push(BehaviorMarker::EventHandler);
        }

        // Check if exported for external consumption
        if Self::is_public_export(node) {
            markers.push(BehaviorMarker::PublicExport);
        }

        // Check plugin/registry patterns
        if Self::is_plugin_registration(node) {
            markers.push(BehaviorMarker::PluginRegistration);
        }

        // Check factory patterns
        if Self::is_factory_method(node) {
            markers.push(BehaviorMarker::FactoryMethod);
        }

        // Check config-driven patterns (Zustand, Redux, serialization)
        if Self::is_config_driven(node) {
            markers.push(BehaviorMarker::ConfigDriven);
        }

        markers
    }

    /// Check if function name suggests it's a callback handler
    fn is_callback_handler(node: &CodeNode) -> bool {
        let name_lower = node.name.to_lowercase();

        // Common callback handler patterns
        CALLBACK_PATTERNS().is_match(&name_lower)
            || node
                .attributes
                .iter()
                .any(|attr| CALLBACK_ATTR_PATTERNS().is_match(attr))
    }

    /// Check if function is a middleware
    fn is_middleware(node: &CodeNode) -> bool {
        let name_lower = node.name.to_lowercase();

        MIDDLEWARE_PATTERNS().is_match(&name_lower)
            || node
                .attributes
                .iter()
                .any(|attr| MIDDLEWARE_ATTR_PATTERNS().is_match(attr))
    }

    /// Check if function is a lifecycle/setup/teardown method
    fn is_lifecycle_method(node: &CodeNode) -> bool {
        let name_lower = node.name.to_lowercase();

        LIFECYCLE_PATTERNS().is_match(&name_lower)
            || node
                .attributes
                .iter()
                .any(|attr| LIFECYCLE_ATTR_PATTERNS().is_match(attr))
    }

    /// Check if function matches event handler naming conventions
    fn is_event_handler(node: &CodeNode) -> bool {
        let name_lower = node.name.to_lowercase();

        // on* pattern (onClick, onChange, onSubmit, etc.)
        // Must have uppercase letter after 'on'
        (node.name.starts_with("on")
            && node.name.len() > 2
            && node.name.chars().nth(2).map_or(false, |c| c.is_uppercase()))
            // Lowercase DOM/SSE event handlers (onopen, onmessage, onerror, onclose, etc.)
            || matches!(
                node.name.as_str(),
                "onopen" | "onmessage" | "onerror" | "onclose" | "onabort"
                    | "onconnect" | "ondisconnect" | "ontimeout" | "ondata"
                    | "onprogress" | "onload" | "onready" | "oncomplete"
            )
            // handle* pattern (handleClick, handleChange, etc.)
            || name_lower.starts_with("handle")
            // *listener pattern (messageListener, errorListener, etc.)
            || name_lower.ends_with("listener")
            // Swift delegate method patterns (called by frameworks, not user code)
            || Self::is_swift_delegate_method(node)
            // Check attributes
            || node
                .attributes
                .iter()
                .any(|attr| EVENT_ATTR_PATTERNS().is_match(attr))
    }

    /// Swift delegate methods follow naming conventions like `Did`, `Will`, `Should`
    /// (e.g., `applicationDidFinishLaunching`, `locationManagerDidChangeAuthorization`).
    /// These are called by Apple frameworks via protocol conformance, not directly.
    fn is_swift_delegate_method(node: &CodeNode) -> bool {
        if node.language != crate::core::Language::Swift {
            return false;
        }
        let name = &node.name;
        // Cocoa delegate naming conventions: contains Did/Will/Should
        name.contains("Did")
            || name.contains("Will")
            || name.contains("Should")
            // Common delegate prefixes for specific Apple frameworks
            || name.starts_with("locationManager")
            || name.starts_with("webView")
            || name.starts_with("tableView")
            || name.starts_with("collectionView")
            || name.starts_with("photoOutput")
            || name.starts_with("audioPlayer")
            || name.starts_with("urlSession")
            || name.starts_with("mapView")
    }

    /// Check if function is exported for external use
    fn is_public_export(node: &CodeNode) -> bool {
        node.name.starts_with("export")
            || node.attributes.iter().any(|attr| {
                attr.contains("export")
                    || attr.contains("public")
                    || attr.contains("@api")
                    || attr.contains("@public")
            })
    }

    /// Check if function is registered in a plugin/registry system
    fn is_plugin_registration(node: &CodeNode) -> bool {
        let name_lower = node.name.to_lowercase();

        PLUGIN_PATTERNS().is_match(&name_lower)
            || node.attributes.iter().any(|attr| {
                PLUGIN_ATTR_PATTERNS().is_match(attr)
                    || attr.contains("register")
                    || attr.contains("plugin")
            })
    }

    /// Check if function is invoked via configuration or framework wiring
    /// (Zustand persist options, Redux middleware, serialization hooks, etc.)
    fn is_config_driven(node: &CodeNode) -> bool {
        matches!(
            node.name.as_str(),
            "migrate"
                | "serialize"
                | "deserialize"
                | "transform"
                | "validate"
                | "sanitize"
                | "comparator"
                | "reducer"
                | "partialize"
                | "onRehydrateStorage"
                | "onFinishHydration"
                | "getStorage"
                | "setStorage"
        )
    }

    /// Check if function is a factory method
    fn is_factory_method(node: &CodeNode) -> bool {
        let name_lower = node.name.to_lowercase();

        FACTORY_PATTERNS().is_match(&name_lower)
    }
}

// ============================================================================
// Compiled regex patterns (lazily initialized)
// ============================================================================

fn callback_patterns() -> &'static Regex {
    static INSTANCE: OnceLock<Regex> = OnceLock::new();
    INSTANCE.get_or_init(|| {
        Regex::new(
            r"(?ix)
            (callback|handler|onload|onsuccess|onerror|onchange|onclick|
             onsubmit|onblur|onfocus|onmouseenter|onmouseleave|
             then|catch|finally|resolve|reject)
            ",
        )
        .unwrap()
    })
}

fn callback_attr_patterns() -> &'static Regex {
    static INSTANCE: OnceLock<Regex> = OnceLock::new();
    INSTANCE.get_or_init(|| Regex::new(r"(?i)(callback|handler|listener|async|promise)").unwrap())
}

fn middleware_patterns() -> &'static Regex {
    static INSTANCE: OnceLock<Regex> = OnceLock::new();
    INSTANCE.get_or_init(|| {
        Regex::new(
            r"(?ix)
            (middleware|interceptor|filter|validator|authenticator|
             authorization|permission|check|guard|protect)
            ",
        )
        .unwrap()
    })
}

fn middleware_attr_patterns() -> &'static Regex {
    static INSTANCE: OnceLock<Regex> = OnceLock::new();
    INSTANCE.get_or_init(|| {
        Regex::new(
            r"(?i)
            (@middleware|@interceptor|@filter|@guard|@route|@post|@get|@put|@delete|
             @patch|@use)
            ",
        )
        .unwrap()
    })
}

fn lifecycle_patterns() -> &'static Regex {
    static INSTANCE: OnceLock<Regex> = OnceLock::new();
    INSTANCE.get_or_init(|| {
        Regex::new(
            r"(?ix)
            (setup|teardown|setdown|cleanup|initialize|init|mount|unmount|
             install|uninstall|enable|disable|start|stop|configure|
             beforeeach|aftereach|beforeall|afterall|before|after)
            ",
        )
        .unwrap()
    })
}

fn lifecycle_attr_patterns() -> &'static Regex {
    static INSTANCE: OnceLock<Regex> = OnceLock::new();
    INSTANCE.get_or_init(|| {
        Regex::new(
            r"(?i)
            (@setup|@teardown|@beforeeach|@aftereach|@beforeall|@afterall|
             @lifecycle|@hook)
            ",
        )
        .unwrap()
    })
}

fn event_attr_patterns() -> &'static Regex {
    static INSTANCE: OnceLock<Regex> = OnceLock::new();
    INSTANCE.get_or_init(|| Regex::new(r"(?i)(@event|@listener|@subscribe|@on|@emit)").unwrap())
}

fn plugin_patterns() -> &'static Regex {
    static INSTANCE: OnceLock<Regex> = OnceLock::new();
    INSTANCE.get_or_init(|| {
        Regex::new(
            r"(?ix)
            (plugin|extension|addon|provider|factory|builder|creator|
             register|install|use|apply)
            ",
        )
        .unwrap()
    })
}

fn plugin_attr_patterns() -> &'static Regex {
    static INSTANCE: OnceLock<Regex> = OnceLock::new();
    INSTANCE.get_or_init(|| {
        Regex::new(r"(?i)(@plugin|@provider|@injectable|@factory|@register)").unwrap()
    })
}

fn factory_patterns() -> &'static Regex {
    static INSTANCE: OnceLock<Regex> = OnceLock::new();
    INSTANCE.get_or_init(|| {
        Regex::new(
            r"(?ix)
            (create|make|build|factory|builder|constructor|new|instantiate|
             produce|generate|create_.*|make_.*)
            ",
        )
        .unwrap()
    })
}

// Re-export patterns as functions for consistency
fn CALLBACK_PATTERNS() -> &'static Regex {
    callback_patterns()
}
fn CALLBACK_ATTR_PATTERNS() -> &'static Regex {
    callback_attr_patterns()
}
fn MIDDLEWARE_PATTERNS() -> &'static Regex {
    middleware_patterns()
}
fn MIDDLEWARE_ATTR_PATTERNS() -> &'static Regex {
    middleware_attr_patterns()
}
fn LIFECYCLE_PATTERNS() -> &'static Regex {
    lifecycle_patterns()
}
fn LIFECYCLE_ATTR_PATTERNS() -> &'static Regex {
    lifecycle_attr_patterns()
}
fn EVENT_ATTR_PATTERNS() -> &'static Regex {
    event_attr_patterns()
}
fn PLUGIN_PATTERNS() -> &'static Regex {
    plugin_patterns()
}
fn PLUGIN_ATTR_PATTERNS() -> &'static Regex {
    plugin_attr_patterns()
}
fn FACTORY_PATTERNS() -> &'static Regex {
    factory_patterns()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_node(name: &str, attrs: Vec<&str>) -> CodeNode {
        CodeNode {
            id: crate::core::NodeId::from_u32(1),
            name: name.to_string(),
            full_name: format!("test.{}", name),
            kind: NodeKind::Function,
            location: crate::core::SourceLocation {
                file: "test.rs".to_string(),
                line_start: 1,
                line_end: 10,
                column_start: 0,
                column_end: 0,
            },
            language: crate::core::Language::Rust,
            visibility: crate::core::Visibility::Public,
            lines_of_code: 5,
            parent_id: None,
            is_async: false,
            is_test: false,
            is_generated: false,
            attributes: attrs.iter().map(|s| s.to_string()).collect(),
            documentation: None,
        }
    }

    #[test]
    fn test_callback_handler_detection() {
        let node = make_node("onSuccess", vec![]);
        assert!(BddContextDetector::is_callback_handler(&node));

        let node = make_node("onClick", vec![]);
        assert!(BddContextDetector::is_event_handler(&node));

        let node = make_node("thenHandler", vec![]);
        assert!(BddContextDetector::is_callback_handler(&node));
    }

    #[test]
    fn test_middleware_detection() {
        let node = make_node("authMiddleware", vec![]);
        assert!(BddContextDetector::is_middleware(&node));

        let node = make_node("validator", vec!["@middleware"]);
        assert!(BddContextDetector::is_middleware(&node));
    }

    #[test]
    fn test_lifecycle_detection() {
        let node = make_node("setUp", vec![]);
        assert!(BddContextDetector::is_lifecycle_method(&node));

        let node = make_node("beforeEach", vec![]);
        assert!(BddContextDetector::is_lifecycle_method(&node));

        let node = make_node("tearDown", vec![]);
        assert!(BddContextDetector::is_lifecycle_method(&node));
    }

    #[test]
    fn test_factory_detection() {
        let node = make_node("createUser", vec![]);
        assert!(BddContextDetector::is_factory_method(&node));

        let node = make_node("buildConfig", vec![]);
        assert!(BddContextDetector::is_factory_method(&node));
    }

    #[test]
    fn test_public_export_detection() {
        let node = make_node("exportData", vec![]);
        assert!(BddContextDetector::is_public_export(&node));

        let node = make_node("normalFunction", vec!["@api"]);
        assert!(BddContextDetector::is_public_export(&node));
    }

    #[test]
    fn test_lowercase_event_handler_detection() {
        for name in &[
            "onopen",
            "onmessage",
            "onerror",
            "onclose",
            "onload",
            "onprogress",
        ] {
            let node = make_node(name, vec![]);
            assert!(
                BddContextDetector::is_event_handler(&node),
                "'{}' should be detected as event handler",
                name
            );
        }
    }

    #[test]
    fn test_config_driven_detection() {
        for name in &[
            "migrate",
            "serialize",
            "deserialize",
            "partialize",
            "onRehydrateStorage",
            "reducer",
        ] {
            let node = make_node(name, vec![]);
            assert!(
                BddContextDetector::is_config_driven(&node),
                "'{}' should be detected as config-driven",
                name
            );
        }
    }

    #[test]
    fn test_config_driven_in_detect_markers() {
        let node = make_node("migrate", vec![]);
        let markers = BddContextDetector::detect_markers(&node);
        assert!(
            markers.contains(&BehaviorMarker::ConfigDriven),
            "migrate should have ConfigDriven marker. Markers: {:?}",
            markers
        );
    }

    fn make_swift_node(name: &str, attrs: Vec<&str>) -> CodeNode {
        CodeNode {
            id: crate::core::NodeId::from_u32(1),
            name: name.to_string(),
            full_name: format!("test.{}", name),
            kind: NodeKind::Function,
            location: crate::core::SourceLocation {
                file: "test.swift".to_string(),
                line_start: 1,
                line_end: 10,
                column_start: 0,
                column_end: 0,
            },
            language: crate::core::Language::Swift,
            visibility: crate::core::Visibility::Public,
            lines_of_code: 5,
            parent_id: None,
            is_async: false,
            is_test: false,
            is_generated: false,
            attributes: attrs.iter().map(|s| s.to_string()).collect(),
            documentation: None,
        }
    }

    #[test]
    fn test_swift_delegate_method_detection() {
        // Did/Will/Should patterns
        for name in &[
            "applicationDidFinishLaunching",
            "applicationWillTerminate",
            "applicationShouldTerminate",
            "locationManagerDidChangeAuthorization",
            "windowDidLoad",
            "scrollViewDidScroll",
        ] {
            let node = make_swift_node(name, vec![]);
            assert!(
                BddContextDetector::is_event_handler(&node),
                "Swift delegate method '{}' should be detected as event handler",
                name
            );
        }
    }

    #[test]
    fn test_swift_framework_delegate_prefixes() {
        for name in &[
            "locationManagerDidUpdateLocations",
            "webViewDidFinishNavigation",
            "tableViewDidSelectRow",
            "collectionViewDidSelectItem",
            "urlSessionDidBecomeInvalid",
            "mapViewDidChangeVisibleRegion",
        ] {
            let node = make_swift_node(name, vec![]);
            assert!(
                BddContextDetector::is_event_handler(&node),
                "Swift delegate '{}' should be detected",
                name
            );
        }
    }

    #[test]
    fn test_non_swift_did_not_delegate() {
        // "Did" in a non-Swift node should NOT trigger the Swift delegate check
        let node = make_node("applicationDidFinishLaunching", vec![]);
        // This is a Rust node (from make_node), not Swift, so it should NOT
        // match the Swift delegate pattern — but it may match other patterns
        // because "handle" is a substring. The point is that it doesn't hit
        // is_swift_delegate_method specifically.
        assert!(
            !BddContextDetector::is_swift_delegate_method(&node),
            "Rust node should NOT be detected as Swift delegate"
        );
    }

    #[test]
    fn test_multiple_markers() {
        let node = make_node("handleUserClick", vec!["@event"]);
        let markers = BddContextDetector::detect_markers(&node);
        assert!(!markers.is_empty());
    }
}
