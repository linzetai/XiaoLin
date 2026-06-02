//! Shell command AST built from tree-sitter's bash grammar.
//!
//! Converts tree-sitter's concrete syntax tree into a high-level AST
//! that accurately represents:
//! - Simple commands with arguments and redirections
//! - Pipelines (`|`)
//! - Logical chains (`&&`, `||`)
//! - Semicolons (`;`)
//! - Subshells `( ... )`
//! - Command substitution `$( ... )`
//! - If / For / While / Case constructs
//! - Heredocs
//! - Function definitions
//! - Quoting context (single-quote, double-quote, heredoc body)

use std::fmt;

use crate::parser::CodeParser;

// ── AST Node Types ───────────────────────────────────────────────────

/// High-level shell AST.
#[derive(Debug, Clone, PartialEq)]
pub enum ShellAst {
    /// A simple command: `name arg1 arg2 ...`
    Command {
        name: String,
        args: Vec<ShellArg>,
        redirections: Vec<Redirection>,
        /// Trailing `&` (background)
        background: bool,
    },
    /// `cmd1 | cmd2 | cmd3`
    Pipeline(Vec<ShellAst>),
    /// `cmd1 && cmd2`
    And(Box<ShellAst>, Box<ShellAst>),
    /// `cmd1 || cmd2`
    Or(Box<ShellAst>, Box<ShellAst>),
    /// `cmd1 ; cmd2`
    Sequence(Vec<ShellAst>),
    /// `( ... )`
    Subshell(Box<ShellAst>),
    /// Function definition: `name() { body }`
    Function { name: String, body: Box<ShellAst> },
    /// `if cond; then body; [elif cond; then body;]* [else body;] fi`
    If {
        condition: Box<ShellAst>,
        then_body: Box<ShellAst>,
        elif_branches: Vec<(ShellAst, ShellAst)>,
        else_body: Option<Box<ShellAst>>,
    },
    /// `for var in words; do body; done`
    For {
        variable: String,
        words: Vec<String>,
        body: Box<ShellAst>,
    },
    /// `while cond; do body; done`
    While {
        condition: Box<ShellAst>,
        body: Box<ShellAst>,
    },
    /// `case word in pattern) body;; esac`
    Case { word: String, arms: Vec<CaseArm> },
    /// Variable assignment: `VAR=value`
    Assignment { name: String, value: String },
    /// A compound list (multiple statements in a block).
    CompoundList(Vec<ShellAst>),
    /// Unparseable or unsupported node, preserved as raw text.
    Raw(String),
}

/// A single case arm: `pattern) body ;;`
#[derive(Debug, Clone, PartialEq)]
pub struct CaseArm {
    pub pattern: String,
    pub body: ShellAst,
}

/// A shell argument with quoting context.
#[derive(Debug, Clone, PartialEq)]
pub enum ShellArg {
    /// Literal (unquoted or partially expanded).
    Literal(String),
    /// Single-quoted string — no expansion inside.
    SingleQuoted(String),
    /// Double-quoted string — expansions possible inside.
    DoubleQuoted(String),
    /// Command substitution `$( ... )`
    CommandSubstitution(Box<ShellAst>),
    /// Heredoc body.
    Heredoc {
        delimiter: String,
        body: String,
        quoted: bool,
    },
}

impl ShellArg {
    pub fn text(&self) -> &str {
        match self {
            Self::Literal(s) | Self::SingleQuoted(s) | Self::DoubleQuoted(s) => s,
            Self::CommandSubstitution(_) => "<cmd-sub>",
            Self::Heredoc { body, .. } => body,
        }
    }

    pub fn is_single_quoted(&self) -> bool {
        matches!(self, Self::SingleQuoted(_))
    }

    pub fn is_command_substitution(&self) -> bool {
        matches!(self, Self::CommandSubstitution(_))
    }
}

/// I/O redirection.
#[derive(Debug, Clone, PartialEq)]
pub struct Redirection {
    pub fd: Option<u32>,
    pub op: RedirectOp,
    pub target: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum RedirectOp {
    /// `>`
    Write,
    /// `>>`
    Append,
    /// `<`
    Read,
    /// `2>&1`
    DupOutput,
    /// `<<` (heredoc)
    HereDoc,
    /// `<<<` (herestring)
    HereString,
}

impl fmt::Display for ShellAst {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Command { name, args, .. } => {
                write!(f, "{}", name)?;
                for a in args {
                    write!(f, " {}", a.text())?;
                }
                Ok(())
            }
            Self::Pipeline(cmds) => {
                for (i, c) in cmds.iter().enumerate() {
                    if i > 0 {
                        write!(f, " | ")?;
                    }
                    write!(f, "{c}")?;
                }
                Ok(())
            }
            Self::And(l, r) => write!(f, "{l} && {r}"),
            Self::Or(l, r) => write!(f, "{l} || {r}"),
            Self::Sequence(stmts) => {
                for (i, s) in stmts.iter().enumerate() {
                    if i > 0 {
                        write!(f, "; ")?;
                    }
                    write!(f, "{s}")?;
                }
                Ok(())
            }
            Self::Subshell(inner) => write!(f, "( {inner} )"),
            Self::Raw(s) => write!(f, "{s}"),
            _ => write!(f, "<complex>"),
        }
    }
}

// ── Parser ───────────────────────────────────────────────────────────

/// Parse a shell command string into a ShellAst.
pub fn parse_shell_ast(source: &str) -> anyhow::Result<ShellAst> {
    if !CodeParser::is_language_available("bash") {
        anyhow::bail!("tree-sitter bash parser not available");
    }

    let parsed = CodeParser::parse(source, "bash")?;
    let root = parsed.tree.root_node();

    if root.child_count() == 0 {
        return Ok(ShellAst::Raw(source.to_string()));
    }

    let mut stmts = Vec::new();
    let src_bytes = source.as_bytes();

    for i in 0..root.child_count() {
        let child = root.child(i as u32).unwrap();
        if child.is_named() {
            stmts.push(convert_node(&child, src_bytes));
        }
    }

    Ok(match stmts.len() {
        0 => ShellAst::Raw(source.to_string()),
        1 => stmts.into_iter().next().unwrap(),
        _ => ShellAst::Sequence(stmts),
    })
}

fn node_text<'a>(node: &tree_sitter::Node, src: &'a [u8]) -> &'a str {
    node.utf8_text(src).unwrap_or("")
}

fn convert_node(node: &tree_sitter::Node, src: &[u8]) -> ShellAst {
    match node.kind() {
        "command" => convert_command(node, src),
        "pipeline" => convert_pipeline(node, src),
        "list" => convert_list(node, src),
        "subshell" => convert_subshell(node, src),
        "function_definition" => convert_function(node, src),
        "if_statement" => convert_if(node, src),
        "for_statement" => convert_for(node, src),
        "while_statement" => convert_while(node, src),
        "case_statement" => convert_case(node, src),
        "variable_assignment" => convert_assignment(node, src),
        "compound_statement" => convert_compound(node, src),
        "redirected_statement" => convert_redirected(node, src),
        "negated_command" => {
            let mut inner = ShellAst::Raw("!".into());
            for i in 0..node.child_count() {
                let c = node.child(i as u32).unwrap();
                if c.is_named() {
                    inner = convert_node(&c, src);
                    break;
                }
            }
            inner
        }
        _ => ShellAst::Raw(node_text(node, src).to_string()),
    }
}

fn convert_command(node: &tree_sitter::Node, src: &[u8]) -> ShellAst {
    let mut name = String::new();
    let mut args = Vec::new();
    let mut redirections = Vec::new();

    for i in 0..node.child_count() {
        let child = node.child(i as u32).unwrap();
        match child.kind() {
            "command_name" => {
                name = node_text(&child, src).to_string();
            }
            "word" | "number" => {
                args.push(ShellArg::Literal(node_text(&child, src).to_string()));
            }
            "raw_string" | "simple_expansion" | "expansion" => {
                args.push(ShellArg::Literal(node_text(&child, src).to_string()));
            }
            "string" => {
                let text = node_text(&child, src);
                if text.starts_with('\'') {
                    let inner = text
                        .strip_prefix('\'')
                        .and_then(|s| s.strip_suffix('\''))
                        .unwrap_or(text);
                    args.push(ShellArg::SingleQuoted(inner.to_string()));
                } else {
                    let inner = text
                        .strip_prefix('"')
                        .and_then(|s| s.strip_suffix('"'))
                        .unwrap_or(text);
                    args.push(ShellArg::DoubleQuoted(inner.to_string()));
                }
            }
            "concatenation" => {
                args.push(convert_concatenation(&child, src));
            }
            "command_substitution" => {
                args.push(convert_command_substitution(&child, src));
            }
            "file_redirect" | "heredoc_redirect" => {
                if let Some(r) = convert_redirect(&child, src) {
                    redirections.push(r);
                }
            }
            _ => {
                if child.is_named() {
                    args.push(ShellArg::Literal(node_text(&child, src).to_string()));
                }
            }
        }
    }

    ShellAst::Command {
        name,
        args,
        redirections,
        background: false,
    }
}

fn convert_concatenation(node: &tree_sitter::Node, src: &[u8]) -> ShellArg {
    let mut parts = String::new();
    for i in 0..node.child_count() {
        let child = node.child(i as u32).unwrap();
        parts.push_str(node_text(&child, src));
    }
    ShellArg::Literal(parts)
}

fn convert_command_substitution(node: &tree_sitter::Node, src: &[u8]) -> ShellArg {
    for i in 0..node.child_count() {
        let child = node.child(i as u32).unwrap();
        if child.is_named() && child.kind() != "$(" {
            return ShellArg::CommandSubstitution(Box::new(convert_node(&child, src)));
        }
    }
    let text = node_text(node, src);
    ShellArg::CommandSubstitution(Box::new(ShellAst::Raw(text.to_string())))
}

fn convert_redirect(node: &tree_sitter::Node, src: &[u8]) -> Option<Redirection> {
    let text = node_text(node, src);
    let mut fd = None;
    let mut op = RedirectOp::Write;
    let mut target = String::new();

    for i in 0..node.child_count() {
        let child = node.child(i as u32).unwrap();
        match child.kind() {
            "file_descriptor" => {
                fd = node_text(&child, src).parse().ok();
            }
            ">>" => op = RedirectOp::Append,
            ">" => op = RedirectOp::Write,
            "<" => op = RedirectOp::Read,
            "<<" => op = RedirectOp::HereDoc,
            "<<<" => op = RedirectOp::HereString,
            "word" | "string" | "heredoc_body" => {
                target = node_text(&child, src).to_string();
            }
            _ => {
                if child.is_named() && target.is_empty() {
                    target = node_text(&child, src).to_string();
                }
            }
        }
    }

    if target.is_empty() && !text.is_empty() {
        target = text.to_string();
    }

    Some(Redirection { fd, op, target })
}

fn convert_pipeline(node: &tree_sitter::Node, src: &[u8]) -> ShellAst {
    let mut commands = Vec::new();
    for i in 0..node.child_count() {
        let child = node.child(i as u32).unwrap();
        if child.is_named() {
            commands.push(convert_node(&child, src));
        }
    }
    if commands.len() == 1 {
        commands.into_iter().next().unwrap()
    } else {
        ShellAst::Pipeline(commands)
    }
}

fn convert_list(node: &tree_sitter::Node, src: &[u8]) -> ShellAst {
    let mut items = Vec::new();
    let mut operators = Vec::new();

    for i in 0..node.child_count() {
        let child = node.child(i as u32).unwrap();
        if child.is_named() {
            items.push(convert_node(&child, src));
        } else {
            let op = node_text(&child, src);
            if op == "&&" || op == "||" || op == ";" {
                operators.push(op.to_string());
            }
        }
    }

    if items.len() < 2 {
        return items
            .into_iter()
            .next()
            .unwrap_or(ShellAst::Raw(String::new()));
    }

    let mut result = items.remove(0);
    for (i, item) in items.into_iter().enumerate() {
        let op = operators.get(i).map(|s| s.as_str()).unwrap_or(";");
        result = match op {
            "&&" => ShellAst::And(Box::new(result), Box::new(item)),
            "||" => ShellAst::Or(Box::new(result), Box::new(item)),
            _ => match result {
                ShellAst::Sequence(ref mut v) => {
                    v.push(item);
                    continue;
                }
                _ => ShellAst::Sequence(vec![result, item]),
            },
        };
    }

    result
}

fn convert_subshell(node: &tree_sitter::Node, src: &[u8]) -> ShellAst {
    let mut inner = ShellAst::Raw(String::new());
    for i in 0..node.child_count() {
        let child = node.child(i as u32).unwrap();
        if child.is_named() {
            inner = convert_node(&child, src);
            break;
        }
    }
    ShellAst::Subshell(Box::new(inner))
}

fn convert_function(node: &tree_sitter::Node, src: &[u8]) -> ShellAst {
    let mut name = String::new();
    let mut body = ShellAst::Raw(String::new());

    for i in 0..node.child_count() {
        let child = node.child(i as u32).unwrap();
        match child.kind() {
            "word" => name = node_text(&child, src).to_string(),
            "compound_statement" => body = convert_compound(&child, src),
            _ => {}
        }
    }

    ShellAst::Function {
        name,
        body: Box::new(body),
    }
}

fn convert_compound(node: &tree_sitter::Node, src: &[u8]) -> ShellAst {
    let mut stmts = Vec::new();
    for i in 0..node.child_count() {
        let child = node.child(i as u32).unwrap();
        if child.is_named() {
            stmts.push(convert_node(&child, src));
        }
    }
    match stmts.len() {
        0 => ShellAst::CompoundList(Vec::new()),
        1 => stmts.into_iter().next().unwrap(),
        _ => ShellAst::CompoundList(stmts),
    }
}

fn convert_if(node: &tree_sitter::Node, src: &[u8]) -> ShellAst {
    let mut condition = ShellAst::Raw(String::new());
    let mut then_body = ShellAst::Raw(String::new());
    let mut elif_branches = Vec::new();
    let mut else_body = None;

    let mut in_condition = false;
    let mut in_then = false;
    let mut in_elif_cond = false;
    let mut in_elif_body = false;
    let mut elif_cond = ShellAst::Raw(String::new());

    for i in 0..node.child_count() {
        let child = node.child(i as u32).unwrap();
        let text = node_text(&child, src);

        match text {
            "if" => in_condition = true,
            "then" => {
                if in_elif_cond {
                    in_elif_cond = false;
                    in_elif_body = true;
                } else {
                    in_condition = false;
                    in_then = true;
                }
            }
            "elif" => {
                in_then = false;
                in_elif_body = false;
                in_elif_cond = true;
            }
            "else" => {
                in_then = false;
                in_elif_body = false;
            }
            "fi" => {}
            _ if child.is_named() => {
                let ast = convert_node(&child, src);
                if in_condition {
                    condition = ast;
                } else if in_then {
                    then_body = ast;
                } else if in_elif_cond {
                    elif_cond = ast;
                } else if in_elif_body {
                    elif_branches.push((elif_cond.clone(), ast));
                    elif_cond = ShellAst::Raw(String::new());
                } else {
                    else_body = Some(Box::new(ast));
                }
            }
            _ => {}
        }
    }

    ShellAst::If {
        condition: Box::new(condition),
        then_body: Box::new(then_body),
        elif_branches,
        else_body,
    }
}

fn convert_for(node: &tree_sitter::Node, src: &[u8]) -> ShellAst {
    let mut variable = String::new();
    let mut words = Vec::new();
    let mut body = ShellAst::Raw(String::new());
    let mut in_words = false;

    for i in 0..node.child_count() {
        let child = node.child(i as u32).unwrap();
        let text = node_text(&child, src);

        match child.kind() {
            "variable_name" => variable = text.to_string(),
            "word" => {
                if variable.is_empty() {
                    variable = text.to_string();
                } else if in_words {
                    words.push(text.to_string());
                }
            }
            "do_group" => {
                body = convert_do_group(&child, src);
            }
            _ => {
                if text == "in" {
                    in_words = true;
                } else if text == "do" {
                    in_words = false;
                }
            }
        }
    }

    ShellAst::For {
        variable,
        words,
        body: Box::new(body),
    }
}

fn convert_while(node: &tree_sitter::Node, src: &[u8]) -> ShellAst {
    let mut condition = ShellAst::Raw(String::new());
    let mut body = ShellAst::Raw(String::new());
    let mut found_do = false;

    for i in 0..node.child_count() {
        let child = node.child(i as u32).unwrap();
        let text = node_text(&child, src);

        if text == "do" || child.kind() == "do_group" {
            found_do = true;
            if child.kind() == "do_group" {
                body = convert_do_group(&child, src);
            }
        } else if child.is_named() && !found_do {
            condition = convert_node(&child, src);
        }
    }

    ShellAst::While {
        condition: Box::new(condition),
        body: Box::new(body),
    }
}

fn convert_do_group(node: &tree_sitter::Node, src: &[u8]) -> ShellAst {
    let mut stmts = Vec::new();
    for i in 0..node.child_count() {
        let child = node.child(i as u32).unwrap();
        if child.is_named() {
            stmts.push(convert_node(&child, src));
        }
    }
    match stmts.len() {
        0 => ShellAst::CompoundList(Vec::new()),
        1 => stmts.into_iter().next().unwrap(),
        _ => ShellAst::CompoundList(stmts),
    }
}

fn convert_case(node: &tree_sitter::Node, src: &[u8]) -> ShellAst {
    let mut word = String::new();
    let mut arms = Vec::new();

    for i in 0..node.child_count() {
        let child = node.child(i as u32).unwrap();
        match child.kind() {
            "word" | "string" => {
                if word.is_empty() {
                    word = node_text(&child, src).to_string();
                }
            }
            "case_item" => {
                let (pattern, body) = convert_case_item(&child, src);
                arms.push(CaseArm { pattern, body });
            }
            _ => {}
        }
    }

    ShellAst::Case { word, arms }
}

fn convert_case_item(node: &tree_sitter::Node, src: &[u8]) -> (String, ShellAst) {
    let mut pattern = String::new();
    let mut body = ShellAst::Raw(String::new());

    for i in 0..node.child_count() {
        let child = node.child(i as u32).unwrap();
        match child.kind() {
            "word" | "concatenation" | "string" | "extglob_pattern" => {
                if pattern.is_empty() {
                    pattern = node_text(&child, src).to_string();
                } else {
                    body = convert_node(&child, src);
                }
            }
            _ if child.is_named() => {
                body = convert_node(&child, src);
            }
            _ => {}
        }
    }

    (pattern, body)
}

fn convert_assignment(node: &tree_sitter::Node, src: &[u8]) -> ShellAst {
    let mut name = String::new();
    let mut value = String::new();

    for i in 0..node.child_count() {
        let child = node.child(i as u32).unwrap();
        match child.kind() {
            "variable_name" => name = node_text(&child, src).to_string(),
            _ if child.is_named() => value = node_text(&child, src).to_string(),
            _ => {}
        }
    }

    ShellAst::Assignment { name, value }
}

fn convert_redirected(node: &tree_sitter::Node, src: &[u8]) -> ShellAst {
    let mut inner = None;
    let mut redirections = Vec::new();

    for i in 0..node.child_count() {
        let child = node.child(i as u32).unwrap();
        match child.kind() {
            "file_redirect" | "heredoc_redirect" | "herestring_redirect" => {
                if let Some(r) = convert_redirect(&child, src) {
                    redirections.push(r);
                }
            }
            _ if child.is_named() => {
                inner = Some(convert_node(&child, src));
            }
            _ => {}
        }
    }

    match inner {
        Some(ShellAst::Command {
            name,
            args,
            redirections: mut existing,
            background,
        }) => {
            existing.extend(redirections);
            ShellAst::Command {
                name,
                args,
                redirections: existing,
                background,
            }
        }
        Some(other) => other,
        None => ShellAst::Raw(node_text(node, src).to_string()),
    }
}

// ── Utility Functions ────────────────────────────────────────────────

/// Extract all command names from an AST (recursive).
pub fn extract_command_names(ast: &ShellAst) -> Vec<String> {
    let mut names = Vec::new();
    collect_command_names(ast, &mut names);
    names
}

fn collect_command_names(ast: &ShellAst, names: &mut Vec<String>) {
    match ast {
        ShellAst::Command { name, args, .. } => {
            if !name.is_empty() {
                names.push(name.clone());
            }
            for arg in args {
                if let ShellArg::CommandSubstitution(inner) = arg {
                    collect_command_names(inner, names);
                }
            }
        }
        ShellAst::Pipeline(cmds) => {
            for c in cmds {
                collect_command_names(c, names);
            }
        }
        ShellAst::And(l, r) | ShellAst::Or(l, r) => {
            collect_command_names(l, names);
            collect_command_names(r, names);
        }
        ShellAst::Sequence(stmts) | ShellAst::CompoundList(stmts) => {
            for s in stmts {
                collect_command_names(s, names);
            }
        }
        ShellAst::Subshell(inner) => collect_command_names(inner, names),
        ShellAst::Function { body, .. } => collect_command_names(body, names),
        ShellAst::If {
            condition,
            then_body,
            elif_branches,
            else_body,
        } => {
            collect_command_names(condition, names);
            collect_command_names(then_body, names);
            for (c, b) in elif_branches {
                collect_command_names(c, names);
                collect_command_names(b, names);
            }
            if let Some(e) = else_body {
                collect_command_names(e, names);
            }
        }
        ShellAst::For { body, .. } => collect_command_names(body, names),
        ShellAst::While { condition, body } => {
            collect_command_names(condition, names);
            collect_command_names(body, names);
        }
        ShellAst::Case { arms, .. } => {
            for arm in arms {
                collect_command_names(&arm.body, names);
            }
        }
        ShellAst::Assignment { .. } | ShellAst::Raw(_) => {}
    }
}

/// Check if the AST contains any command substitutions `$(...)`.
pub fn has_command_substitution(ast: &ShellAst) -> bool {
    match ast {
        ShellAst::Command { args, .. } => args
            .iter()
            .any(|a| matches!(a, ShellArg::CommandSubstitution(_))),
        ShellAst::Pipeline(cmds) => cmds.iter().any(has_command_substitution),
        ShellAst::And(l, r) | ShellAst::Or(l, r) => {
            has_command_substitution(l) || has_command_substitution(r)
        }
        ShellAst::Sequence(stmts) | ShellAst::CompoundList(stmts) => {
            stmts.iter().any(has_command_substitution)
        }
        ShellAst::Subshell(inner) => has_command_substitution(inner),
        _ => false,
    }
}

/// Return the nesting depth of subshells/command substitutions.
pub fn nesting_depth(ast: &ShellAst) -> usize {
    match ast {
        ShellAst::Command { args, .. } => args
            .iter()
            .filter_map(|a| {
                if let ShellArg::CommandSubstitution(inner) = a {
                    Some(1 + nesting_depth(inner))
                } else {
                    None
                }
            })
            .max()
            .unwrap_or(0),
        ShellAst::Subshell(inner) => 1 + nesting_depth(inner),
        ShellAst::Pipeline(cmds) => cmds.iter().map(nesting_depth).max().unwrap_or(0),
        ShellAst::And(l, r) | ShellAst::Or(l, r) => nesting_depth(l).max(nesting_depth(r)),
        ShellAst::Sequence(stmts) | ShellAst::CompoundList(stmts) => {
            stmts.iter().map(nesting_depth).max().unwrap_or(0)
        }
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn skip_if_no_bash() -> bool {
        !CodeParser::is_language_available("bash")
    }

    // ── Simple Command ──

    #[test]
    fn parse_simple_command() {
        if skip_if_no_bash() {
            return;
        }
        let ast = parse_shell_ast("echo hello world").unwrap();
        match &ast {
            ShellAst::Command { name, args, .. } => {
                assert_eq!(name, "echo");
                assert_eq!(args.len(), 2);
            }
            other => panic!("expected Command, got: {other:?}"),
        }
    }

    #[test]
    fn parse_command_no_args() {
        if skip_if_no_bash() {
            return;
        }
        let ast = parse_shell_ast("ls").unwrap();
        assert!(matches!(ast, ShellAst::Command { .. }));
    }

    // ── Pipeline ──

    #[test]
    fn parse_pipeline() {
        if skip_if_no_bash() {
            return;
        }
        let ast = parse_shell_ast("cat file.txt | grep pattern | head -5").unwrap();
        match &ast {
            ShellAst::Pipeline(cmds) => {
                assert_eq!(cmds.len(), 3);
                let names = extract_command_names(&ast);
                assert_eq!(names, vec!["cat", "grep", "head"]);
            }
            other => panic!("expected Pipeline, got: {other:?}"),
        }
    }

    // ── Logical Chains ──

    #[test]
    fn parse_and_chain() {
        if skip_if_no_bash() {
            return;
        }
        let ast = parse_shell_ast("mkdir -p dir && cd dir").unwrap();
        assert!(matches!(ast, ShellAst::And(_, _)));
        let names = extract_command_names(&ast);
        assert_eq!(names, vec!["mkdir", "cd"]);
    }

    #[test]
    fn parse_or_chain() {
        if skip_if_no_bash() {
            return;
        }
        let ast = parse_shell_ast("test -f file || echo missing").unwrap();
        assert!(matches!(ast, ShellAst::Or(_, _)));
    }

    // ── Semicolons ──

    #[test]
    fn parse_semicolons() {
        if skip_if_no_bash() {
            return;
        }
        let ast = parse_shell_ast("echo a; echo b; echo c").unwrap();
        let names = extract_command_names(&ast);
        assert_eq!(names, vec!["echo", "echo", "echo"]);
    }

    // ── Quoting Context ──

    #[test]
    fn parse_single_quoted() {
        if skip_if_no_bash() {
            return;
        }
        let ast = parse_shell_ast("echo 'hello world'").unwrap();
        if let ShellAst::Command { args, .. } = &ast {
            assert!(
                args.iter().any(|a| a.is_single_quoted()),
                "expected single-quoted arg"
            );
        } else {
            panic!("expected Command");
        }
    }

    #[test]
    fn parse_double_quoted() {
        if skip_if_no_bash() {
            return;
        }
        let ast = parse_shell_ast("echo \"hello $USER\"").unwrap();
        if let ShellAst::Command { args, .. } = &ast {
            assert!(args
                .iter()
                .any(|a| matches!(a, ShellArg::DoubleQuoted(_) | ShellArg::Literal(_))));
        } else {
            panic!("expected Command");
        }
    }

    // ── Command Substitution ──

    #[test]
    fn parse_command_substitution() {
        if skip_if_no_bash() {
            return;
        }
        let ast = parse_shell_ast("echo $(whoami)").unwrap();
        assert!(has_command_substitution(&ast));
    }

    #[test]
    fn parse_nested_command_substitution() {
        if skip_if_no_bash() {
            return;
        }
        let ast = parse_shell_ast("echo $(echo $(whoami))").unwrap();
        assert!(has_command_substitution(&ast));
        assert!(nesting_depth(&ast) >= 2);
    }

    // ── Subshell ──

    #[test]
    fn parse_subshell() {
        if skip_if_no_bash() {
            return;
        }
        let ast = parse_shell_ast("(cd /tmp && ls)").unwrap();
        assert!(matches!(ast, ShellAst::Subshell(_)));
    }

    // ── If ──

    #[test]
    fn parse_if_statement() {
        if skip_if_no_bash() {
            return;
        }
        let ast = parse_shell_ast("if test -f foo; then echo found; fi").unwrap();
        assert!(matches!(ast, ShellAst::If { .. }));
    }

    // ── For ──

    #[test]
    fn parse_for_loop() {
        if skip_if_no_bash() {
            return;
        }
        let ast = parse_shell_ast("for f in a b c; do echo $f; done").unwrap();
        if let ShellAst::For {
            variable, words, ..
        } = &ast
        {
            assert_eq!(variable, "f");
            assert_eq!(words, &["a", "b", "c"]);
        } else {
            panic!("expected For, got: {ast:?}");
        }
    }

    // ── While ──

    #[test]
    fn parse_while_loop() {
        if skip_if_no_bash() {
            return;
        }
        let ast = parse_shell_ast("while true; do echo loop; done").unwrap();
        assert!(matches!(ast, ShellAst::While { .. }));
    }

    // ── Case ──

    #[test]
    fn parse_case_statement() {
        if skip_if_no_bash() {
            return;
        }
        let ast = parse_shell_ast("case $x in\n  a) echo A;;\n  b) echo B;;\nesac").unwrap();
        if let ShellAst::Case { word, arms } = &ast {
            assert_eq!(word, "$x");
            assert_eq!(arms.len(), 2);
        } else {
            panic!("expected Case, got: {ast:?}");
        }
    }

    // ── Function ──

    #[test]
    fn parse_function_definition() {
        if skip_if_no_bash() {
            return;
        }
        let ast = parse_shell_ast("greet() { echo hello; }").unwrap();
        if let ShellAst::Function { name, .. } = &ast {
            assert_eq!(name, "greet");
        } else {
            panic!("expected Function, got: {ast:?}");
        }
    }

    // ── Redirections ──

    #[test]
    fn parse_redirect_output() {
        if skip_if_no_bash() {
            return;
        }
        let ast = parse_shell_ast("echo hello > output.txt").unwrap();
        if let ShellAst::Command { redirections, .. } = &ast {
            assert!(!redirections.is_empty());
        } else {
            panic!("expected Command with redirections, got: {ast:?}");
        }
    }

    #[test]
    fn parse_redirect_append() {
        if skip_if_no_bash() {
            return;
        }
        let ast = parse_shell_ast("echo hello >> output.txt").unwrap();
        if let ShellAst::Command { redirections, .. } = &ast {
            assert!(redirections.iter().any(|r| r.op == RedirectOp::Append));
        } else {
            panic!("expected Command with redirections, got: {ast:?}");
        }
    }

    // ── Assignment ──

    #[test]
    fn parse_assignment() {
        if skip_if_no_bash() {
            return;
        }
        let ast = parse_shell_ast("FOO=bar").unwrap();
        if let ShellAst::Assignment { name, value } = &ast {
            assert_eq!(name, "FOO");
            assert_eq!(value, "bar");
        } else {
            panic!("expected Assignment, got: {ast:?}");
        }
    }

    // ── Utility Functions ──

    #[test]
    fn extract_names_complex() {
        if skip_if_no_bash() {
            return;
        }
        let ast = parse_shell_ast("(git add . && git commit -m msg) || echo fail").unwrap();
        let names = extract_command_names(&ast);
        assert!(names.contains(&"git".to_string()));
        assert!(names.contains(&"echo".to_string()));
    }

    #[test]
    fn nesting_depth_flat() {
        if skip_if_no_bash() {
            return;
        }
        let ast = parse_shell_ast("echo hello").unwrap();
        assert_eq!(nesting_depth(&ast), 0);
    }

    #[test]
    fn nesting_depth_subshell() {
        if skip_if_no_bash() {
            return;
        }
        let ast = parse_shell_ast("(echo hello)").unwrap();
        assert_eq!(nesting_depth(&ast), 1);
    }

    #[test]
    fn single_quote_no_expansion() {
        if skip_if_no_bash() {
            return;
        }
        let ast = parse_shell_ast("echo '$(dangerous)'").unwrap();
        assert!(
            !has_command_substitution(&ast),
            "single-quoted $() should not be a command substitution"
        );
        if let ShellAst::Command { args, .. } = &ast {
            assert!(args.iter().any(|a| a.is_single_quoted()));
        }
    }

    #[test]
    fn display_simple_command() {
        if skip_if_no_bash() {
            return;
        }
        let ast = parse_shell_ast("echo hello").unwrap();
        let s = format!("{ast}");
        assert!(s.contains("echo"));
        assert!(s.contains("hello"));
    }
}
