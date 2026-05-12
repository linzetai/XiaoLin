use serde::{Deserialize, Serialize};
use tree_sitter::{Node, Tree};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SymbolKind {
    Function,
    Method,
    Class,
    Struct,
    Enum,
    Interface,
    Trait,
    Module,
    Constant,
    Variable,
    Type,
    Import,
    Other(String),
}

impl std::fmt::Display for SymbolKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Other(s) => write!(f, "{s}"),
            _ => {
                let s = serde_json::to_value(self)
                    .ok()
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_else(|| format!("{self:?}"));
                write!(f, "{s}")
            }
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Symbol {
    pub name: String,
    pub kind: SymbolKind,
    pub start_line: usize,
    pub end_line: usize,
    pub start_col: usize,
    pub signature: String,
}

pub fn extract_symbols(tree: &Tree, source: &str, language: &str) -> Vec<Symbol> {
    let root = tree.root_node();
    let mut symbols = Vec::new();
    collect_symbols(&root, source, language, &mut symbols);
    symbols
}

fn collect_symbols(node: &Node, source: &str, language: &str, out: &mut Vec<Symbol>) {
    if let Some(sym) = node_to_symbol(node, source, language) {
        out.push(sym);
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_symbols(&child, source, language, out);
    }
}

fn node_to_symbol(node: &Node, source: &str, language: &str) -> Option<Symbol> {
    let kind = node.kind();
    let (symbol_kind, name_field) = match language {
        "rust" => match kind {
            "function_item" => (SymbolKind::Function, "name"),
            "struct_item" => (SymbolKind::Struct, "name"),
            "enum_item" => (SymbolKind::Enum, "name"),
            "trait_item" => (SymbolKind::Trait, "name"),
            "impl_item" => {
                let type_node = node.child_by_field_name("type")?;
                let name = node_text(&type_node, source);
                return Some(Symbol {
                    name,
                    kind: SymbolKind::Other("impl".to_string()),
                    start_line: node.start_position().row + 1,
                    end_line: node.end_position().row + 1,
                    start_col: node.start_position().column + 1,
                    signature: extract_signature(node, source),
                });
            }
            "type_item" => (SymbolKind::Type, "name"),
            "const_item" | "static_item" => (SymbolKind::Constant, "name"),
            "mod_item" => (SymbolKind::Module, "name"),
            "use_declaration" => {
                return Some(Symbol {
                    name: node_text(node, source),
                    kind: SymbolKind::Import,
                    start_line: node.start_position().row + 1,
                    end_line: node.end_position().row + 1,
                    start_col: node.start_position().column + 1,
                    signature: String::new(),
                });
            }
            _ => return None,
        },
        "python" => match kind {
            "function_definition" => (SymbolKind::Function, "name"),
            "class_definition" => (SymbolKind::Class, "name"),
            "import_statement" | "import_from_statement" => {
                return Some(Symbol {
                    name: node_text(node, source),
                    kind: SymbolKind::Import,
                    start_line: node.start_position().row + 1,
                    end_line: node.end_position().row + 1,
                    start_col: node.start_position().column + 1,
                    signature: String::new(),
                });
            }
            _ => return None,
        },
        "javascript" | "typescript" | "tsx" | "jsx" => match kind {
            "function_declaration" => (SymbolKind::Function, "name"),
            "class_declaration" => (SymbolKind::Class, "name"),
            "method_definition" => (SymbolKind::Method, "name"),
            "interface_declaration" => (SymbolKind::Interface, "name"),
            "type_alias_declaration" => (SymbolKind::Type, "name"),
            "enum_declaration" => (SymbolKind::Enum, "name"),
            "lexical_declaration" | "variable_declaration" => {
                if is_exported_const(node, source) {
                    return extract_variable_symbol(node, source);
                }
                return None;
            }
            "export_statement" => {
                let mut cursor = node.walk();
                for child in node.children(&mut cursor) {
                    if let Some(sym) = node_to_symbol(&child, source, language) {
                        return Some(sym);
                    }
                }
                return None;
            }
            "import_statement" => {
                return Some(Symbol {
                    name: node_text(node, source),
                    kind: SymbolKind::Import,
                    start_line: node.start_position().row + 1,
                    end_line: node.end_position().row + 1,
                    start_col: node.start_position().column + 1,
                    signature: String::new(),
                });
            }
            _ => return None,
        },
        "go" => match kind {
            "function_declaration" => (SymbolKind::Function, "name"),
            "method_declaration" => (SymbolKind::Method, "name"),
            "type_declaration" => {
                let spec = node.child_by_field_name("type_spec").or_else(|| {
                    let mut c = node.walk();
                    let children: Vec<_> = node.children(&mut c).collect();
                    children.into_iter().find(|n| n.kind() == "type_spec")
                });
                if let Some(spec_node) = spec {
                    if let Some(name_node) = spec_node.child_by_field_name("name") {
                        let name = node_text(&name_node, source);
                        let body = spec_node.child_by_field_name("type");
                        let kind = match body.map(|b| b.kind()) {
                            Some("struct_type") => SymbolKind::Struct,
                            Some("interface_type") => SymbolKind::Interface,
                            _ => SymbolKind::Type,
                        };
                        return Some(Symbol {
                            name,
                            kind,
                            start_line: node.start_position().row + 1,
                            end_line: node.end_position().row + 1,
                            start_col: node.start_position().column + 1,
                            signature: extract_signature(node, source),
                        });
                    }
                }
                return None;
            }
            _ => return None,
        },
        "java" | "kotlin" => match kind {
            "method_declaration" | "function_declaration" => (SymbolKind::Method, "name"),
            "class_declaration" => (SymbolKind::Class, "name"),
            "interface_declaration" => (SymbolKind::Interface, "name"),
            "enum_declaration" => (SymbolKind::Enum, "name"),
            _ => return None,
        },
        _ => match kind {
            k if k.contains("function") && k.contains("declaration") => {
                (SymbolKind::Function, "name")
            }
            k if k.contains("function") && k.contains("definition") => {
                (SymbolKind::Function, "name")
            }
            k if k.contains("class") && k.contains("declaration") => (SymbolKind::Class, "name"),
            k if k.contains("class") && k.contains("definition") => (SymbolKind::Class, "name"),
            k if k.contains("method") => (SymbolKind::Method, "name"),
            _ => return None,
        },
    };

    let name_node = node.child_by_field_name(name_field)?;
    let name = node_text(&name_node, source);
    if name.is_empty() {
        return None;
    }

    Some(Symbol {
        name,
        kind: symbol_kind,
        start_line: node.start_position().row + 1,
        end_line: node.end_position().row + 1,
        start_col: node.start_position().column + 1,
        signature: extract_signature(node, source),
    })
}

fn node_text(node: &Node, source: &str) -> String {
    source[node.byte_range()].to_string()
}

fn extract_signature(node: &Node, source: &str) -> String {
    let start = node.start_byte();
    let text = &source[start..];
    if let Some(brace) = text.find('{') {
        let sig = text[..brace].trim();
        if sig.len() <= 200 {
            return sig.to_string();
        }
        let end = sig.floor_char_boundary(200);
        return format!("{}...", &sig[..end]);
    }
    let first_line = text.lines().next().unwrap_or("");
    if first_line.len() <= 200 {
        first_line.to_string()
    } else {
        let end = first_line.floor_char_boundary(200);
        format!("{}...", &first_line[..end])
    }
}

fn is_exported_const(node: &Node, source: &str) -> bool {
    if let Some(parent) = node.parent() {
        if parent.kind() == "export_statement" {
            return true;
        }
    }
    let text = &source[node.byte_range()];
    text.starts_with("const ") || text.starts_with("export const ")
}

fn extract_variable_symbol(node: &Node, source: &str) -> Option<Symbol> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "variable_declarator" {
            if let Some(name_node) = child.child_by_field_name("name") {
                let name = node_text(&name_node, source);
                if !name.is_empty() {
                    return Some(Symbol {
                        name,
                        kind: SymbolKind::Constant,
                        start_line: node.start_position().row + 1,
                        end_line: node.end_position().row + 1,
                        start_col: node.start_position().column + 1,
                        signature: extract_signature(node, source),
                    });
                }
            }
        }
    }
    None
}

/// Extract deduplicated callee names (function/method calls and macro invocations)
/// from the entire AST. Useful for building call-relationship graphs.
pub fn extract_callees(tree: &Tree, source: &str, language: &str) -> Vec<String> {
    let mut names = Vec::new();
    collect_callees(&tree.root_node(), source, language, &mut names);
    names.sort_unstable();
    names.dedup();
    names
}

fn collect_callees(node: &Node, source: &str, language: &str, out: &mut Vec<String>) {
    let kind = node.kind();

    let callee_name: Option<String> = match language {
        "rust" => match kind {
            "call_expression" => node
                .child_by_field_name("function")
                .and_then(|n| callee_ident_text(&n, source)),
            "macro_invocation" => node
                .child_by_field_name("macro")
                .or_else(|| node.child(0))
                .map(|n| node_text(&n, source)),
            _ => None,
        },
        "python" => match kind {
            "call" => node
                .child_by_field_name("function")
                .and_then(|n| callee_ident_text(&n, source)),
            _ => None,
        },
        "javascript" | "typescript" | "tsx" | "jsx" => match kind {
            "call_expression" => node
                .child_by_field_name("function")
                .and_then(|n| callee_ident_text(&n, source)),
            _ => None,
        },
        "go" => match kind {
            "call_expression" => node
                .child_by_field_name("function")
                .and_then(|n| callee_ident_text(&n, source)),
            _ => None,
        },
        "java" | "kotlin" => match kind {
            "method_invocation" => node
                .child_by_field_name("name")
                .map(|n| node_text(&n, source)),
            _ => None,
        },
        _ => None,
    };

    if let Some(ref name) = callee_name {
        if !name.is_empty() && name.len() < 120 {
            out.push(name.clone());
        }
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_callees(&child, source, language, out);
    }
}

/// For call expressions, get the most meaningful identifier text.
/// Handles `foo`, `self.foo`, `module::bar`, `obj.method`.
fn callee_ident_text(node: &Node, source: &str) -> Option<String> {
    match node.kind() {
        "identifier" | "type_identifier" => Some(node_text(node, source)),
        "field_expression" | "member_expression" => {
            let field = node
                .child_by_field_name("field")
                .or_else(|| node.child_by_field_name("property"));
            field.map(|f| node_text(&f, source))
        }
        "scoped_identifier" | "qualified_name" => Some(node_text(node, source)),
        "attribute" => node
            .child_by_field_name("attribute")
            .map(|n| node_text(&n, source))
            .or_else(|| Some(node_text(node, source))),
        _ => {
            let text = node_text(node, source);
            if text.len() < 80 && !text.contains('\n') {
                Some(text)
            } else {
                None
            }
        }
    }
}

/// Extract (type_name, trait_name) pairs from `impl Trait for Type` blocks.
/// For non-Rust languages, extracts class inheritance / interface implementations.
pub fn extract_trait_impls(tree: &Tree, source: &str, language: &str) -> Vec<(String, String)> {
    let mut impls = Vec::new();
    collect_trait_impls(&tree.root_node(), source, language, &mut impls);
    impls
}

fn collect_trait_impls(node: &Node, source: &str, language: &str, out: &mut Vec<(String, String)>) {
    match language {
        "rust" => {
            if node.kind() == "impl_item" {
                if let Some(trait_node) = node.child_by_field_name("trait") {
                    if let Some(type_node) = node.child_by_field_name("type") {
                        let trait_name = node_text(&trait_node, source);
                        let type_name = node_text(&type_node, source);
                        if !trait_name.is_empty() && !type_name.is_empty() {
                            out.push((type_name, trait_name));
                        }
                    }
                }
            }
        }
        "python" => {
            if node.kind() == "class_definition" {
                if let Some(name_node) = node.child_by_field_name("name") {
                    let class_name = node_text(&name_node, source);
                    if let Some(bases) = node.child_by_field_name("superclasses") {
                        let mut cursor = bases.walk();
                        for base in bases.children(&mut cursor) {
                            if base.kind() == "identifier" || base.kind() == "attribute" {
                                out.push((class_name.clone(), node_text(&base, source)));
                            }
                        }
                    }
                }
            }
        }
        "java" | "kotlin" => {
            if node.kind() == "class_declaration" {
                if let Some(name_node) = node.child_by_field_name("name") {
                    let class_name = node_text(&name_node, source);
                    if let Some(interfaces) = node.child_by_field_name("interfaces") {
                        let mut cursor = interfaces.walk();
                        for iface in interfaces.children(&mut cursor) {
                            if iface.kind() == "type_identifier" || iface.kind() == "identifier" {
                                out.push((class_name.clone(), node_text(&iface, source)));
                            }
                        }
                    }
                    if let Some(superclass) = node.child_by_field_name("superclass") {
                        out.push((class_name, node_text(&superclass, source)));
                    }
                }
            }
        }
        "typescript" | "tsx" => {
            if node.kind() == "class_declaration" {
                if let Some(name_node) = node.child_by_field_name("name") {
                    let class_name = node_text(&name_node, source);
                    let mut cursor = node.walk();
                    for child in node.children(&mut cursor) {
                        if child.kind() == "class_heritage" {
                            let heritage_text = node_text(&child, source);
                            if let Some(rest) = heritage_text.strip_prefix("extends ") {
                                let base = rest.split_whitespace().next().unwrap_or("");
                                if !base.is_empty() {
                                    out.push((class_name.clone(), base.to_string()));
                                }
                            }
                            if let Some(rest) = heritage_text.strip_prefix("implements ") {
                                for iface in rest.split(',') {
                                    let name = iface.trim().split('<').next().unwrap_or("").trim();
                                    if !name.is_empty() {
                                        out.push((class_name.clone(), name.to_string()));
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        _ => {}
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_trait_impls(&child, source, language, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::CodeParser;

    #[test]
    fn extract_rust_symbols() {
        if !CodeParser::is_language_available("rust") {
            return;
        }
        let source = r#"
use std::collections::HashMap;

pub struct Config {
    pub name: String,
}

pub enum Status {
    Active,
    Inactive,
}

pub trait Handler {
    fn handle(&self);
}

pub fn process(input: &str) -> String {
    input.to_string()
}

impl Config {
    pub fn new() -> Self {
        Config { name: String::new() }
    }
}
"#;
        let parsed = CodeParser::parse(source, "rust").unwrap();
        let symbols = extract_symbols(&parsed.tree, source, "rust");
        let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"Config"), "missing Config, got: {names:?}");
        assert!(names.contains(&"Status"), "missing Status, got: {names:?}");
        assert!(
            names.contains(&"Handler"),
            "missing Handler, got: {names:?}"
        );
        assert!(
            names.contains(&"process"),
            "missing process, got: {names:?}"
        );
    }

    #[test]
    fn extract_python_symbols() {
        if !CodeParser::is_language_available("python") {
            return;
        }
        let source = r#"
import os
from pathlib import Path

class MyClass:
    def method(self):
        pass

def standalone_function(x, y):
    return x + y
"#;
        let parsed = CodeParser::parse(source, "python").unwrap();
        let symbols = extract_symbols(&parsed.tree, source, "python");
        let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();
        assert!(
            names.contains(&"MyClass"),
            "missing MyClass, got: {names:?}"
        );
        assert!(
            names.contains(&"standalone_function"),
            "missing standalone_function, got: {names:?}"
        );
        assert!(names.contains(&"method"), "missing method, got: {names:?}");
    }

    #[test]
    fn extract_rust_callees() {
        if !CodeParser::is_language_available("rust") {
            return;
        }
        let source = r#"
fn main() {
    let x = foo();
    bar::baz(x);
    println!("hello");
    obj.method_call();
}
"#;
        let parsed = CodeParser::parse(source, "rust").unwrap();
        let callees = extract_callees(&parsed.tree, source, "rust");
        assert!(
            callees.contains(&"foo".to_string()),
            "missing foo, got: {callees:?}"
        );
        assert!(
            callees.contains(&"bar::baz".to_string()),
            "missing bar::baz, got: {callees:?}"
        );
        assert!(
            callees.contains(&"println".to_string()),
            "missing println, got: {callees:?}"
        );
        assert!(
            callees.contains(&"method_call".to_string()),
            "missing method_call, got: {callees:?}"
        );
    }

    #[test]
    fn extract_rust_trait_impls() {
        if !CodeParser::is_language_available("rust") {
            return;
        }
        let source = r#"
struct MyType;

trait Foo {
    fn foo(&self);
}

impl Foo for MyType {
    fn foo(&self) {}
}

impl Default for MyType {
    fn default() -> Self { MyType }
}

impl MyType {
    fn inherent() {}
}
"#;
        let parsed = CodeParser::parse(source, "rust").unwrap();
        let impls = extract_trait_impls(&parsed.tree, source, "rust");
        assert!(
            impls.contains(&("MyType".to_string(), "Foo".to_string())),
            "missing (MyType, Foo), got: {impls:?}"
        );
        assert!(
            impls.contains(&("MyType".to_string(), "Default".to_string())),
            "missing (MyType, Default), got: {impls:?}"
        );
        assert_eq!(impls.len(), 2, "inherent impl should not appear: {impls:?}");
    }

    #[test]
    fn extract_python_callees() {
        if !CodeParser::is_language_available("python") {
            return;
        }
        let source = r#"
def main():
    x = foo()
    bar.baz(x)
    print("hello")
"#;
        let parsed = CodeParser::parse(source, "python").unwrap();
        let callees = extract_callees(&parsed.tree, source, "python");
        assert!(
            callees.contains(&"foo".to_string()),
            "missing foo, got: {callees:?}"
        );
        assert!(
            callees.contains(&"print".to_string()),
            "missing print, got: {callees:?}"
        );
    }

    #[test]
    fn extract_python_class_inheritance() {
        if !CodeParser::is_language_available("python") {
            return;
        }
        let source = r#"
class Animal:
    pass

class Dog(Animal):
    pass
"#;
        let parsed = CodeParser::parse(source, "python").unwrap();
        let impls = extract_trait_impls(&parsed.tree, source, "python");
        assert!(
            impls.contains(&("Dog".to_string(), "Animal".to_string())),
            "missing (Dog, Animal), got: {impls:?}"
        );
    }
}
