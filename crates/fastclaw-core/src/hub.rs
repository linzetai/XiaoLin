use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

const DEFAULT_REGISTRY_URL: &str = "https://hub.fastclaw.dev/api/v1";

/// ClawHub skill marketplace client.
///
/// Supports discovering, searching, and installing SKILL.md packages
/// from a central registry. Falls back to GitHub-based skill repos
/// when the registry is unavailable.
pub struct HubClient {
    http: reqwest::Client,
    registry_url: String,
    install_dir: PathBuf,
}

/// A skill package published to ClawHub.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillPackage {
    pub id: String,
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub downloads: u64,
    #[serde(default)]
    pub repository: Option<String>,
    #[serde(default)]
    pub homepage: Option<String>,
}

/// Search result from the hub.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub packages: Vec<SkillPackage>,
    pub total: u64,
}

/// Installation result.
#[derive(Debug)]
pub struct InstallResult {
    pub skill_id: String,
    pub version: String,
    pub install_path: PathBuf,
    pub files: Vec<String>,
}

impl HubClient {
    pub fn new(registry_url: Option<&str>, install_dir: &Path) -> Self {
        Self {
            http: reqwest::Client::builder()
                .user_agent("FastClaw-Hub/0.1.0")
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .unwrap_or_default(),
            registry_url: registry_url
                .unwrap_or(DEFAULT_REGISTRY_URL)
                .trim_end_matches('/')
                .to_string(),
            install_dir: install_dir.to_path_buf(),
        }
    }

    /// Create a client that installs to the global ~/.fastclaw/skills/ directory.
    pub fn with_defaults() -> Self {
        let install_dir = crate::skill::resolve_global_skills_dir();
        Self::new(None, &install_dir)
    }

    /// Search for skills by query string.
    pub async fn search(&self, query: &str, limit: usize) -> anyhow::Result<SearchResult> {
        let url = format!("{}/skills/search", self.registry_url);
        let resp = self
            .http
            .get(&url)
            .query(&[("q", query), ("limit", &limit.to_string())])
            .send()
            .await?;

        if resp.status().is_success() {
            let result: SearchResult = resp.json().await?;
            return Ok(result);
        }

        // Fallback: search via GitHub API if registry is unavailable
        tracing::debug!("hub registry unavailable, trying GitHub fallback");
        self.search_github_fallback(query, limit).await
    }

    /// List popular/featured skills.
    pub async fn featured(&self, limit: usize) -> anyhow::Result<Vec<SkillPackage>> {
        let url = format!("{}/skills/featured", self.registry_url);
        let resp = self
            .http
            .get(&url)
            .query(&[("limit", &limit.to_string())])
            .send()
            .await;

        match resp {
            Ok(r) if r.status().is_success() => {
                let packages: Vec<SkillPackage> = r.json().await?;
                Ok(packages)
            }
            _ => Ok(Vec::new()),
        }
    }

    /// Install a skill package by ID. Downloads the SKILL.md and any
    /// associated files into the install directory.
    pub async fn install(
        &self,
        skill_id: &str,
        version: Option<&str>,
    ) -> anyhow::Result<InstallResult> {
        let url = format!(
            "{}/skills/{}/download{}",
            self.registry_url,
            skill_id,
            version.map(|v| format!("?version={v}")).unwrap_or_default()
        );

        let resp = self.http.get(&url).send().await;

        match resp {
            Ok(r) if r.status().is_success() => {
                let pkg: SkillPackageDownload = r.json().await?;
                self.install_from_download(skill_id, &pkg).await
            }
            Ok(r) => {
                let status = r.status();
                let text = r.text().await.unwrap_or_default();

                // Try GitHub fallback
                if let Some(repo) = self.resolve_github_repo(skill_id).await? {
                    return self.install_from_github(&repo, skill_id).await;
                }

                anyhow::bail!("skill not found: {skill_id} (hub: {status} — {text})")
            }
            Err(_) => {
                // Registry offline, try GitHub
                if let Some(repo) = self.resolve_github_repo(skill_id).await? {
                    return self.install_from_github(&repo, skill_id).await;
                }
                anyhow::bail!("hub registry unavailable and no GitHub fallback for: {skill_id}")
            }
        }
    }

    /// Uninstall a skill by removing its directory.
    pub fn uninstall(&self, skill_id: &str) -> anyhow::Result<()> {
        let skill_dir = self.install_dir.join(skill_id);
        if skill_dir.exists() {
            std::fs::remove_dir_all(&skill_dir)?;
            tracing::info!(skill_id, path = %skill_dir.display(), "skill uninstalled");
        } else {
            tracing::warn!(skill_id, "skill not found in install directory");
        }
        Ok(())
    }

    /// List installed skills from the install directory.
    pub fn list_installed(&self) -> anyhow::Result<Vec<String>> {
        let mut installed = Vec::new();
        if !self.install_dir.exists() {
            return Ok(installed);
        }
        for entry in std::fs::read_dir(&self.install_dir)? {
            let entry = entry?;
            if entry.path().is_dir() {
                let skill_md = entry.path().join("SKILL.md");
                if skill_md.exists() {
                    if let Some(name) = entry.file_name().to_str() {
                        installed.push(name.to_string());
                    }
                }
            }
        }
        installed.sort();
        Ok(installed)
    }

    async fn install_from_download(
        &self,
        skill_id: &str,
        pkg: &SkillPackageDownload,
    ) -> anyhow::Result<InstallResult> {
        let skill_dir = self.install_dir.join(skill_id);
        std::fs::create_dir_all(&skill_dir)?;

        let mut files = Vec::new();

        for file in &pkg.files {
            if file.name.contains("..") || file.name.starts_with('/') {
                anyhow::bail!(
                    "path traversal in skill package file name: {}",
                    file.name
                );
            }
            let file_path = skill_dir.join(&file.name);
            let canonical_dir = std::fs::canonicalize(&skill_dir)?;
            if let Ok(canonical_file) = std::fs::canonicalize(
                file_path.parent().unwrap_or(&file_path),
            ) {
                if !canonical_file.starts_with(&canonical_dir) {
                    anyhow::bail!(
                        "path traversal detected: {} escapes skill directory",
                        file.name
                    );
                }
            }
            if let Some(parent) = file_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&file_path, &file.content)?;
            files.push(file.name.clone());
        }

        tracing::info!(
            skill_id,
            version = %pkg.version,
            files = files.len(),
            path = %skill_dir.display(),
            "skill installed from hub"
        );

        Ok(InstallResult {
            skill_id: skill_id.to_string(),
            version: pkg.version.clone(),
            install_path: skill_dir,
            files,
        })
    }

    async fn install_from_github(
        &self,
        repo: &str,
        skill_id: &str,
    ) -> anyhow::Result<InstallResult> {
        let raw_url = format!("https://raw.githubusercontent.com/{}/main/SKILL.md", repo);

        let resp = self.http.get(&raw_url).send().await?;
        if !resp.status().is_success() {
            let alt_url = format!("https://raw.githubusercontent.com/{}/master/SKILL.md", repo);
            let resp2 = self.http.get(&alt_url).send().await?;
            if !resp2.status().is_success() {
                anyhow::bail!("failed to fetch SKILL.md from GitHub repo: {repo}");
            }
            let content = resp2.text().await?;
            return self.save_skill(skill_id, &content, repo);
        }

        let content = resp.text().await?;
        self.save_skill(skill_id, &content, repo)
    }

    fn save_skill(
        &self,
        skill_id: &str,
        content: &str,
        source: &str,
    ) -> anyhow::Result<InstallResult> {
        let skill_dir = self.install_dir.join(skill_id);
        std::fs::create_dir_all(&skill_dir)?;
        let skill_path = skill_dir.join("SKILL.md");
        std::fs::write(&skill_path, content)?;

        tracing::info!(
            skill_id,
            source,
            path = %skill_dir.display(),
            "skill installed from GitHub"
        );

        Ok(InstallResult {
            skill_id: skill_id.to_string(),
            version: "latest".to_string(),
            install_path: skill_dir,
            files: vec!["SKILL.md".to_string()],
        })
    }

    async fn resolve_github_repo(&self, skill_id: &str) -> anyhow::Result<Option<String>> {
        // Convention: skill_id can be "owner/repo" format for GitHub skills
        if skill_id.contains('/') && !skill_id.starts_with("http") {
            return Ok(Some(skill_id.to_string()));
        }

        // Try to look up in a known skill index
        let index_url = format!("{}/skills/{}/meta", self.registry_url, skill_id);
        if let Ok(resp) = self.http.get(&index_url).send().await {
            if resp.status().is_success() {
                if let Ok(pkg) = resp.json::<SkillPackage>().await {
                    if let Some(repo) = pkg.repository {
                        let repo = repo
                            .trim_start_matches("https://github.com/")
                            .trim_end_matches(".git")
                            .to_string();
                        return Ok(Some(repo));
                    }
                }
            }
        }

        Ok(None)
    }

    async fn search_github_fallback(
        &self,
        query: &str,
        limit: usize,
    ) -> anyhow::Result<SearchResult> {
        let search_url = format!(
            "https://api.github.com/search/repositories?q={}+topic:fastclaw-skill&per_page={}",
            urlencoded(query),
            limit,
        );

        let resp = self
            .http
            .get(&search_url)
            .header("Accept", "application/vnd.github+json")
            .send()
            .await?;

        if !resp.status().is_success() {
            return Ok(SearchResult {
                packages: Vec::new(),
                total: 0,
            });
        }

        let json: serde_json::Value = resp.json().await?;
        let total = json
            .get("total_count")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let items = json.get("items").and_then(|v| v.as_array());

        let packages = items
            .map(|arr| {
                arr.iter()
                    .map(|item| SkillPackage {
                        id: item
                            .get("full_name")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string(),
                        name: item
                            .get("name")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string(),
                        version: "latest".to_string(),
                        description: item
                            .get("description")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string(),
                        author: item
                            .get("owner")
                            .and_then(|o| o.get("login"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string(),
                        tags: item
                            .get("topics")
                            .and_then(|v| v.as_array())
                            .map(|a| {
                                a.iter()
                                    .filter_map(|t| t.as_str().map(String::from))
                                    .collect()
                            })
                            .unwrap_or_default(),
                        downloads: item
                            .get("stargazers_count")
                            .and_then(|v| v.as_u64())
                            .unwrap_or(0),
                        repository: item
                            .get("html_url")
                            .and_then(|v| v.as_str())
                            .map(String::from),
                        homepage: item
                            .get("homepage")
                            .and_then(|v| v.as_str())
                            .map(String::from),
                    })
                    .collect()
            })
            .unwrap_or_default();

        Ok(SearchResult { packages, total })
    }
}

#[derive(Deserialize)]
struct SkillPackageDownload {
    version: String,
    files: Vec<SkillFile>,
}

#[derive(Deserialize)]
struct SkillFile {
    name: String,
    content: String,
}

fn urlencoded(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            ' ' => "+".to_string(),
            c if c.is_alphanumeric() || "-._~".contains(c) => c.to_string(),
            c => format!("%{:02X}", c as u32),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urlencoded_basic() {
        assert_eq!(urlencoded("hello world"), "hello+world");
        assert_eq!(urlencoded("feishu-skill"), "feishu-skill");
    }

    #[test]
    fn list_installed_empty() {
        let tmp = std::env::temp_dir().join("fastclaw_hub_test_empty");
        let _ = std::fs::remove_dir_all(&tmp);
        let client = HubClient::new(None, &tmp);
        let installed = client.list_installed().unwrap();
        assert!(installed.is_empty());
    }

    #[test]
    fn install_and_uninstall_local() {
        let tmp = std::env::temp_dir().join("fastclaw_hub_test_install");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        let client = HubClient::new(None, &tmp);

        let skill_dir = tmp.join("test-skill");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(skill_dir.join("SKILL.md"), "# Test Skill").unwrap();

        let installed = client.list_installed().unwrap();
        assert_eq!(installed, vec!["test-skill"]);

        client.uninstall("test-skill").unwrap();
        assert!(!skill_dir.exists());

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
