use std::collections::HashMap;
use std::path::Path;

use async_trait::async_trait;
use fastclaw_core::tool::{Tool, ToolKind, ToolParameterSchema, ToolResult};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Jupyter Notebook cell-level editor.
///
/// Supports three operations on `.ipynb` files:
/// - `replace`: overwrite a cell's source at a given index
/// - `insert`: insert a new cell at a given index
/// - `delete`: remove the cell at a given index
pub struct NotebookEditTool;

#[derive(Deserialize)]
struct NotebookArgs {
    path: String,
    operation: String,
    cell_index: usize,
    #[serde(default)]
    cell_type: Option<String>,
    #[serde(default)]
    source: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct Notebook {
    cells: Vec<NotebookCell>,
    metadata: Value,
    nbformat: u32,
    nbformat_minor: u32,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct NotebookCell {
    cell_type: String,
    source: Vec<String>,
    metadata: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    outputs: Option<Vec<Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    execution_count: Option<Value>,
}

impl NotebookCell {
    fn new_code(source_lines: Vec<String>) -> Self {
        Self {
            cell_type: "code".to_string(),
            source: source_lines,
            metadata: serde_json::json!({}),
            outputs: Some(Vec::new()),
            execution_count: Some(Value::Null),
        }
    }

    fn new_markdown(source_lines: Vec<String>) -> Self {
        Self {
            cell_type: "markdown".to_string(),
            source: source_lines,
            metadata: serde_json::json!({}),
            outputs: None,
            execution_count: None,
        }
    }
}

fn source_to_lines(source: &str) -> Vec<String> {
    if source.is_empty() {
        return vec![];
    }
    let parts: Vec<&str> = source.split('\n').collect();
    let mut lines = Vec::with_capacity(parts.len());
    for (i, part) in parts.iter().enumerate() {
        if i < parts.len() - 1 {
            lines.push(format!("{part}\n"));
        } else if !part.is_empty() {
            // Last segment without trailing newline.
            lines.push(part.to_string());
        }
        // If the last part is empty (source ended with \n), skip it —
        // the preceding line already has its \n.
    }
    lines
}

fn read_notebook(path: &Path) -> Result<Notebook, String> {
    if path.extension().and_then(|e| e.to_str()) != Some("ipynb") {
        return Err(format!(
            "Not a Jupyter notebook: {}. Only .ipynb files are supported.",
            path.display()
        ));
    }
    let content = std::fs::read_to_string(path)
        .map_err(|e| format!("Failed to read {}: {e}", path.display()))?;
    serde_json::from_str::<Notebook>(&content)
        .map_err(|e| format!("Failed to parse notebook {}: {e}", path.display()))
}

fn write_notebook(path: &Path, nb: &Notebook) -> Result<(), String> {
    let json = serde_json::to_string_pretty(nb)
        .map_err(|e| format!("Failed to serialize notebook: {e}"))?;
    std::fs::write(path, json).map_err(|e| format!("Failed to write {}: {e}", path.display()))
}

#[async_trait]
impl Tool for NotebookEditTool {
    fn kind(&self) -> ToolKind {
        ToolKind::Edit
    }

    fn name(&self) -> &str {
        "notebook_edit"
    }

    fn description(&self) -> &str {
        "Edit a Jupyter Notebook (.ipynb) at the cell level. \
         Operations: 'replace' (overwrite cell source), 'insert' (add new cell), \
         'delete' (remove cell). Provide path, operation, cell_index, and optionally \
         cell_type ('code'|'markdown') and source."
    }

    fn search_hint(&self) -> &str {
        "jupyter notebook ipynb cell edit python data science"
    }

    fn is_deferred(&self) -> bool {
        true
    }

    fn parameters_schema(&self) -> ToolParameterSchema {
        let mut props = HashMap::new();
        props.insert(
            "path".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "Absolute or relative path to the .ipynb file."
            }),
        );
        props.insert(
            "operation".to_string(),
            serde_json::json!({
                "type": "string",
                "enum": ["replace", "insert", "delete"],
                "description": "Cell operation: replace, insert, or delete."
            }),
        );
        props.insert(
            "cell_index".to_string(),
            serde_json::json!({
                "type": "integer",
                "description": "0-based index of the target cell."
            }),
        );
        props.insert(
            "cell_type".to_string(),
            serde_json::json!({
                "type": "string",
                "enum": ["code", "markdown"],
                "description": "Cell type for insert/replace. Defaults to 'code'."
            }),
        );
        props.insert(
            "source".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "New cell source content (required for replace and insert)."
            }),
        );
        ToolParameterSchema {
            schema_type: "object".to_string(),
            properties: props,
            required: vec![
                "path".to_string(),
                "operation".to_string(),
                "cell_index".to_string(),
            ],
        }
    }

    async fn execute(&self, arguments: &str) -> ToolResult {
        let args: NotebookArgs = match serde_json::from_str(arguments) {
            Ok(v) => v,
            Err(e) => {
                return ToolResult::err(format!(
                    "Invalid arguments: {e}. Expected {{\"path\": \"...\", \"operation\": \"replace|insert|delete\", \"cell_index\": N, ...}}"
                ))
            }
        };

        let path = Path::new(&args.path);

        match args.operation.as_str() {
            "replace" => execute_replace(path, &args),
            "insert" => execute_insert(path, &args),
            "delete" => execute_delete(path, &args),
            other => ToolResult::err(format!(
                "Unknown operation '{other}'. Must be 'replace', 'insert', or 'delete'."
            )),
        }
    }
}

fn execute_replace(path: &Path, args: &NotebookArgs) -> ToolResult {
    let source = match &args.source {
        Some(s) => s,
        None => return ToolResult::err("'source' is required for replace operation.".to_string()),
    };

    let mut nb = match read_notebook(path) {
        Ok(nb) => nb,
        Err(e) => return ToolResult::err(e),
    };

    if args.cell_index >= nb.cells.len() {
        return ToolResult::err(format!(
            "cell_index {} out of range (notebook has {} cells).",
            args.cell_index,
            nb.cells.len()
        ));
    }

    let cell = &mut nb.cells[args.cell_index];
    cell.source = source_to_lines(source);

    if let Some(ct) = &args.cell_type {
        cell.cell_type = ct.clone();
        if ct == "markdown" {
            cell.outputs = None;
            cell.execution_count = None;
        } else if ct == "code" && cell.outputs.is_none() {
            cell.outputs = Some(Vec::new());
            cell.execution_count = Some(Value::Null);
        }
    }

    if let Err(e) = write_notebook(path, &nb) {
        return ToolResult::err(e);
    }

    ToolResult::ok(format!(
        "Replaced cell {} in {}.",
        args.cell_index,
        path.display()
    ))
}

fn execute_insert(path: &Path, args: &NotebookArgs) -> ToolResult {
    let source = match &args.source {
        Some(s) => s,
        None => return ToolResult::err("'source' is required for insert operation.".to_string()),
    };

    let mut nb = match read_notebook(path) {
        Ok(nb) => nb,
        Err(e) => return ToolResult::err(e),
    };

    if args.cell_index > nb.cells.len() {
        return ToolResult::err(format!(
            "cell_index {} out of range for insert (notebook has {} cells, max insert index is {}).",
            args.cell_index,
            nb.cells.len(),
            nb.cells.len()
        ));
    }

    let cell_type = args.cell_type.as_deref().unwrap_or("code");
    let lines = source_to_lines(source);
    let cell = match cell_type {
        "markdown" => NotebookCell::new_markdown(lines),
        _ => NotebookCell::new_code(lines),
    };

    nb.cells.insert(args.cell_index, cell);

    if let Err(e) = write_notebook(path, &nb) {
        return ToolResult::err(e);
    }

    ToolResult::ok(format!(
        "Inserted {} cell at index {} in {} (now {} cells).",
        cell_type,
        args.cell_index,
        path.display(),
        nb.cells.len()
    ))
}

fn execute_delete(path: &Path, args: &NotebookArgs) -> ToolResult {
    let mut nb = match read_notebook(path) {
        Ok(nb) => nb,
        Err(e) => return ToolResult::err(e),
    };

    if args.cell_index >= nb.cells.len() {
        return ToolResult::err(format!(
            "cell_index {} out of range (notebook has {} cells).",
            args.cell_index,
            nb.cells.len()
        ));
    }

    nb.cells.remove(args.cell_index);

    if let Err(e) = write_notebook(path, &nb) {
        return ToolResult::err(e);
    }

    ToolResult::ok(format!(
        "Deleted cell {} from {} (now {} cells).",
        args.cell_index,
        path.display(),
        nb.cells.len()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn sample_notebook() -> String {
        serde_json::json!({
            "cells": [
                {
                    "cell_type": "markdown",
                    "source": ["# Hello\n", "This is a test."],
                    "metadata": {}
                },
                {
                    "cell_type": "code",
                    "source": ["print('hello')"],
                    "metadata": {},
                    "outputs": [],
                    "execution_count": null
                },
                {
                    "cell_type": "code",
                    "source": ["x = 42\n", "print(x)"],
                    "metadata": {},
                    "outputs": [],
                    "execution_count": 1
                }
            ],
            "metadata": {
                "kernelspec": {
                    "display_name": "Python 3",
                    "language": "python",
                    "name": "python3"
                }
            },
            "nbformat": 4,
            "nbformat_minor": 5
        })
        .to_string()
    }

    fn write_temp_notebook(content: &str) -> NamedTempFile {
        let mut f = tempfile::Builder::new()
            .suffix(".ipynb")
            .tempfile()
            .unwrap();
        f.write_all(content.as_bytes()).unwrap();
        f.flush().unwrap();
        f
    }

    fn read_back(path: &Path) -> Notebook {
        let content = std::fs::read_to_string(path).unwrap();
        serde_json::from_str(&content).unwrap()
    }

    #[test]
    fn parse_standard_ipynb() {
        let tmp = write_temp_notebook(&sample_notebook());
        let nb = read_notebook(tmp.path()).unwrap();
        assert_eq!(nb.cells.len(), 3);
        assert_eq!(nb.cells[0].cell_type, "markdown");
        assert_eq!(nb.cells[1].cell_type, "code");
        assert_eq!(nb.nbformat, 4);
    }

    #[test]
    fn reject_non_ipynb() {
        let mut f = tempfile::Builder::new().suffix(".py").tempfile().unwrap();
        f.write_all(b"print('hello')").unwrap();
        let result = read_notebook(f.path());
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Not a Jupyter notebook"));
    }

    #[test]
    fn replace_cell_source() {
        let tmp = write_temp_notebook(&sample_notebook());
        let args = NotebookArgs {
            path: tmp.path().to_string_lossy().to_string(),
            operation: "replace".to_string(),
            cell_index: 1,
            cell_type: None,
            source: Some("print('replaced')".to_string()),
        };
        let result = execute_replace(tmp.path(), &args);
        assert!(result.success);

        let nb = read_back(tmp.path());
        assert_eq!(nb.cells[1].source, vec!["print('replaced')"]);
    }

    #[test]
    fn replace_cell_with_type_change() {
        let tmp = write_temp_notebook(&sample_notebook());
        let args = NotebookArgs {
            path: tmp.path().to_string_lossy().to_string(),
            operation: "replace".to_string(),
            cell_index: 1,
            cell_type: Some("markdown".to_string()),
            source: Some("# Now markdown".to_string()),
        };
        let result = execute_replace(tmp.path(), &args);
        assert!(result.success);

        let nb = read_back(tmp.path());
        assert_eq!(nb.cells[1].cell_type, "markdown");
        assert!(nb.cells[1].outputs.is_none());
    }

    #[test]
    fn replace_out_of_range() {
        let tmp = write_temp_notebook(&sample_notebook());
        let args = NotebookArgs {
            path: tmp.path().to_string_lossy().to_string(),
            operation: "replace".to_string(),
            cell_index: 99,
            cell_type: None,
            source: Some("x".to_string()),
        };
        let result = execute_replace(tmp.path(), &args);
        assert!(!result.success);
        assert!(result.output.contains("out of range"));
    }

    #[test]
    fn replace_missing_source() {
        let tmp = write_temp_notebook(&sample_notebook());
        let args = NotebookArgs {
            path: tmp.path().to_string_lossy().to_string(),
            operation: "replace".to_string(),
            cell_index: 0,
            cell_type: None,
            source: None,
        };
        let result = execute_replace(tmp.path(), &args);
        assert!(!result.success);
        assert!(result.output.contains("source"));
    }

    #[test]
    fn insert_code_cell() {
        let tmp = write_temp_notebook(&sample_notebook());
        let args = NotebookArgs {
            path: tmp.path().to_string_lossy().to_string(),
            operation: "insert".to_string(),
            cell_index: 1,
            cell_type: Some("code".to_string()),
            source: Some("y = 100".to_string()),
        };
        let result = execute_insert(tmp.path(), &args);
        assert!(result.success);

        let nb = read_back(tmp.path());
        assert_eq!(nb.cells.len(), 4);
        assert_eq!(nb.cells[1].cell_type, "code");
        assert_eq!(nb.cells[1].source, vec!["y = 100"]);
        assert!(nb.cells[1].outputs.is_some());
    }

    #[test]
    fn insert_markdown_cell() {
        let tmp = write_temp_notebook(&sample_notebook());
        let args = NotebookArgs {
            path: tmp.path().to_string_lossy().to_string(),
            operation: "insert".to_string(),
            cell_index: 0,
            cell_type: Some("markdown".to_string()),
            source: Some("# Title".to_string()),
        };
        let result = execute_insert(tmp.path(), &args);
        assert!(result.success);

        let nb = read_back(tmp.path());
        assert_eq!(nb.cells.len(), 4);
        assert_eq!(nb.cells[0].cell_type, "markdown");
        assert!(nb.cells[0].outputs.is_none());
    }

    #[test]
    fn insert_at_end() {
        let tmp = write_temp_notebook(&sample_notebook());
        let args = NotebookArgs {
            path: tmp.path().to_string_lossy().to_string(),
            operation: "insert".to_string(),
            cell_index: 3, // After last cell.
            cell_type: None,
            source: Some("last_cell()".to_string()),
        };
        let result = execute_insert(tmp.path(), &args);
        assert!(result.success);

        let nb = read_back(tmp.path());
        assert_eq!(nb.cells.len(), 4);
        assert_eq!(nb.cells[3].source, vec!["last_cell()"]);
    }

    #[test]
    fn insert_out_of_range() {
        let tmp = write_temp_notebook(&sample_notebook());
        let args = NotebookArgs {
            path: tmp.path().to_string_lossy().to_string(),
            operation: "insert".to_string(),
            cell_index: 99,
            cell_type: None,
            source: Some("x".to_string()),
        };
        let result = execute_insert(tmp.path(), &args);
        assert!(!result.success);
        assert!(result.output.contains("out of range"));
    }

    #[test]
    fn delete_cell() {
        let tmp = write_temp_notebook(&sample_notebook());
        let args = NotebookArgs {
            path: tmp.path().to_string_lossy().to_string(),
            operation: "delete".to_string(),
            cell_index: 1,
            cell_type: None,
            source: None,
        };
        let result = execute_delete(tmp.path(), &args);
        assert!(result.success);

        let nb = read_back(tmp.path());
        assert_eq!(nb.cells.len(), 2);
        assert_eq!(nb.cells[0].cell_type, "markdown");
        assert_eq!(nb.cells[1].cell_type, "code");
        assert_eq!(nb.cells[1].source, vec!["x = 42\n", "print(x)"]);
    }

    #[test]
    fn delete_out_of_range() {
        let tmp = write_temp_notebook(&sample_notebook());
        let args = NotebookArgs {
            path: tmp.path().to_string_lossy().to_string(),
            operation: "delete".to_string(),
            cell_index: 99,
            cell_type: None,
            source: None,
        };
        let result = execute_delete(tmp.path(), &args);
        assert!(!result.success);
        assert!(result.output.contains("out of range"));
    }

    #[test]
    fn roundtrip_preserves_structure() {
        let tmp = write_temp_notebook(&sample_notebook());
        let nb_before = read_notebook(tmp.path()).unwrap();

        // Replace cell 0, then read back.
        let args = NotebookArgs {
            path: tmp.path().to_string_lossy().to_string(),
            operation: "replace".to_string(),
            cell_index: 0,
            cell_type: None,
            source: Some("# Modified".to_string()),
        };
        execute_replace(tmp.path(), &args);

        let nb_after = read_back(tmp.path());
        assert_eq!(nb_after.nbformat, nb_before.nbformat);
        assert_eq!(nb_after.nbformat_minor, nb_before.nbformat_minor);
        assert_eq!(nb_after.cells.len(), nb_before.cells.len());
        // Metadata preserved.
        assert_eq!(nb_after.metadata, nb_before.metadata);
    }

    #[test]
    fn source_to_lines_multiline() {
        let lines = source_to_lines("line1\nline2\nline3");
        assert_eq!(lines, vec!["line1\n", "line2\n", "line3"]);
    }

    #[test]
    fn source_to_lines_trailing_newline() {
        let lines = source_to_lines("line1\nline2\n");
        assert_eq!(lines, vec!["line1\n", "line2\n"]);
    }

    #[test]
    fn source_to_lines_single_line() {
        let lines = source_to_lines("hello");
        assert_eq!(lines, vec!["hello"]);
    }
}
