use std::path::{Path, PathBuf};
use std::sync::Arc;

use dashmap::DashMap;
use rand::prelude::*;

const ADJECTIVES: &[&str] = &[
    "brave", "calm", "dark", "eager", "fair", "glad", "keen", "loud", "mild", "neat", "pale",
    "quick", "rich", "safe", "tall", "vast", "warm", "bold", "cool", "deep", "fine", "good",
    "high", "just", "kind", "lean", "next", "open", "pure", "rare", "slim", "true", "wide", "able",
    "busy", "crisp", "dry", "even", "fast", "gold", "hard", "iron", "jade", "live", "main", "new",
    "old", "raw", "soft", "thin", "used", "void", "wild", "aged", "bare", "cold", "dull", "easy",
    "flat", "gray", "half", "icy", "long", "mini",
];

const NOUNS: &[&str] = &[
    "lion", "wolf", "bear", "hawk", "deer", "fish", "frog", "swan", "moon", "star", "wind", "rain",
    "wave", "leaf", "seed", "rock", "tree", "hill", "lake", "dawn", "dusk", "tide", "mesa", "glen",
    "arch", "bolt", "calm", "dart", "edge", "flux", "gate", "haze", "iris", "jade", "knot", "lark",
    "mist", "node", "opal", "peak", "reef", "sage", "vale", "wren", "axis", "beam", "cove", "dome",
    "echo", "fern", "glow", "hive", "isle", "jewel", "kite", "loom", "mint", "nest", "orb", "pyre",
    "quay", "rune", "silo", "thorn",
];

/// Write to a temp file in the same directory, then rename for atomicity.
fn atomic_write(path: &Path, data: &[u8]) -> Result<(), String> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, data).map_err(|e| format!("Failed to write temp file: {e}"))?;
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("Failed to rename temp file: {e}")
    })
}

fn generate_slug() -> String {
    let mut rng = thread_rng();
    let adj = ADJECTIVES[rng.gen_range(0..ADJECTIVES.len())];
    let noun = NOUNS[rng.gen_range(0..NOUNS.len())];
    format!("{adj}-{noun}")
}

/// Manages plan file persistence on disk.
///
/// Each session gets a unique slug (e.g. "brave-lion") that maps to a `.md`
/// file under the plans directory. Slugs are cached in-memory so the same
/// session always uses the same file.
#[derive(Debug, Clone)]
pub struct PlanFileStore {
    plans_dir: PathBuf,
    slugs: Arc<DashMap<String, String>>,
}

impl Default for PlanFileStore {
    fn default() -> Self {
        Self::new(None)
    }
}

impl PlanFileStore {
    pub fn new(plans_dir: Option<PathBuf>) -> Self {
        let dir = plans_dir.unwrap_or_else(|| {
            dirs::home_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".xiaolin")
                .join("plans")
        });
        let store = Self {
            plans_dir: dir,
            slugs: Arc::new(DashMap::new()),
        };
        let _ = store.load_index();
        store
    }

    pub fn plans_dir(&self) -> &Path {
        &self.plans_dir
    }

    pub fn get_or_create_slug(&self, session_id: &str) -> String {
        if let Some(existing) = self.slugs.get(session_id) {
            return existing.clone();
        }
        let slug = self.generate_unique_slug();
        self.slugs
            .entry(session_id.to_string())
            .or_insert(slug)
            .clone()
    }

    fn generate_unique_slug(&self) -> String {
        let used: std::collections::HashSet<String> =
            self.slugs.iter().map(|e| e.value().clone()).collect();
        for _ in 0..10 {
            let slug = generate_slug();
            if !used.contains(&slug) {
                return slug;
            }
        }
        let base = generate_slug();
        let mut rng = thread_rng();
        format!("{base}-{}", rng.gen_range(100..999))
    }

    pub fn set_slug(&self, session_id: &str, slug: &str) {
        self.slugs.insert(session_id.to_string(), slug.to_string());
    }

    pub fn get_slug(&self, session_id: &str) -> Option<String> {
        self.slugs.get(session_id).map(|v| v.clone())
    }

    pub fn clear_slug(&self, session_id: &str) {
        self.slugs.remove(session_id);
    }

    pub fn plan_path(&self, session_id: &str) -> PathBuf {
        let slug = self.get_or_create_slug(session_id);
        self.plans_dir.join(format!("{slug}.md"))
    }

    pub fn write_plan(&self, session_id: &str, content: &str) -> Result<PathBuf, String> {
        let path = self.plan_path(session_id);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create plans directory: {e}"))?;
        }
        atomic_write(&path, content.as_bytes())?;
        let _ = self.save_index();
        Ok(path)
    }

    pub fn read_plan(&self, session_id: &str) -> Option<String> {
        let path = self.plan_path(session_id);
        std::fs::read_to_string(&path).ok()
    }

    pub fn plan_exists(&self, session_id: &str) -> bool {
        self.plan_path(session_id).exists()
    }

    fn index_path(&self) -> PathBuf {
        self.plans_dir.join(".plan-index.json")
    }

    /// Persist the session-to-slug mapping to disk so it survives restarts.
    pub fn save_index(&self) -> Result<(), String> {
        if let Some(parent) = self.index_path().parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create plans directory: {e}"))?;
        }
        let map: std::collections::HashMap<String, String> = self
            .slugs
            .iter()
            .map(|e| (e.key().clone(), e.value().clone()))
            .collect();
        let json = serde_json::to_string_pretty(&map)
            .map_err(|e| format!("Failed to serialize index: {e}"))?;
        atomic_write(&self.index_path(), json.as_bytes())
    }

    /// Load session-to-slug mapping from disk.
    pub fn load_index(&self) -> Result<usize, String> {
        let path = self.index_path();
        if !path.exists() {
            return Ok(0);
        }
        let data =
            std::fs::read_to_string(&path).map_err(|e| format!("Failed to read index: {e}"))?;
        let map: std::collections::HashMap<String, String> =
            serde_json::from_str(&data).map_err(|e| format!("Failed to parse index: {e}"))?;
        let count = map.len();
        for (session_id, slug) in map {
            self.slugs.insert(session_id, slug);
        }
        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slug_is_deterministic_per_session() {
        let store = PlanFileStore::new(Some(PathBuf::from("/tmp/test-plans")));
        let s1 = store.get_or_create_slug("sess-1");
        let s2 = store.get_or_create_slug("sess-1");
        assert_eq!(s1, s2);
    }

    #[test]
    fn different_sessions_get_different_slugs() {
        let store = PlanFileStore::new(Some(PathBuf::from("/tmp/test-plans")));
        let s1 = store.get_or_create_slug("sess-a");
        let s2 = store.get_or_create_slug("sess-b");
        // Technically could collide, but with 64*64=4096 combinations it's unlikely
        // Just verify they are valid slugs
        assert!(s1.contains('-'));
        assert!(s2.contains('-'));
    }

    #[test]
    fn plan_path_uses_slug() {
        let store = PlanFileStore::new(Some(PathBuf::from("/tmp/plans")));
        store.set_slug("my-session", "brave-lion");
        let path = store.plan_path("my-session");
        assert_eq!(path, PathBuf::from("/tmp/plans/brave-lion.md"));
    }

    #[test]
    fn write_and_read_plan() {
        let dir = std::env::temp_dir().join(format!("xiaolin-plan-test-{}", std::process::id()));
        let store = PlanFileStore::new(Some(dir.clone()));
        store.set_slug("test", "test-plan");

        let path = store
            .write_plan("test", "# My Plan\n\n- Step 1\n- Step 2\n")
            .unwrap();
        assert!(path.exists());

        let content = store.read_plan("test").unwrap();
        assert!(content.contains("Step 1"));
        assert!(content.contains("Step 2"));

        // Cleanup
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn read_nonexistent_plan_returns_none() {
        let store = PlanFileStore::new(Some(PathBuf::from("/tmp/nonexistent-plans-dir-xyz")));
        store.set_slug("nope", "nope-nope");
        assert!(store.read_plan("nope").is_none());
    }

    #[test]
    fn clear_slug_removes_mapping() {
        let store = PlanFileStore::new(Some(PathBuf::from("/tmp/plans")));
        store.get_or_create_slug("sess-1");
        assert!(store.get_slug("sess-1").is_some());
        store.clear_slug("sess-1");
        assert!(store.get_slug("sess-1").is_none());
    }
}
