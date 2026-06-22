use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use rusqlite::{params, Connection};
use serde::Serialize;

static GLOBAL_INDEX: OnceLock<Arc<SymbolIndex>> = OnceLock::new();

#[derive(Debug, Clone, Serialize)]
pub struct IndexedSymbol {
    pub name: String,
    pub kind: String,
    pub file_path: String,
    pub start_line: usize,
    pub end_line: usize,
    pub signature: String,
}

pub struct SymbolIndex {
    conn: Mutex<Connection>,
}

impl SymbolIndex {
    pub fn open(db_path: &Path) -> anyhow::Result<Self> {
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(db_path)?;
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             PRAGMA busy_timeout = 5000;",
        )?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS symbols (
                id          INTEGER PRIMARY KEY,
                name        TEXT NOT NULL,
                kind        TEXT NOT NULL,
                file_path   TEXT NOT NULL,
                start_line  INTEGER NOT NULL,
                end_line    INTEGER NOT NULL,
                signature   TEXT NOT NULL DEFAULT '',
                file_hash   TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_symbols_name ON symbols(name);
            CREATE INDEX IF NOT EXISTS idx_symbols_file ON symbols(file_path);
            CREATE INDEX IF NOT EXISTS idx_symbols_kind ON symbols(kind);",
        )?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    pub fn global() -> &'static Arc<SymbolIndex> {
        GLOBAL_INDEX.get_or_init(|| {
            let state_dir = dirs::data_local_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join("xiaolin");
            let db_path = state_dir.join("symbol_index.db");
            match SymbolIndex::open(&db_path) {
                Ok(idx) => Arc::new(idx),
                Err(e) => {
                    tracing::warn!(error = %e, "Failed to open symbol index DB, using in-memory");
                    Arc::new(SymbolIndex::open(Path::new(":memory:")).expect("in-memory DB"))
                }
            }
        })
    }

    pub fn index_file(
        &self,
        file_path: &str,
        symbols: &[xiaolin_treesitter::Symbol],
        file_hash: &str,
    ) -> anyhow::Result<()> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("{e}"))?;
        let tx = conn.unchecked_transaction()?;

        tx.execute(
            "DELETE FROM symbols WHERE file_path = ?1",
            params![file_path],
        )?;

        {
            let mut stmt = tx.prepare_cached(
                "INSERT INTO symbols (name, kind, file_path, start_line, end_line, signature, file_hash)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            )?;

            for sym in symbols {
                stmt.execute(params![
                    sym.name,
                    sym.kind.to_string(),
                    file_path,
                    sym.start_line,
                    sym.end_line,
                    sym.signature,
                    file_hash,
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub fn file_hash(&self, file_path: &str) -> anyhow::Result<Option<String>> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("{e}"))?;
        let mut stmt =
            conn.prepare_cached("SELECT file_hash FROM symbols WHERE file_path = ?1 LIMIT 1")?;
        let hash = stmt
            .query_row(params![file_path], |row| row.get::<_, String>(0))
            .ok();
        Ok(hash)
    }

    pub fn lookup(&self, name: &str) -> Vec<IndexedSymbol> {
        let conn = match self.conn.lock() {
            Ok(c) => c,
            Err(_) => return Vec::new(),
        };
        let mut stmt = match conn.prepare(
            "SELECT name, kind, file_path, start_line, end_line, signature
             FROM symbols WHERE name = ?1 OR name LIKE ?2 LIMIT 100",
        ) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };

        let prefix = format!("{name}%");
        stmt.query_map(params![name, prefix], |row| {
            Ok(IndexedSymbol {
                name: row.get(0)?,
                kind: row.get(1)?,
                file_path: row.get(2)?,
                start_line: row.get(3)?,
                end_line: row.get(4)?,
                signature: row.get(5)?,
            })
        })
        .ok()
        .into_iter()
        .flatten()
        .filter_map(|r| r.ok())
        .collect()
    }

    pub fn find_references(&self, name: &str) -> Vec<IndexedSymbol> {
        let conn = match self.conn.lock() {
            Ok(c) => c,
            Err(_) => return Vec::new(),
        };
        let pattern = format!("%{name}%");
        let mut stmt = match conn.prepare(
            "SELECT name, kind, file_path, start_line, end_line, signature
             FROM symbols WHERE signature LIKE ?1 OR name = ?2 LIMIT 200",
        ) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };

        stmt.query_map(params![pattern, name], |row| {
            Ok(IndexedSymbol {
                name: row.get(0)?,
                kind: row.get(1)?,
                file_path: row.get(2)?,
                start_line: row.get(3)?,
                end_line: row.get(4)?,
                signature: row.get(5)?,
            })
        })
        .ok()
        .into_iter()
        .flatten()
        .filter_map(|r| r.ok())
        .collect()
    }

    pub fn symbols_in_file(&self, file_path: &str) -> Vec<IndexedSymbol> {
        let conn = match self.conn.lock() {
            Ok(c) => c,
            Err(_) => return Vec::new(),
        };
        let mut stmt = match conn.prepare(
            "SELECT name, kind, file_path, start_line, end_line, signature
             FROM symbols WHERE file_path = ?1 ORDER BY start_line",
        ) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };

        stmt.query_map(params![file_path], |row| {
            Ok(IndexedSymbol {
                name: row.get(0)?,
                kind: row.get(1)?,
                file_path: row.get(2)?,
                start_line: row.get(3)?,
                end_line: row.get(4)?,
                signature: row.get(5)?,
            })
        })
        .ok()
        .into_iter()
        .flatten()
        .filter_map(|r| r.ok())
        .collect()
    }

    pub fn symbol_count(&self) -> usize {
        let conn = match self.conn.lock() {
            Ok(c) => c,
            Err(_) => return 0,
        };
        conn.query_row("SELECT COUNT(*) FROM symbols", [], |row| {
            row.get::<_, usize>(0)
        })
        .unwrap_or(0)
    }
}

const SKIP_DIRS: &[&str] = &[
    ".git",
    "node_modules",
    "target",
    "__pycache__",
    ".mypy_cache",
    "dist",
    "build",
    ".next",
    "vendor",
    ".cargo",
];

const SOURCE_EXTENSIONS: &[&str] = &[
    "rs", "py", "js", "ts", "tsx", "jsx", "go", "java", "kt", "c", "cpp", "h", "hpp", "cs", "rb",
    "swift", "scala",
];

fn is_source_file(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| SOURCE_EXTENSIONS.contains(&e))
        .unwrap_or(false)
}

fn should_skip_dir(name: &str) -> bool {
    SKIP_DIRS.contains(&name) || name.starts_with('.')
}

fn compute_file_hash(content: &str) -> String {
    blake3::hash(content.as_bytes()).to_hex().to_string()
}

pub fn start_background_scan(root: PathBuf, index: Arc<SymbolIndex>) {
    tokio::spawn(async move {
        tracing::info!(root = %root.display(), "Starting background symbol scan");
        let mut indexed = 0usize;
        let mut skipped = 0usize;

        let walker = build_walker(&root);
        for path in &walker {
            let path = path.as_path();
            if !is_source_file(path) {
                continue;
            }

            let lang = match xiaolin_treesitter::CodeParser::detect_language(path) {
                Some(l) if xiaolin_treesitter::CodeParser::is_language_available(&l) => l,
                _ => continue,
            };

            let source = match std::fs::read_to_string(path) {
                Ok(s) => s,
                Err(_) => continue,
            };

            let hash = compute_file_hash(&source);
            let rel = path
                .strip_prefix(&root)
                .unwrap_or(path)
                .to_string_lossy()
                .to_string();

            if let Ok(Some(existing)) = index.file_hash(&rel) {
                if existing == hash {
                    skipped += 1;
                    continue;
                }
            }

            let parsed = match xiaolin_treesitter::CodeParser::parse(&source, &lang) {
                Ok(p) => p,
                Err(_) => continue,
            };

            let symbols = xiaolin_treesitter::extract_symbols(&parsed.tree, &source, &lang);
            if let Err(e) = index.index_file(&rel, &symbols, &hash) {
                tracing::debug!(error = %e, file = %rel, "Failed to index file");
            } else {
                indexed += 1;
            }

            if indexed > 0 && indexed.is_multiple_of(100) {
                tokio::task::yield_now().await;
            }
        }

        tracing::info!(indexed, skipped, "Background symbol scan complete");
    });
}

pub fn start_watcher(root: PathBuf, index: Arc<SymbolIndex>) {
    use notify::{Event, EventKind, RecursiveMode, Watcher};

    let root_clone = root.clone();
    let (tx, rx) = std::sync::mpsc::channel::<Event>();

    std::thread::spawn(move || {
        let mut watcher = match notify::recommended_watcher(move |res: Result<Event, _>| {
            if let Ok(event) = res {
                let _ = tx.send(event);
            }
        }) {
            Ok(w) => w,
            Err(e) => {
                tracing::warn!(error = %e, "Failed to create file watcher for symbol index");
                return;
            }
        };

        if let Err(e) = watcher.watch(&root_clone, RecursiveMode::Recursive) {
            tracing::warn!(error = %e, "Failed to start watching for symbol index");
            return;
        }

        tracing::debug!(root = %root_clone.display(), "Symbol index file watcher started");

        while let Ok(event) = rx.recv() {
            match event.kind {
                EventKind::Create(_) | EventKind::Modify(_) => {
                    for path in &event.paths {
                        if !is_source_file(path) {
                            continue;
                        }
                        if path
                            .components()
                            .any(|c| should_skip_dir(&c.as_os_str().to_string_lossy()))
                        {
                            continue;
                        }
                        reindex_single_file(path, &root_clone, &index);
                    }
                }
                EventKind::Remove(_) => {
                    for path in &event.paths {
                        let rel = path
                            .strip_prefix(&root_clone)
                            .unwrap_or(path)
                            .to_string_lossy()
                            .to_string();
                        if let Ok(conn) = index.conn.lock() {
                            let _ = conn
                                .execute("DELETE FROM symbols WHERE file_path = ?1", params![rel]);
                        }
                    }
                }
                _ => {}
            }
        }
    });
}

fn reindex_single_file(path: &Path, root: &Path, index: &SymbolIndex) {
    let lang = match xiaolin_treesitter::CodeParser::detect_language(path) {
        Some(l) if xiaolin_treesitter::CodeParser::is_language_available(&l) => l,
        _ => return,
    };

    let source = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(_) => return,
    };

    let hash = compute_file_hash(&source);
    let rel = path
        .strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .to_string();

    if let Ok(Some(existing)) = index.file_hash(&rel) {
        if existing == hash {
            return;
        }
    }

    let parsed = match xiaolin_treesitter::CodeParser::parse(&source, &lang) {
        Ok(p) => p,
        Err(_) => return,
    };

    let symbols = xiaolin_treesitter::extract_symbols(&parsed.tree, &source, &lang);
    if let Err(e) = index.index_file(&rel, &symbols, &hash) {
        tracing::debug!(error = %e, file = %rel, "Failed to re-index file on change");
    }
}

fn build_walker(root: &Path) -> Vec<std::path::PathBuf> {
    let mut files = Vec::new();
    walk_dir(root, &mut files);
    files
}

fn walk_dir(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if !should_skip_dir(&name_str) {
                walk_dir(&path, out);
            }
        } else {
            out.push(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn make_test_index() -> (SymbolIndex, TempDir) {
        let tmp = TempDir::new().unwrap();
        let db_path = tmp.path().join("test_symbols.db");
        let index = SymbolIndex::open(&db_path).unwrap();
        (index, tmp)
    }

    #[test]
    fn create_and_lookup() {
        let (index, _tmp) = make_test_index();
        let symbols = vec![xiaolin_treesitter::Symbol {
            name: "process_data".into(),
            kind: xiaolin_treesitter::SymbolKind::Function,
            start_line: 10,
            end_line: 25,
            start_col: 1,
            signature: "pub fn process_data(input: &str) -> Result<()>".into(),
        }];
        index.index_file("src/main.rs", &symbols, "abc123").unwrap();

        let results = index.lookup("process_data");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "process_data");
        assert_eq!(results[0].kind, "function");
        assert_eq!(results[0].start_line, 10);
    }

    #[test]
    fn incremental_reindex_replaces_old_symbols() {
        let (index, _tmp) = make_test_index();
        let v1 = vec![xiaolin_treesitter::Symbol {
            name: "old_fn".into(),
            kind: xiaolin_treesitter::SymbolKind::Function,
            start_line: 1,
            end_line: 5,
            start_col: 1,
            signature: "fn old_fn()".into(),
        }];
        index.index_file("lib.rs", &v1, "v1").unwrap();
        assert_eq!(index.lookup("old_fn").len(), 1);

        let v2 = vec![xiaolin_treesitter::Symbol {
            name: "new_fn".into(),
            kind: xiaolin_treesitter::SymbolKind::Function,
            start_line: 1,
            end_line: 5,
            start_col: 1,
            signature: "fn new_fn()".into(),
        }];
        index.index_file("lib.rs", &v2, "v2").unwrap();

        assert!(index.lookup("old_fn").is_empty());
        assert_eq!(index.lookup("new_fn").len(), 1);
    }

    #[test]
    fn file_hash_skip_unchanged() {
        let (index, _tmp) = make_test_index();
        let symbols = vec![xiaolin_treesitter::Symbol {
            name: "foo".into(),
            kind: xiaolin_treesitter::SymbolKind::Function,
            start_line: 1,
            end_line: 3,
            start_col: 1,
            signature: "fn foo()".into(),
        }];
        index.index_file("a.rs", &symbols, "hash1").unwrap();

        let stored = index.file_hash("a.rs").unwrap();
        assert_eq!(stored, Some("hash1".into()));
    }

    #[test]
    fn symbols_in_file_returns_ordered() {
        let (index, _tmp) = make_test_index();
        let symbols = vec![
            xiaolin_treesitter::Symbol {
                name: "beta".into(),
                kind: xiaolin_treesitter::SymbolKind::Function,
                start_line: 20,
                end_line: 30,
                start_col: 1,
                signature: "fn beta()".into(),
            },
            xiaolin_treesitter::Symbol {
                name: "alpha".into(),
                kind: xiaolin_treesitter::SymbolKind::Struct,
                start_line: 1,
                end_line: 10,
                start_col: 1,
                signature: "struct alpha".into(),
            },
        ];
        index.index_file("mod.rs", &symbols, "h").unwrap();

        let result = index.symbols_in_file("mod.rs");
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].name, "alpha");
        assert_eq!(result[1].name, "beta");
    }

    #[test]
    fn find_references_via_signature() {
        let (index, _tmp) = make_test_index();
        let symbols = vec![xiaolin_treesitter::Symbol {
            name: "handle".into(),
            kind: xiaolin_treesitter::SymbolKind::Function,
            start_line: 5,
            end_line: 15,
            start_col: 1,
            signature: "fn handle(req: Request) -> process_data(req)".into(),
        }];
        index.index_file("handler.rs", &symbols, "h1").unwrap();

        let refs = index.find_references("process_data");
        assert!(!refs.is_empty(), "should find reference in signature");
    }

    #[test]
    fn symbol_count_works() {
        let (index, _tmp) = make_test_index();
        assert_eq!(index.symbol_count(), 0);
        let symbols = vec![
            xiaolin_treesitter::Symbol {
                name: "a".into(),
                kind: xiaolin_treesitter::SymbolKind::Function,
                start_line: 1,
                end_line: 2,
                start_col: 1,
                signature: "fn a()".into(),
            },
            xiaolin_treesitter::Symbol {
                name: "b".into(),
                kind: xiaolin_treesitter::SymbolKind::Struct,
                start_line: 3,
                end_line: 5,
                start_col: 1,
                signature: "struct b".into(),
            },
        ];
        index.index_file("test.rs", &symbols, "h").unwrap();
        assert_eq!(index.symbol_count(), 2);
    }
}
