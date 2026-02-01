//! Expression-level constant evaluation.
//!
//! Evaluates constant expressions by pattern-matching on source text,
//! handling integer arithmetic, string operations, boolean logic,
//! and comparison operators.

use std::collections::HashMap;

use super::constant_prop::ConstValue;

/// Evaluate a constant expression from source text.
///
/// Attempts to evaluate the expression given the current constant environment.
/// Returns `Bottom` if the expression cannot be evaluated at compile time.
pub fn eval_const_expr(expr: &str, env: &HashMap<String, ConstValue>) -> ConstValue {
    let trimmed = expr.trim();

    if trimmed.is_empty() {
        return ConstValue::Bottom;
    }

    // Try parenthesized expression: strip outer parens and recurse.
    if let Some(inner) = strip_balanced_parens(trimmed) {
        return eval_const_expr(inner, env);
    }

    // Try integer literal
    if let Some(v) = parse_int_literal(trimmed) {
        return ConstValue::Constant(v);
    }

    // Try boolean literal
    if let Some(v) = parse_bool_literal(trimmed) {
        return ConstValue::BoolConst(v);
    }

    // Try string literal
    if let Some(v) = parse_string_literal(trimmed) {
        return ConstValue::StringConst(v);
    }

    // Try variable reference
    if is_identifier(trimmed) {
        return env.get(trimmed).cloned().unwrap_or(ConstValue::Top);
    }

    // Try unary operators: not, -, !
    if let Some(result) = eval_unary(trimmed, env) {
        return result;
    }

    // Try ternary/conditional (before binary to avoid ambiguity with "if" keyword)
    if let Some(result) = eval_ternary(trimmed, env) {
        return result;
    }

    // Try len() call
    if let Some(result) = eval_len(trimmed, env) {
        return result;
    }

    // Try binary operators
    if let Some(result) = eval_binary(trimmed, env) {
        return result;
    }

    ConstValue::Bottom
}

/// Strip balanced outer parentheses from an expression, if present.
fn strip_balanced_parens(s: &str) -> Option<&str> {
    if !s.starts_with('(') || !s.ends_with(')') {
        return None;
    }
    // Verify the parens are actually matched: the open paren at index 0
    // must match the close paren at the end.
    let mut depth = 0i32;
    for (i, ch) in s.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                // If depth hits zero before the last char, the outer parens
                // are not wrapping the whole expression.
                if depth == 0 && i < s.len() - 1 {
                    return None;
                }
            }
            _ => {}
        }
    }
    if depth == 0 {
        Some(&s[1..s.len() - 1])
    } else {
        None
    }
}

/// Parse an integer literal (decimal, hex, octal, binary).
/// Handles underscores in numbers (e.g. `1_000_000`) and optional leading `-`.
fn parse_int_literal(s: &str) -> Option<i64> {
    let (negative, digits) = if let Some(rest) = s.strip_prefix('-') {
        (true, rest.trim())
    } else {
        (false, s)
    };

    if digits.is_empty() {
        return None;
    }

    let cleaned: String = digits.chars().filter(|&c| c != '_').collect();
    if cleaned.is_empty() {
        return None;
    }

    let value = if let Some(hex) = cleaned
        .strip_prefix("0x")
        .or_else(|| cleaned.strip_prefix("0X"))
    {
        i64::from_str_radix(hex, 16).ok()?
    } else if let Some(oct) = cleaned
        .strip_prefix("0o")
        .or_else(|| cleaned.strip_prefix("0O"))
    {
        i64::from_str_radix(oct, 8).ok()?
    } else if let Some(bin) = cleaned
        .strip_prefix("0b")
        .or_else(|| cleaned.strip_prefix("0B"))
    {
        i64::from_str_radix(bin, 2).ok()?
    } else {
        cleaned.parse::<i64>().ok()?
    };

    Some(if negative { -value } else { value })
}

/// Parse a boolean literal.
fn parse_bool_literal(s: &str) -> Option<bool> {
    match s {
        "true" | "True" | "TRUE" => Some(true),
        "false" | "False" | "FALSE" => Some(false),
        _ => None,
    }
}

/// Parse a string literal (single or double quoted).
/// Handles basic escape sequences.
fn parse_string_literal(s: &str) -> Option<String> {
    let (quote, inner) = if s.len() >= 2 {
        if s.starts_with('"') && s.ends_with('"') {
            ('"', &s[1..s.len() - 1])
        } else if s.starts_with('\'') && s.ends_with('\'') {
            ('\'', &s[1..s.len() - 1])
        } else {
            return None;
        }
    } else {
        return None;
    };

    // Make sure there are no unescaped quotes inside
    let mut result = String::new();
    let mut chars = inner.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            match chars.next() {
                Some('n') => result.push('\n'),
                Some('t') => result.push('\t'),
                Some('r') => result.push('\r'),
                Some('\\') => result.push('\\'),
                Some('\'') => result.push('\''),
                Some('"') => result.push('"'),
                Some('0') => result.push('\0'),
                Some(other) => {
                    result.push('\\');
                    result.push(other);
                }
                None => result.push('\\'),
            }
        } else if ch == quote {
            // Unescaped quote inside -- invalid
            return None;
        } else {
            result.push(ch);
        }
    }
    Some(result)
}

/// Check if string is a valid identifier.
fn is_identifier(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .next()
            .is_some_and(|c| c.is_alphabetic() || c == '_')
        && s.chars().all(|c| c.is_alphanumeric() || c == '_')
}

/// Evaluate unary operators.
fn eval_unary(expr: &str, env: &HashMap<String, ConstValue>) -> Option<ConstValue> {
    // Boolean negation: "not expr" or "!expr"
    if let Some(rest) = expr.strip_prefix("not ") {
        let inner = eval_const_expr(rest.trim(), env);
        return Some(match inner.is_truthy() {
            Some(b) => ConstValue::BoolConst(!b),
            None => ConstValue::Bottom,
        });
    }
    if let Some(rest) = expr.strip_prefix('!') {
        let rest = rest.trim();
        if !rest.is_empty() {
            let inner = eval_const_expr(rest, env);
            return Some(match inner.is_truthy() {
                Some(b) => ConstValue::BoolConst(!b),
                None => ConstValue::Bottom,
            });
        }
    }

    // Unary minus: "-expr" (but not a literal, which was already handled)
    if let Some(rest) = expr.strip_prefix('-') {
        let rest = rest.trim();
        if !rest.is_empty() && !rest.chars().next().unwrap().is_ascii_digit() {
            let inner = eval_const_expr(rest, env);
            return Some(match inner {
                ConstValue::Constant(v) => ConstValue::Constant(-v),
                _ => ConstValue::Bottom,
            });
        }
    }

    None
}

/// Operator precedence levels (lower number = lower precedence = binds more loosely).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Precedence {
    Or = 1,
    And = 2,
    Comparison = 3,
    Addition = 4,
    Multiplication = 5,
    Power = 6,
}

/// Recognized binary operators with their textual representation and precedence.
static OPERATORS: &[(&str, Precedence)] = &[
    // Logical (lowest precedence)
    (" or ", Precedence::Or),
    ("||", Precedence::Or),
    (" and ", Precedence::And),
    ("&&", Precedence::And),
    // Comparison
    ("==", Precedence::Comparison),
    ("!=", Precedence::Comparison),
    ("<=", Precedence::Comparison),
    (">=", Precedence::Comparison),
    ("<", Precedence::Comparison),
    (">", Precedence::Comparison),
    // Additive
    ("+", Precedence::Addition),
    ("-", Precedence::Addition),
    // Multiplicative
    ("*", Precedence::Multiplication),
    ("/", Precedence::Multiplication),
    ("%", Precedence::Multiplication),
    // Power
    ("**", Precedence::Power),
];

/// Find the main (lowest-precedence, rightmost for left-associative) binary
/// operator in an expression, respecting parentheses and string literals.
fn find_split_operator(expr: &str) -> Option<(usize, usize, &'static str)> {
    let bytes = expr.as_bytes();
    let len = bytes.len();

    // We want the lowest precedence operator. Among equal precedence, we want
    // the rightmost occurrence (for left-associativity).
    // For power (**), we want the leftmost (right-associativity), but we keep
    // it simple and just pick rightmost -- good enough for constant eval.
    let mut best: Option<(usize, usize, &str, Precedence)> = None;

    let mut i = 0;
    let mut paren_depth: i32 = 0;
    let mut in_single_quote = false;
    let mut in_double_quote = false;

    while i < len {
        let ch = bytes[i] as char;

        // Track string literals
        if ch == '\'' && !in_double_quote {
            if i == 0 || bytes[i - 1] != b'\\' {
                in_single_quote = !in_single_quote;
            }
            i += 1;
            continue;
        }
        if ch == '"' && !in_single_quote {
            if i == 0 || bytes[i - 1] != b'\\' {
                in_double_quote = !in_double_quote;
            }
            i += 1;
            continue;
        }
        if in_single_quote || in_double_quote {
            i += 1;
            continue;
        }

        // Track parentheses
        if ch == '(' {
            paren_depth += 1;
            i += 1;
            continue;
        }
        if ch == ')' {
            paren_depth -= 1;
            i += 1;
            continue;
        }

        // Only look for operators at depth 0
        if paren_depth == 0 {
            // At each position, find the best (longest) matching operator.
            // We try all operators but prefer multi-char operators over
            // single-char ones at the same position to avoid e.g. matching
            // `*` at a position that is part of `**`.
            let mut pos_match: Option<(usize, &str, Precedence)> = None;

            for &(op_str, prec) in OPERATORS {
                let op_len = op_str.len();
                if i + op_len > len {
                    continue;
                }
                if &expr[i..i + op_len] != op_str {
                    continue;
                }
                // Avoid matching + or - at the start (unary)
                if (op_str == "+" || op_str == "-") && i == 0 {
                    continue;
                }

                // Prefer the longest operator at this position
                let dominated = match &pos_match {
                    Some((existing_len, _, _)) => op_len <= *existing_len,
                    None => false,
                };
                if !dominated {
                    pos_match = Some((op_len, op_str, prec));
                }
            }

            if let Some((op_len, op_str, prec)) = pos_match {
                let should_replace = match &best {
                    None => true,
                    Some((_, _, _, best_prec)) => prec <= *best_prec,
                };
                if should_replace {
                    best = Some((i, op_len, op_str, prec));
                }
                // Skip past the multi-char operator so we don't re-match
                // a suffix of it (e.g. the second `*` in `**`).
                if op_len > 1 {
                    i += op_len;
                    continue;
                }
            }
        }

        i += 1;
    }

    best.map(|(pos, op_len, op_str, _)| (pos, op_len, op_str))
}

/// Evaluate binary operators.
fn eval_binary(expr: &str, env: &HashMap<String, ConstValue>) -> Option<ConstValue> {
    let (pos, op_len, op_str) = find_split_operator(expr)?;

    let lhs_str = expr[..pos].trim();
    let rhs_str = expr[pos + op_len..].trim();

    if lhs_str.is_empty() || rhs_str.is_empty() {
        return None;
    }

    let lhs = eval_const_expr(lhs_str, env);
    let rhs = eval_const_expr(rhs_str, env);

    // If either side is Bottom or Top, we usually cannot evaluate.
    // Exception: short-circuit logic.
    match op_str.trim() {
        "or" | "||" => {
            // Short circuit: true or _ = true
            if lhs.is_truthy() == Some(true) {
                return Some(ConstValue::BoolConst(true));
            }
            if lhs.is_truthy() == Some(false) {
                return rhs.is_truthy().map(ConstValue::BoolConst);
            }
            return None;
        }
        "and" | "&&" => {
            // Short circuit: false and _ = false
            if lhs.is_truthy() == Some(false) {
                return Some(ConstValue::BoolConst(false));
            }
            if lhs.is_truthy() == Some(true) {
                return rhs.is_truthy().map(ConstValue::BoolConst);
            }
            return None;
        }
        _ => {}
    }

    // For arithmetic and comparison, both sides need to be resolved.
    match (&lhs, &rhs) {
        // Integer arithmetic
        (ConstValue::Constant(a), ConstValue::Constant(b)) => {
            match op_str {
                "+" => Some(ConstValue::Constant(a.wrapping_add(*b))),
                "-" => Some(ConstValue::Constant(a.wrapping_sub(*b))),
                "*" => Some(ConstValue::Constant(a.wrapping_mul(*b))),
                "/" => {
                    if *b == 0 {
                        Some(ConstValue::Bottom)
                    } else {
                        Some(ConstValue::Constant(a.wrapping_div(*b)))
                    }
                }
                "%" => {
                    if *b == 0 {
                        Some(ConstValue::Bottom)
                    } else {
                        Some(ConstValue::Constant(a.wrapping_rem(*b)))
                    }
                }
                "**" => {
                    if *b < 0 {
                        Some(ConstValue::Bottom) // negative exponents yield fractions
                    } else {
                        Some(ConstValue::Constant(a.wrapping_pow(*b as u32)))
                    }
                }
                "==" => Some(ConstValue::BoolConst(a == b)),
                "!=" => Some(ConstValue::BoolConst(a != b)),
                "<" => Some(ConstValue::BoolConst(a < b)),
                ">" => Some(ConstValue::BoolConst(a > b)),
                "<=" => Some(ConstValue::BoolConst(a <= b)),
                ">=" => Some(ConstValue::BoolConst(a >= b)),
                _ => None,
            }
        }
        // String concatenation with +
        (ConstValue::StringConst(a), ConstValue::StringConst(b)) if op_str == "+" => {
            Some(ConstValue::StringConst(format!("{}{}", a, b)))
        }
        // String equality
        (ConstValue::StringConst(a), ConstValue::StringConst(b)) => match op_str {
            "==" => Some(ConstValue::BoolConst(a == b)),
            "!=" => Some(ConstValue::BoolConst(a != b)),
            _ => None,
        },
        // Boolean comparison
        (ConstValue::BoolConst(a), ConstValue::BoolConst(b)) => match op_str {
            "==" => Some(ConstValue::BoolConst(a == b)),
            "!=" => Some(ConstValue::BoolConst(a != b)),
            _ => None,
        },
        _ => None,
    }
}

/// Evaluate ternary expressions.
///
/// Supports:
/// - Python style: `value_true if condition else value_false`
/// - C style: `condition ? value_true : value_false`
fn eval_ternary(expr: &str, env: &HashMap<String, ConstValue>) -> Option<ConstValue> {
    // Python style: look for " if " ... " else "
    if let Some(if_pos) = find_keyword_at_depth0(expr, " if ") {
        let value_true_str = expr[..if_pos].trim();
        let rest = &expr[if_pos + 4..]; // skip " if "
        if let Some(else_pos) = find_keyword_at_depth0(rest, " else ") {
            let condition_str = rest[..else_pos].trim();
            let value_false_str = rest[else_pos + 6..].trim(); // skip " else "

            let cond = eval_const_expr(condition_str, env);
            return match cond.is_truthy() {
                Some(true) => Some(eval_const_expr(value_true_str, env)),
                Some(false) => Some(eval_const_expr(value_false_str, env)),
                None => None,
            };
        }
    }

    // C style: condition ? value_true : value_false
    if let Some(q_pos) = find_keyword_at_depth0(expr, "?") {
        let condition_str = expr[..q_pos].trim();
        let rest = &expr[q_pos + 1..];
        if let Some(colon_pos) = find_keyword_at_depth0(rest, ":") {
            let value_true_str = rest[..colon_pos].trim();
            let value_false_str = rest[colon_pos + 1..].trim();

            let cond = eval_const_expr(condition_str, env);
            return match cond.is_truthy() {
                Some(true) => Some(eval_const_expr(value_true_str, env)),
                Some(false) => Some(eval_const_expr(value_false_str, env)),
                None => None,
            };
        }
    }

    None
}

/// Find a keyword/substring in the expression at parenthesis depth 0,
/// not inside string literals. Returns the byte offset of the match.
fn find_keyword_at_depth0(expr: &str, keyword: &str) -> Option<usize> {
    let bytes = expr.as_bytes();
    let kw_bytes = keyword.as_bytes();
    let kw_len = kw_bytes.len();
    let len = bytes.len();

    if len < kw_len {
        return None;
    }

    let mut i = 0;
    let mut paren_depth: i32 = 0;
    let mut in_single_quote = false;
    let mut in_double_quote = false;

    while i + kw_len <= len {
        let ch = bytes[i] as char;

        if ch == '\'' && !in_double_quote && (i == 0 || bytes[i - 1] != b'\\') {
            in_single_quote = !in_single_quote;
            i += 1;
            continue;
        }
        if ch == '"' && !in_single_quote && (i == 0 || bytes[i - 1] != b'\\') {
            in_double_quote = !in_double_quote;
            i += 1;
            continue;
        }
        if in_single_quote || in_double_quote {
            i += 1;
            continue;
        }
        if ch == '(' {
            paren_depth += 1;
            i += 1;
            continue;
        }
        if ch == ')' {
            paren_depth -= 1;
            i += 1;
            continue;
        }

        if paren_depth == 0 && &bytes[i..i + kw_len] == kw_bytes {
            return Some(i);
        }

        i += 1;
    }

    None
}

/// Evaluate `len()` calls.
fn eval_len(expr: &str, env: &HashMap<String, ConstValue>) -> Option<ConstValue> {
    let trimmed = expr.trim();
    if !trimmed.starts_with("len(") || !trimmed.ends_with(')') {
        return None;
    }
    let inner = &trimmed[4..trimmed.len() - 1].trim();
    let val = eval_const_expr(inner, env);
    match val {
        ConstValue::StringConst(s) => Some(ConstValue::Constant(s.len() as i64)),
        _ => None,
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_env() -> HashMap<String, ConstValue> {
        HashMap::new()
    }

    fn env_with(pairs: &[(&str, ConstValue)]) -> HashMap<String, ConstValue> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect()
    }

    // ---- Integer literal parsing ------------------------------------------

    #[test]
    fn test_parse_int_decimal() {
        assert_eq!(parse_int_literal("42"), Some(42));
        assert_eq!(parse_int_literal("0"), Some(0));
        assert_eq!(parse_int_literal("-7"), Some(-7));
    }

    #[test]
    fn test_parse_int_hex() {
        assert_eq!(parse_int_literal("0xFF"), Some(255));
        assert_eq!(parse_int_literal("0x10"), Some(16));
    }

    #[test]
    fn test_parse_int_octal() {
        assert_eq!(parse_int_literal("0o77"), Some(63));
    }

    #[test]
    fn test_parse_int_binary() {
        assert_eq!(parse_int_literal("0b1010"), Some(10));
    }

    #[test]
    fn test_parse_int_underscores() {
        assert_eq!(parse_int_literal("1_000_000"), Some(1_000_000));
    }

    // ---- Boolean literal parsing ------------------------------------------

    #[test]
    fn test_parse_bool() {
        assert_eq!(parse_bool_literal("true"), Some(true));
        assert_eq!(parse_bool_literal("True"), Some(true));
        assert_eq!(parse_bool_literal("FALSE"), Some(false));
        assert_eq!(parse_bool_literal("maybe"), None);
    }

    // ---- String literal parsing -------------------------------------------

    #[test]
    fn test_parse_string_double() {
        assert_eq!(parse_string_literal("\"hello\""), Some("hello".into()));
    }

    #[test]
    fn test_parse_string_single() {
        assert_eq!(parse_string_literal("'world'"), Some("world".into()));
    }

    #[test]
    fn test_parse_string_escapes() {
        assert_eq!(
            parse_string_literal("\"line\\nbreak\""),
            Some("line\nbreak".into())
        );
        assert_eq!(
            parse_string_literal("\"tab\\there\""),
            Some("tab\there".into())
        );
    }

    #[test]
    fn test_parse_string_empty() {
        assert_eq!(parse_string_literal("\"\""), Some(String::new()));
    }

    // ---- Identifier check -------------------------------------------------

    #[test]
    fn test_is_identifier() {
        assert!(is_identifier("foo"));
        assert!(is_identifier("_bar"));
        assert!(is_identifier("x2"));
        assert!(!is_identifier("2x"));
        assert!(!is_identifier(""));
        assert!(!is_identifier("a+b"));
    }

    // ---- Integer arithmetic -----------------------------------------------

    #[test]
    fn test_add() {
        assert_eq!(
            eval_const_expr("2+3", &empty_env()),
            ConstValue::Constant(5)
        );
    }

    #[test]
    fn test_subtract() {
        assert_eq!(
            eval_const_expr("10 - 4", &empty_env()),
            ConstValue::Constant(6)
        );
    }

    #[test]
    fn test_multiply() {
        assert_eq!(
            eval_const_expr("3 * 7", &empty_env()),
            ConstValue::Constant(21)
        );
    }

    #[test]
    fn test_divide() {
        assert_eq!(
            eval_const_expr("15 / 3", &empty_env()),
            ConstValue::Constant(5)
        );
    }

    #[test]
    fn test_modulo() {
        assert_eq!(
            eval_const_expr("17 % 5", &empty_env()),
            ConstValue::Constant(2)
        );
    }

    #[test]
    fn test_power() {
        assert_eq!(
            eval_const_expr("2 ** 10", &empty_env()),
            ConstValue::Constant(1024)
        );
    }

    #[test]
    fn test_divide_by_zero() {
        assert_eq!(eval_const_expr("5 / 0", &empty_env()), ConstValue::Bottom);
    }

    // ---- String operations ------------------------------------------------

    #[test]
    fn test_string_concat() {
        assert_eq!(
            eval_const_expr("\"hello\" + \" \" + \"world\"", &empty_env()),
            ConstValue::StringConst("hello world".into())
        );
    }

    #[test]
    fn test_string_equality() {
        assert_eq!(
            eval_const_expr("\"abc\" == \"abc\"", &empty_env()),
            ConstValue::BoolConst(true)
        );
        assert_eq!(
            eval_const_expr("\"abc\" != \"def\"", &empty_env()),
            ConstValue::BoolConst(true)
        );
    }

    // ---- Boolean logic ----------------------------------------------------

    #[test]
    fn test_and() {
        assert_eq!(
            eval_const_expr("true and false", &empty_env()),
            ConstValue::BoolConst(false)
        );
        assert_eq!(
            eval_const_expr("true and true", &empty_env()),
            ConstValue::BoolConst(true)
        );
    }

    #[test]
    fn test_or() {
        assert_eq!(
            eval_const_expr("true or false", &empty_env()),
            ConstValue::BoolConst(true)
        );
        assert_eq!(
            eval_const_expr("false or false", &empty_env()),
            ConstValue::BoolConst(false)
        );
    }

    #[test]
    fn test_not() {
        assert_eq!(
            eval_const_expr("not true", &empty_env()),
            ConstValue::BoolConst(false)
        );
        assert_eq!(
            eval_const_expr("not false", &empty_env()),
            ConstValue::BoolConst(true)
        );
    }

    #[test]
    fn test_bang_not() {
        assert_eq!(
            eval_const_expr("!true", &empty_env()),
            ConstValue::BoolConst(false)
        );
    }

    // ---- Comparisons ------------------------------------------------------

    #[test]
    fn test_greater_than() {
        assert_eq!(
            eval_const_expr("5 > 3", &empty_env()),
            ConstValue::BoolConst(true)
        );
        assert_eq!(
            eval_const_expr("3 > 5", &empty_env()),
            ConstValue::BoolConst(false)
        );
    }

    #[test]
    fn test_equals() {
        assert_eq!(
            eval_const_expr("5 == 5", &empty_env()),
            ConstValue::BoolConst(true)
        );
        assert_eq!(
            eval_const_expr("5 == 6", &empty_env()),
            ConstValue::BoolConst(false)
        );
    }

    #[test]
    fn test_less_equal() {
        assert_eq!(
            eval_const_expr("3 <= 4", &empty_env()),
            ConstValue::BoolConst(true)
        );
        assert_eq!(
            eval_const_expr("4 <= 4", &empty_env()),
            ConstValue::BoolConst(true)
        );
        assert_eq!(
            eval_const_expr("5 <= 4", &empty_env()),
            ConstValue::BoolConst(false)
        );
    }

    #[test]
    fn test_greater_equal() {
        assert_eq!(
            eval_const_expr("3 >= 4", &empty_env()),
            ConstValue::BoolConst(false)
        );
        assert_eq!(
            eval_const_expr("4 >= 4", &empty_env()),
            ConstValue::BoolConst(true)
        );
    }

    #[test]
    fn test_not_equal() {
        assert_eq!(
            eval_const_expr("5 != 3", &empty_env()),
            ConstValue::BoolConst(true)
        );
    }

    // ---- Ternary ----------------------------------------------------------

    #[test]
    fn test_python_ternary_true() {
        assert_eq!(
            eval_const_expr("5 if True else 10", &empty_env()),
            ConstValue::Constant(5)
        );
    }

    #[test]
    fn test_python_ternary_false() {
        assert_eq!(
            eval_const_expr("5 if False else 10", &empty_env()),
            ConstValue::Constant(10)
        );
    }

    #[test]
    fn test_c_ternary_true() {
        assert_eq!(
            eval_const_expr("true ? 42 : 0", &empty_env()),
            ConstValue::Constant(42)
        );
    }

    #[test]
    fn test_c_ternary_false() {
        assert_eq!(
            eval_const_expr("false ? 42 : 0", &empty_env()),
            ConstValue::Constant(0)
        );
    }

    // ---- len() ------------------------------------------------------------

    #[test]
    fn test_len_string_literal() {
        assert_eq!(
            eval_const_expr("len(\"hello\")", &empty_env()),
            ConstValue::Constant(5)
        );
    }

    #[test]
    fn test_len_empty_string() {
        assert_eq!(
            eval_const_expr("len(\"\")", &empty_env()),
            ConstValue::Constant(0)
        );
    }

    #[test]
    fn test_len_variable() {
        let env = env_with(&[("s", ConstValue::StringConst("abcdef".into()))]);
        assert_eq!(eval_const_expr("len(s)", &env), ConstValue::Constant(6));
    }

    // ---- Variable substitution --------------------------------------------

    #[test]
    fn test_variable_lookup() {
        let env = env_with(&[("x", ConstValue::Constant(5))]);
        assert_eq!(eval_const_expr("x + 3", &env), ConstValue::Constant(8));
    }

    #[test]
    fn test_variable_unknown() {
        assert_eq!(eval_const_expr("x", &empty_env()), ConstValue::Top);
    }

    #[test]
    fn test_variable_bool() {
        let env = env_with(&[("flag", ConstValue::BoolConst(true))]);
        assert_eq!(
            eval_const_expr("not flag", &env),
            ConstValue::BoolConst(false)
        );
    }

    // ---- Nested / precedence ----------------------------------------------

    #[test]
    fn test_nested_parens() {
        assert_eq!(
            eval_const_expr("(2 + 3) * 4", &empty_env()),
            ConstValue::Constant(20)
        );
    }

    #[test]
    fn test_precedence_mul_before_add() {
        // 2 + 3 * 4 = 2 + 12 = 14
        assert_eq!(
            eval_const_expr("2 + 3 * 4", &empty_env()),
            ConstValue::Constant(14)
        );
    }

    #[test]
    fn test_complex_expression() {
        let env = env_with(&[
            ("a", ConstValue::Constant(10)),
            ("b", ConstValue::Constant(3)),
        ]);
        // a + b * 2 = 10 + 6 = 16
        assert_eq!(eval_const_expr("a + b * 2", &env), ConstValue::Constant(16));
    }

    #[test]
    fn test_comparison_with_logic() {
        // (5 > 3) and (2 < 4) -> true and true -> true
        assert_eq!(
            eval_const_expr("5 > 3 and 2 < 4", &empty_env()),
            ConstValue::BoolConst(true)
        );
    }

    #[test]
    fn test_short_circuit_or() {
        // true or anything -> true
        assert_eq!(
            eval_const_expr("true or unknown_var", &empty_env()),
            ConstValue::BoolConst(true)
        );
    }

    #[test]
    fn test_short_circuit_and() {
        // false and anything -> false
        assert_eq!(
            eval_const_expr("false and unknown_var", &empty_env()),
            ConstValue::BoolConst(false)
        );
    }

    #[test]
    fn test_bottom_for_unknown_expr() {
        assert_eq!(
            eval_const_expr("foo(bar, baz)", &empty_env()),
            ConstValue::Bottom
        );
    }

    #[test]
    fn test_boolean_and_c_style() {
        assert_eq!(
            eval_const_expr("true && false", &empty_env()),
            ConstValue::BoolConst(false)
        );
        assert_eq!(
            eval_const_expr("false || true", &empty_env()),
            ConstValue::BoolConst(true)
        );
    }

    #[test]
    fn test_negative_literal() {
        assert_eq!(
            eval_const_expr("-42", &empty_env()),
            ConstValue::Constant(-42)
        );
    }

    #[test]
    fn test_unary_negate_var() {
        let env = env_with(&[("x", ConstValue::Constant(7))]);
        assert_eq!(eval_const_expr("-x", &env), ConstValue::Constant(-7));
    }

    #[test]
    fn test_hex_literal_expr() {
        assert_eq!(
            eval_const_expr("0xFF", &empty_env()),
            ConstValue::Constant(255)
        );
    }

    #[test]
    fn test_empty_expr() {
        assert_eq!(eval_const_expr("", &empty_env()), ConstValue::Bottom);
        assert_eq!(eval_const_expr("  ", &empty_env()), ConstValue::Bottom);
    }

    #[test]
    fn test_truthy_integer() {
        // 0 is falsy, nonzero is truthy
        assert_eq!(
            eval_const_expr("0 or 5", &empty_env()),
            ConstValue::BoolConst(true) // 0 is false, then check 5 which is truthy
        );
    }

    #[test]
    fn test_string_concat_with_var() {
        let env = env_with(&[("greeting", ConstValue::StringConst("hi".into()))]);
        assert_eq!(
            eval_const_expr("greeting + \"!\"", &env),
            ConstValue::StringConst("hi!".into())
        );
    }
}
