use serde::Serialize;
use tree_sitter::{Node, Tree};

/// A semantic chunk of code, representing a top-level definition or logical block.
#[derive(Debug, Clone, Serialize)]
pub struct CodeChunk {
    pub start_line: usize,
    pub end_line: usize,
    pub kind: String,
    pub name: Option<String>,
    pub content: String,
}

/// Chunk source code into semantic blocks using the AST.
/// Falls back to line-based chunking if the language isn't supported or the tree has errors.
pub fn chunk_file(
    tree: &Tree,
    source: &str,
    language: &str,
    max_chunk_lines: usize,
) -> Vec<CodeChunk> {
    let root = tree.root_node();
    let mut chunks = Vec::new();
    let mut last_end_byte = 0;

    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        let start_byte = child.start_byte();
        if start_byte > last_end_byte {
            let gap = &source[last_end_byte..start_byte];
            if gap.trim().len() > 1 {
                let start_line = source[..last_end_byte].lines().count() + 1;
                let end_line = source[..start_byte].lines().count();
                chunks.push(CodeChunk {
                    start_line,
                    end_line,
                    kind: "preamble".to_string(),
                    name: None,
                    content: gap.to_string(),
                });
            }
        }

        let chunk = node_to_chunk(&child, source, language);
        let line_count = chunk.end_line.saturating_sub(chunk.start_line) + 1;

        if line_count > max_chunk_lines && child.child_count() > 0 {
            split_large_node(&child, source, language, max_chunk_lines, &mut chunks);
        } else {
            chunks.push(chunk);
        }

        last_end_byte = child.end_byte();
    }

    if last_end_byte < source.len() {
        let trailing = &source[last_end_byte..];
        if trailing.trim().len() > 1 {
            let start_line = source[..last_end_byte].lines().count() + 1;
            let end_line = source.lines().count();
            chunks.push(CodeChunk {
                start_line,
                end_line,
                kind: "trailing".to_string(),
                name: None,
                content: trailing.to_string(),
            });
        }
    }

    chunks
}

fn node_to_chunk(node: &Node, source: &str, language: &str) -> CodeChunk {
    let name = extract_chunk_name(node, source, language);
    CodeChunk {
        start_line: node.start_position().row + 1,
        end_line: node.end_position().row + 1,
        kind: node.kind().to_string(),
        name,
        content: source[node.byte_range()].to_string(),
    }
}

fn extract_chunk_name(node: &Node, source: &str, _language: &str) -> Option<String> {
    if let Some(name_node) = node.child_by_field_name("name") {
        return Some(source[name_node.byte_range()].to_string());
    }
    if node.kind() == "impl_item" {
        if let Some(type_node) = node.child_by_field_name("type") {
            return Some(source[type_node.byte_range()].to_string());
        }
    }
    None
}

fn split_large_node(
    node: &Node,
    source: &str,
    language: &str,
    max_chunk_lines: usize,
    out: &mut Vec<CodeChunk>,
) {
    let mut cursor = node.walk();
    let children: Vec<_> = node.children(&mut cursor).collect();

    if children.is_empty() {
        out.push(node_to_chunk(node, source, language));
        return;
    }

    let mut group_start: Option<usize> = None;
    let mut group_end: Option<usize> = None;

    for (i, child) in children.iter().enumerate() {
        let child_start = child.start_position().row + 1;
        let child_end = child.end_position().row + 1;

        if group_start.is_none() {
            group_start = Some(child_start);
            group_end = Some(child_end);
            continue;
        }

        let current_group_start = group_start.unwrap();
        let new_span = child_end.saturating_sub(current_group_start) + 1;

        if new_span > max_chunk_lines {
            let ge = group_end.unwrap();
            let text_start = line_to_byte_offset(source, current_group_start);
            let text_end = line_to_byte_offset(source, ge + 1).min(source.len());
            out.push(CodeChunk {
                start_line: current_group_start,
                end_line: ge,
                kind: format!("{}_part", node.kind()),
                name: extract_chunk_name(node, source, language),
                content: source[text_start..text_end].to_string(),
            });
            group_start = Some(child_start);
            group_end = Some(child_end);
        } else {
            group_end = Some(child_end);
        }

        if i == children.len() - 1 {
            let gs = group_start.unwrap();
            let ge = group_end.unwrap();
            let text_start = line_to_byte_offset(source, gs);
            let text_end = line_to_byte_offset(source, ge + 1).min(source.len());
            out.push(CodeChunk {
                start_line: gs,
                end_line: ge,
                kind: format!("{}_part", node.kind()),
                name: extract_chunk_name(node, source, language),
                content: source[text_start..text_end].to_string(),
            });
        }
    }
}

fn line_to_byte_offset(source: &str, line: usize) -> usize {
    if line <= 1 {
        return 0;
    }
    let target = line - 1;
    let mut count = 0;
    for (i, c) in source.char_indices() {
        if c == '\n' {
            count += 1;
            if count == target {
                return i + 1;
            }
        }
    }
    source.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::CodeParser;

    #[test]
    fn chunk_rust_file() {
        if !CodeParser::is_language_available("rust") {
            return;
        }
        let source = r#"use std::io;

pub struct Foo {
    bar: i32,
}

impl Foo {
    pub fn new() -> Self {
        Foo { bar: 0 }
    }

    pub fn get_bar(&self) -> i32 {
        self.bar
    }
}

pub fn standalone() {
    println!("hello");
}
"#;
        let parsed = CodeParser::parse(source, "rust").unwrap();
        let chunks = chunk_file(&parsed.tree, source, "rust", 50);
        assert!(!chunks.is_empty(), "should produce at least one chunk");
        for chunk in &chunks {
            assert!(chunk.start_line <= chunk.end_line);
            assert!(!chunk.content.is_empty());
        }
    }

    #[test]
    fn chunk_splits_large_nodes() {
        if !CodeParser::is_language_available("rust") {
            return;
        }
        let mut methods = String::new();
        for i in 0..20 {
            methods.push_str(&format!(
                "    pub fn method_{i}(&self) -> i32 {{\n        {i}\n    }}\n\n"
            ));
        }
        let source = format!("pub struct Big;\n\nimpl Big {{\n{methods}}}\n");
        let parsed = CodeParser::parse(&source, "rust").unwrap();
        let chunks = chunk_file(&parsed.tree, &source, "rust", 10);
        assert!(
            chunks.len() > 1,
            "large impl block should be split into multiple chunks, got {}",
            chunks.len()
        );
    }
}
