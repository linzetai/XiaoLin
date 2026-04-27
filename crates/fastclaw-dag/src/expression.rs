//! Mini condition expression engine for DAG [`crate::definition::NodeKind::Condition`] nodes
//! and loop [`crate::definition::LoopConfig::condition_expr`] values.
//!
//! Supports JSON-pointer-style paths with array indices (`$.node_id.field`, `$.items[0].name`),
//! comparisons, logical operators, list membership (`$.x in ["a","b"]`), `contains(hay, needle)`,
//! and optional ternary-style branch selection: `predicate ? "then_label" : "else_label"`.

use serde_json::Value;

const MAX_EXPRESSION_LEN: usize = 4096;
const MAX_RECURSION_DEPTH: usize = 32;

/// Evaluate a boolean expression (no ternary branch selection) against JSON context.
pub fn evaluate_bool(expression: &str, context: &Value) -> anyhow::Result<bool> {
    eval_bool_expr(expression, context)
}

/// Evaluate a condition expression against a JSON context (typically a snapshot of node outputs
/// plus `input`). Returns the branch label: `"true"` / `"false"` for plain boolean expressions,
/// or the chosen side of a ternary.
pub fn evaluate_condition(expression: &str, context: &Value) -> anyhow::Result<String> {
    let expression = expression.trim();
    if expression.is_empty() {
        anyhow::bail!("empty condition expression");
    }

    if expression.len() > MAX_EXPRESSION_LEN {
        anyhow::bail!(
            "expression too long ({} bytes, max {MAX_EXPRESSION_LEN})",
            expression.len()
        );
    }

    if let Some((cond_src, then_src, else_src)) = split_top_level_ternary(expression) {
        let cond_val = eval_bool_expr(cond_src, context)?;
        let branch_src = if cond_val { then_src } else { else_src };
        Ok(parse_branch_label(branch_src)?)
    } else {
        let b = eval_bool_expr(expression, context)?;
        Ok(if b {
            "true".to_string()
        } else {
            "false".to_string()
        })
    }
}

fn parse_branch_label(src: &str) -> anyhow::Result<String> {
    let s = src.trim();
    if s.is_empty() {
        anyhow::bail!("empty branch label in ternary");
    }
    if let Some(rest) = s.strip_prefix('"') {
        let end = rest
            .find('"')
            .ok_or_else(|| anyhow::anyhow!("unterminated string in branch label"))?;
        Ok(rest[..end].to_string())
    } else {
        Ok(s.to_string())
    }
}

/// Split `cond ? then : else` at the top level (respecting string quotes).
fn split_top_level_ternary(src: &str) -> Option<(&str, &str, &str)> {
    let bytes = src.as_bytes();
    let mut depth = 0i32;
    let mut in_string = false;
    let mut escape = false;
    let mut q_pos = None;

    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if in_string {
            if escape {
                escape = false;
            } else if c == b'\\' {
                escape = true;
            } else if c == b'"' {
                in_string = false;
            }
            i += 1;
            continue;
        }
        if c == b'"' {
            in_string = true;
            i += 1;
            continue;
        }
        if c == b'(' {
            depth += 1;
            i += 1;
            continue;
        }
        if c == b')' {
            depth -= 1;
            i += 1;
            continue;
        }
        if depth == 0 && c == b'?' {
            q_pos = Some(i);
            break;
        }
        i += 1;
    }

    let q = q_pos?;

    // Find matching ':' for this `?` (handles nested ternaries).
    let mut depth = 1i32;
    let mut in_string = false;
    let mut escape = false;
    let mut i = q + 1;
    let mut colon_pos = None;
    while i < bytes.len() {
        let c = bytes[i];
        if in_string {
            if escape {
                escape = false;
            } else if c == b'\\' {
                escape = true;
            } else if c == b'"' {
                in_string = false;
            }
            i += 1;
            continue;
        }
        if c == b'"' {
            in_string = true;
            i += 1;
            continue;
        }
        if c == b'(' {
            depth += 1;
            i += 1;
            continue;
        }
        if c == b')' {
            depth -= 1;
            i += 1;
            continue;
        }
        if c == b'?' {
            depth += 1;
            i += 1;
            continue;
        }
        if c == b':' {
            depth -= 1;
            if depth == 0 {
                colon_pos = Some(i);
                break;
            }
            i += 1;
            continue;
        }
        i += 1;
    }

    let colon = colon_pos?;
    let cond = src[..q].trim();
    let then_s = src[q + 1..colon].trim();
    let else_s = src[colon + 1..].trim();
    Some((cond, then_s, else_s))
}

fn eval_bool_expr(src: &str, context: &Value) -> anyhow::Result<bool> {
    let tokens = tokenize(src)?;
    let mut p = Parser {
        tokens,
        pos: 0,
        depth: 0,
    };
    let v = p.parse_or(context)?;
    if !p.is_eof() {
        anyhow::bail!("unexpected token after expression");
    }
    Ok(v)
}

#[derive(Debug, Clone, PartialEq)]
enum PathSegment {
    Key(String),
    Index(u32),
}

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Path(Vec<PathSegment>),
    Str(String),
    Number(f64),
    True,
    False,
    Null,
    EqEq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
    Not,
    LParen,
    RParen,
    In,
    LBracket,
    RBracket,
    Comma,
    FuncContains,
}

fn tokenize(src: &str) -> anyhow::Result<Vec<Token>> {
    let mut out = Vec::new();
    let bytes = src.as_bytes();
    let mut i = 0usize;

    while i < bytes.len() {
        let c = bytes[i];
        if c.is_ascii_whitespace() {
            i += 1;
            continue;
        }

        if c == b'$' {
            if bytes.get(i + 1) != Some(&b'.') {
                anyhow::bail!("path must start with '$.' at byte {}", i);
            }
            i += 2;
            if i >= bytes.len() || !(bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_' || bytes[i] == b'-') {
                anyhow::bail!("empty path after '$.'");
            }
            let mut segments: Vec<PathSegment> = Vec::new();
            loop {
                let seg_start = i;
                while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_' || bytes[i] == b'-') {
                    i += 1;
                }
                if i == seg_start {
                    anyhow::bail!("empty path segment");
                }
                segments.push(PathSegment::Key(src[seg_start..i].to_string()));

                while i < bytes.len() && bytes[i] == b'[' {
                    i += 1;
                    let idx_start = i;
                    while i < bytes.len() && bytes[i].is_ascii_digit() {
                        i += 1;
                    }
                    if i == idx_start {
                        anyhow::bail!("expected index digits inside [...]");
                    }
                    let idx: u32 = src[idx_start..i]
                        .parse()
                        .map_err(|_| anyhow::anyhow!("invalid array index"))?;
                    if bytes.get(i) != Some(&b']') {
                        anyhow::bail!("expected ']' after array index");
                    }
                    i += 1;
                    segments.push(PathSegment::Index(idx));
                }

                if i >= bytes.len() || bytes[i] != b'.' {
                    break;
                }
                i += 1;
                if i >= bytes.len() || !(bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_' || bytes[i] == b'-') {
                    anyhow::bail!("empty path segment after '.'");
                }
            }
            out.push(Token::Path(segments));
            continue;
        }

        if c == b'[' {
            out.push(Token::LBracket);
            i += 1;
            continue;
        }
        if c == b']' {
            out.push(Token::RBracket);
            i += 1;
            continue;
        }
        if c == b',' {
            out.push(Token::Comma);
            i += 1;
            continue;
        }

        if c == b'"' {
            i += 1;
            let start = i;
            let mut escape = false;
            while i < bytes.len() {
                if escape {
                    escape = false;
                    i += 1;
                    continue;
                }
                if bytes[i] == b'\\' {
                    escape = true;
                    i += 1;
                    continue;
                }
                if bytes[i] == b'"' {
                    break;
                }
                i += 1;
            }
            if i >= bytes.len() {
                anyhow::bail!("unterminated string literal");
            }
            let raw = &src[start..i];
            i += 1;
            out.push(Token::Str(unescape_json_string(raw)));
            continue;
        }

        if c == b'=' && bytes.get(i + 1) == Some(&b'=') {
            out.push(Token::EqEq);
            i += 2;
            continue;
        }
        if c == b'!' && bytes.get(i + 1) == Some(&b'=') {
            out.push(Token::Ne);
            i += 2;
            continue;
        }
        if c == b'<' && bytes.get(i + 1) == Some(&b'=') {
            out.push(Token::Le);
            i += 2;
            continue;
        }
        if c == b'>' && bytes.get(i + 1) == Some(&b'=') {
            out.push(Token::Ge);
            i += 2;
            continue;
        }
        if c == b'&' && bytes.get(i + 1) == Some(&b'&') {
            out.push(Token::And);
            i += 2;
            continue;
        }
        if c == b'|' && bytes.get(i + 1) == Some(&b'|') {
            out.push(Token::Or);
            i += 2;
            continue;
        }
        if c == b'<' {
            out.push(Token::Lt);
            i += 1;
            continue;
        }
        if c == b'>' {
            out.push(Token::Gt);
            i += 1;
            continue;
        }
        if c == b'!' {
            out.push(Token::Not);
            i += 1;
            continue;
        }
        if c == b'(' {
            out.push(Token::LParen);
            i += 1;
            continue;
        }
        if c == b')' {
            out.push(Token::RParen);
            i += 1;
            continue;
        }

        if c == b'-' || c.is_ascii_digit() {
            let start = i;
            if c == b'-' {
                i += 1;
            }
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                i += 1;
            }
            if i < bytes.len() && bytes[i] == b'.' {
                i += 1;
                while i < bytes.len() && bytes[i].is_ascii_digit() {
                    i += 1;
                }
            }
            let num: f64 = src[start..i]
                .parse()
                .map_err(|_| anyhow::anyhow!("invalid number"))?;
            out.push(Token::Number(num));
            continue;
        }

        if c.is_ascii_alphabetic() || c == b'_' {
            let start = i;
            i += 1;
            while i < bytes.len() {
                let ch = bytes[i];
                if ch.is_ascii_alphanumeric() || ch == b'_' {
                    i += 1;
                } else {
                    break;
                }
            }
            let word = &src[start..i];
            match word {
                "true" => out.push(Token::True),
                "false" => out.push(Token::False),
                "null" => out.push(Token::Null),
                "in" => out.push(Token::In),
                "contains" => out.push(Token::FuncContains),
                _ => anyhow::bail!("unknown identifier '{word}'"),
            }
            continue;
        }

        anyhow::bail!("unexpected character {:?} at {}", c as char, i);
    }

    Ok(out)
}

fn unescape_json_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars();
    while let Some(ch) = it.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        match it.next() {
            Some('"') => out.push('"'),
            Some('\\') => out.push('\\'),
            Some('/') => out.push('/'),
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('t') => out.push('\t'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

fn resolve_path(context: &Value, segments: &[PathSegment]) -> anyhow::Result<Value> {
    let mut cur: &Value = context;
    for seg in segments {
        match seg {
            PathSegment::Key(k) => match cur {
                Value::Object(m) => {
                    cur = m.get(k).unwrap_or(&Value::Null);
                }
                _ => return Ok(Value::Null),
            },
            PathSegment::Index(ix) => match cur {
                Value::Array(a) => {
                    cur = a.get(*ix as usize).unwrap_or(&Value::Null);
                }
                _ => return Ok(Value::Null),
            },
        }
    }
    Ok(cur.clone())
}

fn eval_in_membership(lhs: &Value, rhs: &Value) -> anyhow::Result<bool> {
    let Some(arr) = rhs.as_array() else {
        anyhow::bail!("right-hand side of `in` must be a JSON array");
    };
    Ok(arr.iter().any(|e| e == lhs))
}

fn value_as_search_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Null => "null".into(),
        _ => v.to_string(),
    }
}

fn eval_contains_call(a: &Value, b: &Value) -> anyhow::Result<bool> {
    let hay = value_as_search_text(a);
    let needle = value_as_search_text(b);
    Ok(hay.contains(&needle))
}

fn cmp_values(op: &Token, left: &Value, right: &Value) -> anyhow::Result<bool> {
    use std::cmp::Ordering;
    let ord = compare_values(left, right)?;
    Ok(match op {
        Token::EqEq => ord == Ordering::Equal,
        Token::Ne => ord != Ordering::Equal,
        Token::Lt => ord == Ordering::Less,
        Token::Le => ord != Ordering::Greater,
        Token::Gt => ord == Ordering::Greater,
        Token::Ge => ord != Ordering::Less,
        _ => anyhow::bail!("internal: not a comparison op"),
    })
}

fn compare_values(left: &Value, right: &Value) -> anyhow::Result<std::cmp::Ordering> {
    use std::cmp::Ordering;
    match (left, right) {
        (Value::Null, Value::Null) => Ok(Ordering::Equal),
        (Value::Null, _) => Ok(Ordering::Less),
        (_, Value::Null) => Ok(Ordering::Greater),
        (Value::Bool(a), Value::Bool(b)) => Ok(a.cmp(b)),
        (Value::Number(a), Value::Number(b)) => {
            let af = a.as_f64().unwrap_or(0.0);
            let bf = b.as_f64().unwrap_or(0.0);
            Ok(af.partial_cmp(&bf).unwrap_or(Ordering::Equal))
        }
        (Value::String(a), Value::String(b)) => Ok(a.cmp(b)),
        (Value::Number(a), Value::String(b)) => {
            let af = a.as_f64().unwrap_or(0.0);
            if let Ok(bf) = b.parse::<f64>() {
                Ok(af.partial_cmp(&bf).unwrap_or(Ordering::Equal))
            } else {
                Ok(af.to_string().cmp(b))
            }
        }
        (Value::String(a), Value::Number(b)) => {
            let bf = b.as_f64().unwrap_or(0.0);
            if let Ok(af) = a.parse::<f64>() {
                Ok(af.partial_cmp(&bf).unwrap_or(Ordering::Equal))
            } else {
                Ok(a.cmp(&bf.to_string()))
            }
        }
        _ => Ok(left.to_string().cmp(&right.to_string())),
    }
}

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
    depth: usize,
}

impl Parser {
    fn enter(&mut self) -> anyhow::Result<()> {
        self.depth += 1;
        if self.depth > MAX_RECURSION_DEPTH {
            anyhow::bail!("expression exceeds maximum nesting depth ({MAX_RECURSION_DEPTH})");
        }
        Ok(())
    }

    fn leave(&mut self) {
        self.depth = self.depth.saturating_sub(1);
    }

    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }

    fn bump(&mut self) -> Option<Token> {
        let t = self.tokens.get(self.pos).cloned();
        if t.is_some() {
            self.pos += 1;
        }
        t
    }

    fn is_eof(&self) -> bool {
        self.pos >= self.tokens.len()
    }

    fn parse_or(&mut self, ctx: &Value) -> anyhow::Result<bool> {
        self.enter()?;
        let result = (|| -> anyhow::Result<bool> {
            let mut v = self.parse_and(ctx)?;
            while matches!(self.peek(), Some(Token::Or)) {
                self.bump();
                let rhs = self.parse_and(ctx)?;
                v = v || rhs;
            }
            Ok(v)
        })();
        self.leave();
        result
    }

    fn parse_and(&mut self, ctx: &Value) -> anyhow::Result<bool> {
        let mut v = self.parse_cmp(ctx)?;
        while matches!(self.peek(), Some(Token::And)) {
            self.bump();
            let rhs = self.parse_cmp(ctx)?;
            v = v && rhs;
        }
        Ok(v)
    }

    fn parse_cmp(&mut self, ctx: &Value) -> anyhow::Result<bool> {
        let lhs = self.parse_unary_value(ctx)?;
        if matches!(self.peek(), Some(Token::In)) {
            self.bump();
            let rhs = self.parse_unary_value(ctx)?;
            return eval_in_membership(&lhs, &rhs);
        }
        if matches!(
            self.peek(),
            Some(Token::EqEq | Token::Ne | Token::Lt | Token::Le | Token::Gt | Token::Ge)
        ) {
            let op = self
                .bump()
                .ok_or_else(|| anyhow::anyhow!("unexpected end of expression"))?;
            let rhs = self.parse_unary_value(ctx)?;
            return cmp_values(&op, &lhs, &rhs);
        }
        Ok(as_bool(&lhs))
    }

    fn parse_unary_value(&mut self, ctx: &Value) -> anyhow::Result<Value> {
        if matches!(self.peek(), Some(Token::Not)) {
            self.bump();
            let inner = self.parse_unary_value(ctx)?;
            return Ok(Value::Bool(!as_bool(&inner)));
        }
        self.parse_primary(ctx)
    }

    fn parse_primary(&mut self, ctx: &Value) -> anyhow::Result<Value> {
        match self.peek().cloned() {
            Some(Token::FuncContains) => {
                self.bump();
                match self.bump() {
                    Some(Token::LParen) => {}
                    _ => anyhow::bail!("expected '(' after contains"),
                }
                let a = self.parse_unary_value(ctx)?;
                match self.bump() {
                    Some(Token::Comma) => {}
                    _ => anyhow::bail!("expected ',' in contains(...)"),
                }
                let b = self.parse_unary_value(ctx)?;
                match self.bump() {
                    Some(Token::RParen) => {}
                    _ => anyhow::bail!("expected ')' after contains(...)"),
                }
                Ok(Value::Bool(eval_contains_call(&a, &b)?))
            }
            Some(Token::LBracket) => {
                self.bump();
                let mut items = Vec::new();
                if matches!(self.peek(), Some(Token::RBracket)) {
                    self.bump();
                    return Ok(Value::Array(items));
                }
                loop {
                    items.push(self.parse_unary_value(ctx)?);
                    match self.peek() {
                        Some(Token::Comma) => {
                            self.bump();
                        }
                        Some(Token::RBracket) => {
                            self.bump();
                            return Ok(Value::Array(items));
                        }
                        _ => anyhow::bail!("expected ',' or ']' in array literal"),
                    }
                }
            }
            Some(Token::LParen) => {
                self.bump();
                self.enter()?;
                let inner = self.parse_or(ctx);
                self.leave();
                let v = inner?;
                match self.bump() {
                    Some(Token::RParen) => Ok(Value::Bool(v)),
                    _ => anyhow::bail!("expected ')'"),
                }
            }
            Some(Token::Path(segs)) => {
                self.bump();
                resolve_path(ctx, &segs)
            }
            Some(Token::Str(s)) => {
                self.bump();
                Ok(Value::String(s))
            }
            Some(Token::Number(n)) => {
                self.bump();
                Ok(serde_json::Number::from_f64(n)
                    .map(Value::Number)
                    .unwrap_or(Value::Null))
            }
            Some(Token::True) => {
                self.bump();
                Ok(Value::Bool(true))
            }
            Some(Token::False) => {
                self.bump();
                Ok(Value::Bool(false))
            }
            Some(Token::Null) => {
                self.bump();
                Ok(Value::Null)
            }
            Some(other) => anyhow::bail!("unexpected token in expression: {:?}", other),
            None => anyhow::bail!("unexpected end of expression"),
        }
    }
}

fn as_bool(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().unwrap_or(0.0) != 0.0,
        Value::String(s) => !s.is_empty() && s != "false" && s != "0",
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn cmp_and_ternary() {
        let ctx = json!({
            "input": { "score": 0.9, "lang": "rust" },
            "n1": { "ok": true }
        });
        assert_eq!(
            evaluate_condition(r#"$.input.score > 0.8 ? "high" : "low""#, &ctx).unwrap(),
            "high"
        );
        assert_eq!(
            evaluate_condition(r#"$.input.score < 0.8 ? "high" : "low""#, &ctx).unwrap(),
            "low"
        );
        assert_eq!(
            evaluate_condition(r#"$.input.lang == "rust""#, &ctx).unwrap(),
            "true"
        );
        assert_eq!(
            evaluate_condition(r#"$.input.lang != "go""#, &ctx).unwrap(),
            "true"
        );
        assert_eq!(
            evaluate_condition(r#"$.n1.ok && $.input.score >= 0.9"#, &ctx).unwrap(),
            "true"
        );
        assert_eq!(
            evaluate_condition(r#"!($.input.lang == "go")"#, &ctx).unwrap(),
            "true"
        );
    }

    #[test]
    fn array_index_path() {
        let ctx = json!({
            "data": [10, 20, 30],
            "items": [ { "name": "a" }, { "name": "b" } ]
        });
        assert_eq!(
            evaluate_condition(r#"$.data[1] == 20"#, &ctx).unwrap(),
            "true"
        );
        assert_eq!(
            evaluate_condition(r#"$.items[1].name == "b""#, &ctx).unwrap(),
            "true"
        );
    }

    #[test]
    fn in_operator_membership() {
        let ctx = json!({
            "input": { "status": "pending" }
        });
        assert!(evaluate_bool(
            r#"$.input.status in ["active", "pending"]"#,
            &ctx
        )
        .unwrap());
        assert!(!evaluate_bool(r#"$.input.status in ["done"]"#, &ctx).unwrap());
    }

    #[test]
    fn contains_function() {
        let ctx = json!({
            "input": { "name": "my_test_case" }
        });
        assert!(evaluate_bool(r#"contains($.input.name, "test")"#, &ctx).unwrap());
        assert!(!evaluate_bool(r#"contains($.input.name, "nope")"#, &ctx).unwrap());
    }

    #[test]
    fn expression_too_long() {
        let long = format!("$.x == {}", "1".repeat(5000));
        let ctx = json!({"x": 1});
        assert!(evaluate_condition(&long, &ctx).is_err());
    }

    #[test]
    fn deep_nesting_rejected() {
        let open: String = "(".repeat(50);
        let close: String = ")".repeat(50);
        let expr = format!("{open}true{close}");
        let ctx = json!({});
        assert!(evaluate_condition(&expr, &ctx).is_err());
    }
}
