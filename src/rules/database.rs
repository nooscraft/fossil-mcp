//! In-memory rule database with built-in security rules.

use std::collections::HashMap;

use crate::core::{Confidence, Language, PatternType, Rule, Severity};

/// In-memory database of security rules.
#[derive(Debug)]
pub struct RuleDatabase {
    rules: Vec<Rule>,
    by_id: HashMap<String, usize>,
    by_language: HashMap<Language, Vec<usize>>,
}

impl RuleDatabase {
    pub fn new() -> Self {
        Self {
            rules: Vec::new(),
            by_id: HashMap::new(),
            by_language: HashMap::new(),
        }
    }

    /// Create a database with built-in rules.
    pub fn with_defaults() -> Self {
        let mut db = Self::new();
        db.add_builtin_rules();
        db
    }

    /// Add a rule to the database.
    pub fn add_rule(&mut self, rule: Rule) {
        let idx = self.rules.len();
        self.by_id.insert(rule.id.clone(), idx);
        for lang in &rule.languages {
            self.by_language.entry(*lang).or_default().push(idx);
        }
        self.rules.push(rule);
    }

    /// Get a rule by ID.
    pub fn get_rule(&self, id: &str) -> Option<&Rule> {
        self.by_id.get(id).map(|&idx| &self.rules[idx])
    }

    /// Get all rules for a language.
    pub fn rules_for_language(&self, language: Language) -> Vec<&Rule> {
        self.by_language
            .get(&language)
            .map(|indices| indices.iter().map(|&idx| &self.rules[idx]).collect())
            .unwrap_or_default()
    }

    /// Get all rules.
    pub fn all_rules(&self) -> &[Rule] {
        &self.rules
    }

    /// Get all enabled rules.
    pub fn enabled_rules(&self) -> Vec<&Rule> {
        self.rules.iter().filter(|r| r.enabled).collect()
    }

    /// Total number of rules.
    pub fn len(&self) -> usize {
        self.rules.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    /// Load external rules from a directory (supports both Fossil and Semgrep YAML formats).
    ///
    /// Returns the number of rules loaded.
    pub fn load_external_rules(
        &mut self,
        dir: &std::path::Path,
    ) -> Result<usize, crate::core::Error> {
        let rules = crate::rules::loader::RuleLoader::load_from_dir(dir)?;
        let count = rules.len();
        for rule in rules {
            self.add_rule(rule);
        }
        Ok(count)
    }

    fn add_builtin_rules(&mut self) {
        // SQL Injection
        self.add_rule(Rule {
            id: "SEC001".to_string(),
            name: "SQL Injection".to_string(),
            description: "User input used directly in SQL query".to_string(),
            severity: Severity::Critical,
            confidence: Confidence::High,
            languages: vec![
                Language::Python,
                Language::JavaScript,
                Language::Java,
                Language::PHP,
                Language::Ruby,
                Language::Go,
            ],
            pattern: r#"execute\s*\(.*["'].*%|\.query\s*\(.*["'].*%|\.query\s*\(.*format|execute\s*\(.*format|["'].*(?:SELECT|INSERT|UPDATE|DELETE|DROP|ALTER)\b.*["'].*%|["'].*(?:SELECT|INSERT|UPDATE|DELETE|DROP|ALTER)\b.*\{.*\}|["'].*(?:SELECT|INSERT|UPDATE|DELETE|DROP|ALTER)\b.*["']\s*[+.]|Sprintf\s*\(\s*["'].*(?:SELECT|INSERT|UPDATE|DELETE|DROP|ALTER)\b|\.QueryRow\s*\(.*Sprintf|\.Query\s*\(.*Sprintf|\.Exec\s*\(.*Sprintf"#.to_string(),
            pattern_type: PatternType::Regex,
            cwe: Some("CWE-89".to_string()),
            owasp: Some("A03:2021".to_string()),
            fix_suggestion: Some(
                "Use parameterized queries instead of string formatting".to_string(),
            ),
            tags: vec!["injection".to_string(), "sql".to_string()],
            enabled: true,
            cvss_score: None,
            cve_references: Vec::new(),
        });

        // XSS
        self.add_rule(Rule {
            id: "SEC002".to_string(),
            name: "Cross-Site Scripting (XSS)".to_string(),
            description: "User input rendered without escaping".to_string(),
            severity: Severity::High,
            confidence: Confidence::Medium,
            languages: vec![Language::JavaScript, Language::TypeScript, Language::PHP],
            pattern: r"innerHTML\s*=|document\.write\s*\(|\.html\s*\(".to_string(),
            pattern_type: PatternType::Regex,
            cwe: Some("CWE-79".to_string()),
            owasp: Some("A03:2021".to_string()),
            fix_suggestion: Some(
                "Use textContent or a template engine with auto-escaping".to_string(),
            ),
            tags: vec!["xss".to_string(), "injection".to_string()],
            enabled: true,
            cvss_score: None,
            cve_references: Vec::new(),
        });

        // Command Injection
        self.add_rule(Rule {
            id: "SEC003".to_string(),
            name: "Command Injection".to_string(),
            description: "User input passed to system command execution".to_string(),
            severity: Severity::Critical,
            confidence: Confidence::High,
            languages: vec![Language::Python, Language::JavaScript, Language::Ruby, Language::PHP, Language::Go],
            pattern: r"os\.system\s*\(|subprocess\.\w+\s*\(.*shell\s*=\s*True|exec\s*\(|child_process\.exec\s*\(|exec\.Command\s*\(".to_string(),
            pattern_type: PatternType::Regex,
            cwe: Some("CWE-78".to_string()),
            owasp: Some("A03:2021".to_string()),
            fix_suggestion: Some("Use subprocess with shell=False and a list of arguments".to_string()),
            tags: vec!["injection".to_string(), "command".to_string()],
            enabled: true,
            cvss_score: None,
            cve_references: Vec::new(),
        });

        // Hardcoded Secrets
        self.add_rule(Rule {
            id: "SEC004".to_string(),
            name: "Hardcoded Secret".to_string(),
            description: "Secret or credential hardcoded in source code".to_string(),
            severity: Severity::High,
            confidence: Confidence::Medium,
            languages: Language::all().to_vec(),
            pattern:
                r#"(?i)(password|secret|api[_-]?key|apikey|token|private[_-]?key)\w*["']?\s*(?::=|=>|[:=])\s*["'][^"']{8,}["']"#
                    .to_string(),
            pattern_type: PatternType::Regex,
            cwe: Some("CWE-798".to_string()),
            owasp: Some("A07:2021".to_string()),
            fix_suggestion: Some("Use environment variables or a secrets manager".to_string()),
            tags: vec!["secret".to_string(), "credential".to_string()],
            enabled: true,
            cvss_score: None,
            cve_references: Vec::new(),
        });

        // Path Traversal
        self.add_rule(Rule {
            id: "SEC005".to_string(),
            name: "Path Traversal".to_string(),
            description: "User input used in file path operations".to_string(),
            severity: Severity::High,
            confidence: Confidence::Medium,
            languages: vec![
                Language::Python,
                Language::JavaScript,
                Language::Java,
                Language::PHP,
                Language::Ruby,
                Language::Go,
            ],
            pattern: r#"open\s*\(.*request|readFile\s*\(.*req|FileInputStream\s*\(.*request|fs\.readFile\w*\s*\(|FileInputStream\s*\(\s*["'][^"']*["']\s*\+\s*request|readFile\s*\(.*(?:params|query|param|getParameter)"#
                .to_string(),
            pattern_type: PatternType::Regex,
            cwe: Some("CWE-22".to_string()),
            owasp: Some("A01:2021".to_string()),
            fix_suggestion: Some("Validate and sanitize file paths, use allowlists".to_string()),
            tags: vec!["path-traversal".to_string()],
            enabled: true,
            cvss_score: None,
            cve_references: Vec::new(),
        });

        // Insecure Deserialization
        self.add_rule(Rule {
            id: "SEC006".to_string(),
            name: "Insecure Deserialization".to_string(),
            description: "Deserialization of untrusted data".to_string(),
            severity: Severity::Critical,
            confidence: Confidence::High,
            languages: vec![
                Language::Python,
                Language::Java,
                Language::PHP,
                Language::Ruby,
            ],
            pattern:
                r"(?i)pickle\.loads?\s*\(|yaml\.unsafe_load\s*\(|ObjectInputStream|unserialize\s*\("
                    .to_string(),
            pattern_type: PatternType::Regex,
            cwe: Some("CWE-502".to_string()),
            owasp: Some("A08:2021".to_string()),
            fix_suggestion: Some(
                "Use safe deserialization with explicit type checking".to_string(),
            ),
            tags: vec!["deserialization".to_string()],
            enabled: true,
            cvss_score: None,
            cve_references: Vec::new(),
        });

        // Weak Crypto
        self.add_rule(Rule {
            id: "SEC007".to_string(),
            name: "Weak Cryptographic Algorithm".to_string(),
            description: "Use of weak hash or encryption algorithm".to_string(),
            severity: Severity::Medium,
            confidence: Confidence::High,
            languages: Language::all().to_vec(),
            pattern: r#"(?i)\b(md5|sha1|des|rc4)\b\s*[\.(]|getInstance\s*\(\s*["'](?:MD5|SHA-?1|DES|RC4)["']\)|hashlib\.\s*(?:md5|sha1)\s*\(|md5\.Sum\s*\(|md5\.New\s*\(|sha1\.Sum\s*\(|sha1\.New\s*\("#.to_string(),
            pattern_type: PatternType::Regex,
            cwe: Some("CWE-327".to_string()),
            owasp: Some("A02:2021".to_string()),
            fix_suggestion: Some("Use SHA-256 or stronger algorithms".to_string()),
            tags: vec!["crypto".to_string()],
            enabled: true,
            cvss_score: None,
            cve_references: Vec::new(),
        });

        self.add_ai_security_rules();
        self.add_iac_rules();
        self.add_tree_sitter_rules();
    }

    fn add_tree_sitter_rules(&mut self) {
        // SEC001-TS: SQL Injection via f-string in execute() (Python, tree-sitter)
        //
        // Detects patterns like:
        //   cursor.execute(f"SELECT * FROM users WHERE id = {user_id}")
        //
        // This is structurally precise: it only fires when a string with
        // interpolation (f-string) is passed as an argument to a method
        // named "execute".
        //
        // In tree-sitter-python 0.23, f-strings are `string` nodes containing
        // `interpolation` children (not `formatted_string`).
        self.add_rule(Rule {
            id: "SEC001-TS".to_string(),
            name: "SQL Injection (AST): f-string in execute()".to_string(),
            description: "F-string used as argument to execute(), indicating potential SQL injection"
                .to_string(),
            severity: Severity::Critical,
            confidence: Confidence::High,
            languages: vec![Language::Python],
            pattern: r#"(call
  function: (attribute
    object: (identifier) @_obj
    attribute: (identifier) @_method
    (#eq? @_method "execute"))
  arguments: (argument_list
    (string (interpolation) @sql_fstring)))"#
                .to_string(),
            pattern_type: PatternType::TreeSitterQuery,
            cwe: Some("CWE-89".to_string()),
            owasp: Some("A03:2021".to_string()),
            fix_suggestion: Some(
                "Use parameterized queries: cursor.execute(\"SELECT ... WHERE id = %s\", (user_id,))"
                    .to_string(),
            ),
            tags: vec![
                "injection".to_string(),
                "sql".to_string(),
                "ast".to_string(),
            ],
            enabled: true,
            cvss_score: None,
            cve_references: Vec::new(),
        });

        // SEC001-TS2: SQL Injection via %-format in execute() (Python, tree-sitter)
        //
        // Detects patterns like:
        //   cursor.execute("SELECT * FROM users WHERE id = %s" % user_id)
        self.add_rule(Rule {
            id: "SEC001-TS2".to_string(),
            name: "SQL Injection (AST): %-format in execute()".to_string(),
            description:
                "String %-formatting used as argument to execute(), indicating potential SQL injection"
                    .to_string(),
            severity: Severity::Critical,
            confidence: Confidence::High,
            languages: vec![Language::Python],
            pattern: r#"(call
  function: (attribute
    object: (identifier) @_obj
    attribute: (identifier) @_method
    (#eq? @_method "execute"))
  arguments: (argument_list
    (binary_operator
      left: (string) @_sql_string
      right: (_) @_user_var)))"#
                .to_string(),
            pattern_type: PatternType::TreeSitterQuery,
            cwe: Some("CWE-89".to_string()),
            owasp: Some("A03:2021".to_string()),
            fix_suggestion: Some(
                "Use parameterized queries: cursor.execute(\"SELECT ... WHERE id = %s\", (user_id,))"
                    .to_string(),
            ),
            tags: vec![
                "injection".to_string(),
                "sql".to_string(),
                "ast".to_string(),
            ],
            enabled: true,
            cvss_score: None,
            cve_references: Vec::new(),
        });

        // SEC003-TS: Command Injection via os.system() (Python, tree-sitter)
        //
        // Detects patterns like:
        //   os.system("ping " + hostname)
        //   os.system(f"ping {hostname}")
        self.add_rule(Rule {
            id: "SEC003-TS".to_string(),
            name: "Command Injection (AST): os.system() call".to_string(),
            description: "Call to os.system() detected, which is vulnerable to command injection"
                .to_string(),
            severity: Severity::Critical,
            confidence: Confidence::High,
            languages: vec![Language::Python],
            pattern: r#"(call
  function: (attribute
    object: (identifier) @_obj
    (#eq? @_obj "os")
    attribute: (identifier) @_method
    (#eq? @_method "system"))
  arguments: (argument_list (_) @cmd_arg))"#
                .to_string(),
            pattern_type: PatternType::TreeSitterQuery,
            cwe: Some("CWE-78".to_string()),
            owasp: Some("A03:2021".to_string()),
            fix_suggestion: Some(
                "Use subprocess.run() with shell=False and a list of arguments".to_string(),
            ),
            tags: vec![
                "injection".to_string(),
                "command".to_string(),
                "ast".to_string(),
            ],
            enabled: true,
            cvss_score: None,
            cve_references: Vec::new(),
        });

        // SEC006-TS: Insecure Deserialization via pickle.loads (Python, tree-sitter)
        //
        // Detects patterns like:
        //   pickle.loads(data)
        //   pickle.load(f)
        self.add_rule(Rule {
            id: "SEC006-TS".to_string(),
            name: "Insecure Deserialization (AST): pickle usage".to_string(),
            description:
                "Call to pickle.loads() or pickle.load() detected, which can execute arbitrary code"
                    .to_string(),
            severity: Severity::Critical,
            confidence: Confidence::Certain,
            languages: vec![Language::Python],
            pattern: r#"(call
  function: (attribute
    object: (identifier) @_obj
    (#eq? @_obj "pickle")
    attribute: (identifier) @_method
    (#match? @_method "^loads?$"))
  arguments: (argument_list (_) @arg))"#
                .to_string(),
            pattern_type: PatternType::TreeSitterQuery,
            cwe: Some("CWE-502".to_string()),
            owasp: Some("A08:2021".to_string()),
            fix_suggestion: Some(
                "Use json.loads() or a safe deserialization library instead of pickle".to_string(),
            ),
            tags: vec!["deserialization".to_string(), "ast".to_string()],
            enabled: true,
            cvss_score: None,
            cve_references: Vec::new(),
        });

        // SEC002-TS: XSS via innerHTML assignment (JavaScript, tree-sitter)
        //
        // Detects patterns like:
        //   element.innerHTML = userInput
        self.add_rule(Rule {
            id: "SEC002-TS".to_string(),
            name: "XSS (AST): innerHTML assignment".to_string(),
            description:
                "Assignment to innerHTML detected, which can lead to XSS if user input is used"
                    .to_string(),
            severity: Severity::High,
            confidence: Confidence::Medium,
            languages: vec![Language::JavaScript, Language::TypeScript],
            pattern: r#"(assignment_expression
  left: (member_expression
    property: (property_identifier) @_prop
    (#eq? @_prop "innerHTML"))
  right: (_) @value)"#
                .to_string(),
            pattern_type: PatternType::TreeSitterQuery,
            cwe: Some("CWE-79".to_string()),
            owasp: Some("A03:2021".to_string()),
            fix_suggestion: Some(
                "Use textContent instead of innerHTML, or sanitize with DOMPurify".to_string(),
            ),
            tags: vec![
                "xss".to_string(),
                "injection".to_string(),
                "ast".to_string(),
            ],
            enabled: true,
            cvss_score: None,
            cve_references: Vec::new(),
        });
    }

    fn add_iac_rules(&mut self) {
        // Docker rules
        self.add_rule(Rule {
            id: "IAC-DOCKER-001".to_string(),
            name: "Dockerfile: Running as Root".to_string(),
            description:
                "Container runs as root user, which violates the principle of least privilege"
                    .to_string(),
            severity: Severity::High,
            confidence: Confidence::High,
            languages: vec![],
            pattern: r"(?m)^(?!.*USER\s+\S).*(?:FROM|ENTRYPOINT|CMD)\s".to_string(),
            pattern_type: PatternType::Regex,
            cwe: Some("CWE-250".to_string()),
            owasp: None,
            fix_suggestion: Some("Add a USER directive to run as a non-root user".to_string()),
            tags: vec![
                "iac".to_string(),
                "docker".to_string(),
                "security".to_string(),
            ],
            enabled: true,
            cvss_score: None,
            cve_references: Vec::new(),
        });

        self.add_rule(Rule {
            id: "IAC-DOCKER-002".to_string(),
            name: "Dockerfile: Using latest Tag".to_string(),
            description: "Using 'latest' tag or no tag makes builds non-reproducible".to_string(),
            severity: Severity::Medium,
            confidence: Confidence::High,
            languages: vec![],
            pattern: r"(?i)FROM\s+\S+:latest\b|FROM\s+[a-zA-Z][a-zA-Z0-9._/-]+\s".to_string(),
            pattern_type: PatternType::Regex,
            cwe: None,
            owasp: None,
            fix_suggestion: Some(
                "Pin base images to specific version tags or SHA digests".to_string(),
            ),
            tags: vec![
                "iac".to_string(),
                "docker".to_string(),
                "best-practice".to_string(),
            ],
            enabled: true,
            cvss_score: None,
            cve_references: Vec::new(),
        });

        self.add_rule(Rule {
            id: "IAC-DOCKER-003".to_string(),
            name: "Dockerfile: COPY Wildcard".to_string(),
            description: "COPY with wildcard may include sensitive files like .env or .git"
                .to_string(),
            severity: Severity::Medium,
            confidence: Confidence::Medium,
            languages: vec![],
            pattern: r"COPY\s+\.\s|COPY\s+\*".to_string(),
            pattern_type: PatternType::Regex,
            cwe: Some("CWE-200".to_string()),
            owasp: None,
            fix_suggestion: Some("Use specific COPY targets and a .dockerignore file".to_string()),
            tags: vec!["iac".to_string(), "docker".to_string()],
            enabled: true,
            cvss_score: None,
            cve_references: Vec::new(),
        });

        self.add_rule(Rule {
            id: "IAC-DOCKER-004".to_string(),
            name: "Dockerfile: Secrets in ENV".to_string(),
            description:
                "Secrets or credentials passed via ENV directive are visible in image layers"
                    .to_string(),
            severity: Severity::Critical,
            confidence: Confidence::High,
            languages: vec![],
            pattern:
                r#"(?i)ENV\s+(?:\S+\s+)?(?:password|secret|api[_-]?key|token|private[_-]?key)\s*="#
                    .to_string(),
            pattern_type: PatternType::Regex,
            cwe: Some("CWE-798".to_string()),
            owasp: None,
            fix_suggestion: Some(
                "Use Docker secrets or runtime environment variables instead of build-time ENV"
                    .to_string(),
            ),
            tags: vec![
                "iac".to_string(),
                "docker".to_string(),
                "secret".to_string(),
            ],
            enabled: true,
            cvss_score: None,
            cve_references: Vec::new(),
        });

        // Kubernetes rules
        self.add_rule(Rule {
            id: "IAC-K8S-001".to_string(),
            name: "Kubernetes: Privileged Container".to_string(),
            description: "Container running in privileged mode has full host access".to_string(),
            severity: Severity::Critical,
            confidence: Confidence::Certain,
            languages: vec![],
            pattern: r#"privileged\s*:\s*true"#.to_string(),
            pattern_type: PatternType::Regex,
            cwe: Some("CWE-250".to_string()),
            owasp: None,
            fix_suggestion: Some(
                "Remove privileged: true and use specific capabilities instead".to_string(),
            ),
            tags: vec![
                "iac".to_string(),
                "kubernetes".to_string(),
                "security".to_string(),
            ],
            enabled: true,
            cvss_score: None,
            cve_references: Vec::new(),
        });

        self.add_rule(Rule {
            id: "IAC-K8S-002".to_string(),
            name: "Kubernetes: No Resource Limits".to_string(),
            description: "Container without resource limits can consume unbounded resources"
                .to_string(),
            severity: Severity::Medium,
            confidence: Confidence::Medium,
            languages: vec![],
            pattern: r"(?s)containers\s*:.*?(?:image\s*:)(?!.*(?:limits\s*:))".to_string(),
            pattern_type: PatternType::Regex,
            cwe: Some("CWE-770".to_string()),
            owasp: None,
            fix_suggestion: Some("Add resource limits (cpu, memory) to container spec".to_string()),
            tags: vec![
                "iac".to_string(),
                "kubernetes".to_string(),
                "reliability".to_string(),
            ],
            enabled: true,
            cvss_score: None,
            cve_references: Vec::new(),
        });

        self.add_rule(Rule {
            id: "IAC-K8S-003".to_string(),
            name: "Kubernetes: Host Network Enabled".to_string(),
            description: "Pod using host network namespace bypasses network isolation".to_string(),
            severity: Severity::High,
            confidence: Confidence::Certain,
            languages: vec![],
            pattern: r#"hostNetwork\s*:\s*true"#.to_string(),
            pattern_type: PatternType::Regex,
            cwe: Some("CWE-668".to_string()),
            owasp: None,
            fix_suggestion: Some(
                "Remove hostNetwork: true and use Kubernetes Services for networking".to_string(),
            ),
            tags: vec![
                "iac".to_string(),
                "kubernetes".to_string(),
                "security".to_string(),
            ],
            enabled: true,
            cvss_score: None,
            cve_references: Vec::new(),
        });

        // Terraform rules
        self.add_rule(Rule {
            id: "IAC-TF-001".to_string(),
            name: "Terraform: Public S3 Bucket".to_string(),
            description: "S3 bucket configured with public access".to_string(),
            severity: Severity::Critical,
            confidence: Confidence::High,
            languages: vec![],
            pattern: r#"(?:acl\s*=\s*"public-read"|block_public_acls\s*=\s*false|block_public_policy\s*=\s*false)"#.to_string(),
            pattern_type: PatternType::Regex,
            cwe: Some("CWE-284".to_string()),
            owasp: None,
            fix_suggestion: Some("Set acl to 'private' and enable S3 Block Public Access".to_string()),
            tags: vec!["iac".to_string(), "terraform".to_string(), "aws".to_string(), "security".to_string()],
            enabled: true,
            cvss_score: None,
            cve_references: Vec::new(),
        });

        self.add_rule(Rule {
            id: "IAC-TF-002".to_string(),
            name: "Terraform: Open Security Group".to_string(),
            description: "Security group allows unrestricted inbound access (0.0.0.0/0)"
                .to_string(),
            severity: Severity::High,
            confidence: Confidence::High,
            languages: vec![],
            pattern:
                r#"(?:cidr_blocks\s*=\s*\[\s*"0\.0\.0\.0/0"\s*\]|ingress\s*\{[^}]*0\.0\.0\.0/0)"#
                    .to_string(),
            pattern_type: PatternType::Regex,
            cwe: Some("CWE-284".to_string()),
            owasp: None,
            fix_suggestion: Some("Restrict CIDR blocks to specific IP ranges".to_string()),
            tags: vec![
                "iac".to_string(),
                "terraform".to_string(),
                "aws".to_string(),
                "security".to_string(),
            ],
            enabled: true,
            cvss_score: None,
            cve_references: Vec::new(),
        });

        self.add_rule(Rule {
            id: "IAC-TF-003".to_string(),
            name: "Terraform: Unencrypted EBS Volume".to_string(),
            description: "EBS volume without encryption exposes data at rest".to_string(),
            severity: Severity::High,
            confidence: Confidence::High,
            languages: vec![],
            pattern: r#"resource\s+"aws_ebs_volume"[^{]*\{(?:(?!encrypted\s*=\s*true)[^}])*\}"#
                .to_string(),
            pattern_type: PatternType::Regex,
            cwe: Some("CWE-311".to_string()),
            owasp: None,
            fix_suggestion: Some("Set encrypted = true on EBS volumes".to_string()),
            tags: vec![
                "iac".to_string(),
                "terraform".to_string(),
                "aws".to_string(),
                "encryption".to_string(),
            ],
            enabled: true,
            cvss_score: None,
            cve_references: Vec::new(),
        });
    }

    fn add_ai_security_rules(&mut self) {
        // SEC-AI-001: User input concatenated into LLM prompt
        self.add_rule(Rule {
            id: "SEC-AI-001".to_string(),
            name: "Prompt Injection: User Input in LLM Prompt".to_string(),
            description: "User input directly concatenated into an LLM prompt string, enabling prompt injection attacks".to_string(),
            severity: Severity::High,
            confidence: Confidence::Medium,
            languages: vec![Language::Python, Language::JavaScript, Language::TypeScript],
            pattern: r#"(?:prompt|system_prompt|messages?)\s*(?:=|\+=|\.format\(|%|\.append\().*(?:user_input|request\.|req\.|input\(|args\[|params\[)"#.to_string(),
            pattern_type: PatternType::Regex,
            cwe: Some("CWE-77".to_string()),
            owasp: None,
            fix_suggestion: Some("Sanitize and validate user input before including in prompts. Use structured prompt templates with clear boundaries.".to_string()),
            tags: vec!["ai-security".to_string(), "prompt-injection".to_string()],
            enabled: true,
            cvss_score: None,
            cve_references: Vec::new(),
        });

        // SEC-AI-002: Instruction override patterns
        self.add_rule(Rule {
            id: "SEC-AI-002".to_string(),
            name: "Prompt Injection: Instruction Override Pattern".to_string(),
            description: "Detects common prompt injection instruction override patterns in string literals".to_string(),
            severity: Severity::High,
            confidence: Confidence::High,
            languages: Language::all().to_vec(),
            pattern: r#"(?i)["'].*(?:ignore (?:previous|above|all) (?:instructions?|prompts?)|you are now|new instructions?:|system:\s*override|forget (?:everything|your|all)|disregard (?:previous|all|your)).*["']"#.to_string(),
            pattern_type: PatternType::Regex,
            cwe: Some("CWE-77".to_string()),
            owasp: None,
            fix_suggestion: Some("Do not include untrusted content that could contain instruction override patterns in prompts.".to_string()),
            tags: vec!["ai-security".to_string(), "prompt-injection".to_string()],
            enabled: true,
            cvss_score: None,
            cve_references: Vec::new(),
        });

        // SEC-AI-003: Role-play injection patterns
        self.add_rule(Rule {
            id: "SEC-AI-003".to_string(),
            name: "Prompt Injection: Role-Play Injection".to_string(),
            description: "Detects role-play based prompt injection patterns that attempt to override model behavior".to_string(),
            severity: Severity::Medium,
            confidence: Confidence::Medium,
            languages: Language::all().to_vec(),
            pattern: r#"(?i)["'].*(?:pretend you (?:are|were)|act as (?:if|a)|you(?:'re| are) (?:DAN|an? (?:unrestricted|unfiltered))|roleplay as|jailbreak|do anything now).*["']"#.to_string(),
            pattern_type: PatternType::Regex,
            cwe: Some("CWE-77".to_string()),
            owasp: None,
            fix_suggestion: Some("Filter user input for role-play injection patterns before including in LLM prompts.".to_string()),
            tags: vec!["ai-security".to_string(), "prompt-injection".to_string(), "jailbreak".to_string()],
            enabled: true,
            cvss_score: None,
            cve_references: Vec::new(),
        });

        // SEC-AI-004: Encoding-based prompt obfuscation
        self.add_rule(Rule {
            id: "SEC-AI-004".to_string(),
            name: "Prompt Injection: Encoding Obfuscation".to_string(),
            description: "Detects base64/hex/unicode encoding of user input before inclusion in prompts, which may be used to bypass filters".to_string(),
            severity: Severity::Medium,
            confidence: Confidence::Low,
            languages: vec![Language::Python, Language::JavaScript, Language::TypeScript],
            pattern: r#"(?:base64\.(?:b64)?(?:encode|decode)|atob|btoa|\\u[0-9a-fA-F]{4}|\\x[0-9a-fA-F]{2}|bytes\.fromhex|hex\(\)).*(?:prompt|message|system|instruction)"#.to_string(),
            pattern_type: PatternType::Regex,
            cwe: Some("CWE-116".to_string()),
            owasp: None,
            fix_suggestion: Some("Avoid encoding transformations on user input that will be included in prompts.".to_string()),
            tags: vec!["ai-security".to_string(), "prompt-injection".to_string(), "obfuscation".to_string()],
            enabled: true,
            cvss_score: None,
            cve_references: Vec::new(),
        });

        // SEC-AI-005: Prompt leaking patterns
        self.add_rule(Rule {
            id: "SEC-AI-005".to_string(),
            name: "Prompt Injection: Prompt Leaking".to_string(),
            description: "Detects patterns that attempt to extract or leak the system prompt".to_string(),
            severity: Severity::Medium,
            confidence: Confidence::High,
            languages: Language::all().to_vec(),
            pattern: r#"(?i)["'].*(?:(?:print|show|display|reveal|output|repeat|echo) (?:your |the )?(?:system |initial )?(?:prompt|instructions?|rules)|what (?:are|were) your (?:instructions?|rules|prompt)).*["']"#.to_string(),
            pattern_type: PatternType::Regex,
            cwe: Some("CWE-200".to_string()),
            owasp: None,
            fix_suggestion: Some("Implement output filtering to prevent system prompt leakage. Use prompt guards.".to_string()),
            tags: vec!["ai-security".to_string(), "prompt-leaking".to_string()],
            enabled: true,
            cvss_score: None,
            cve_references: Vec::new(),
        });
    }
}

impl Default for RuleDatabase {
    fn default() -> Self {
        Self::with_defaults()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_rules() {
        let db = RuleDatabase::with_defaults();
        assert!(db.len() >= 7);
        assert!(db.get_rule("SEC001").is_some());
    }

    #[test]
    fn test_rules_for_language() {
        let db = RuleDatabase::with_defaults();
        let python_rules = db.rules_for_language(Language::Python);
        assert!(!python_rules.is_empty());
    }

    #[test]
    fn test_enabled_rules() {
        let db = RuleDatabase::with_defaults();
        let enabled = db.enabled_rules();
        assert_eq!(enabled.len(), db.len());
    }
}
