//! Rule-based user profile extraction (no LLM calls).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// How the user tends to phrase requests (inferred from keywords).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum CommunicationStyle {
    #[default]
    Mixed,
    Detailed,
    Concise,
}

/// Cross-session user signals inferred from message text.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserProfile {
    pub languages: HashMap<String, u32>,
    pub frameworks: HashMap<String, u32>,
    pub communication_style: CommunicationStyle,
    pub expertise_tags: Vec<String>,
    pub last_updated: DateTime<Utc>,
}

impl Default for UserProfile {
    fn default() -> Self {
        Self {
            languages: HashMap::new(),
            frameworks: HashMap::new(),
            communication_style: CommunicationStyle::Mixed,
            expertise_tags: Vec::new(),
            last_updated: Utc::now(),
        }
    }
}

impl UserProfile {
    /// From a single message, update counts and tags using keyword / heuristic rules only.
    pub fn extract_from_message(&mut self, content: &str) {
        let lower = content.to_lowercase();

        for (canonical, needles) in LANGUAGE_KEYWORDS {
            for n in *needles {
                let c = count_occurrences_ci(content, &lower, n);
                if c > 0 {
                    *self.languages.entry((*canonical).to_string()).or_insert(0) += c;
                }
            }
        }

        for (canonical, needles) in FRAMEWORK_KEYWORDS {
            for n in *needles {
                let c = count_occurrences_ci(content, &lower, n);
                if c > 0 {
                    *self.frameworks.entry((*canonical).to_string()).or_insert(0) += c;
                }
            }
        }

        self.merge_expertise_tags(content, &lower);
        self.update_communication_style(content, &lower);
        self.last_updated = Utc::now();
    }

    fn update_communication_style(&mut self, content: &str, lower: &str) {
        let mut detailed_score = 0i32;
        let mut concise_score = 0i32;

        for w in DETAILED_HINTS {
            if lower.contains(w) {
                detailed_score += 1;
            }
        }
        for w in CONCISE_HINTS {
            if lower.contains(w) {
                concise_score += 1;
            }
        }

        if content.chars().count() > 800 {
            detailed_score += 1;
        } else if content.chars().count() < 120 && content.lines().count() <= 3 {
            concise_score += 1;
        }

        self.communication_style = match detailed_score.cmp(&concise_score) {
            std::cmp::Ordering::Greater => CommunicationStyle::Detailed,
            std::cmp::Ordering::Less => CommunicationStyle::Concise,
            std::cmp::Ordering::Equal => CommunicationStyle::Mixed,
        };
    }

    fn merge_expertise_tags(&mut self, _content: &str, lower: &str) {
        let mut seen = std::collections::HashSet::<String>::new();
        for t in &self.expertise_tags {
            seen.insert(t.to_lowercase());
        }

        for (tag, triggers) in DOMAIN_TAGS {
            for tr in *triggers {
                if lower.contains(tr) {
                    let t = (*tag).to_string();
                    if seen.insert(t.to_lowercase()) {
                        self.expertise_tags.push(t);
                    }
                    break;
                }
            }
        }

        if looks_like_backend(lower) && seen.insert("backend".into()) {
            self.expertise_tags.push("backend".into());
        }
        if looks_like_frontend(lower) && seen.insert("frontend".into()) {
            self.expertise_tags.push("frontend".into());
        }
        if looks_like_devops(lower) && seen.insert("devops".into()) {
            self.expertise_tags.push("devops".into());
        }
        if looks_like_ml(lower) && seen.insert("machine-learning".into()) {
            self.expertise_tags.push("machine-learning".into());
        }
    }

    /// Compact text suitable for a system or developer message.
    pub fn to_prompt_text(&self) -> String {
        let mut out = String::new();

        if !self.languages.is_empty() {
            let mut langs: Vec<_> = self.languages.iter().collect();
            langs.sort_by(|a, b| b.1.cmp(a.1));
            let top: Vec<String> = langs
                .into_iter()
                .take(8)
                .map(|(k, v)| format!("{k} ({v})"))
                .collect();
            out.push_str("Programming languages (mention counts): ");
            out.push_str(&top.join(", "));
            out.push('\n');
        }

        if !self.frameworks.is_empty() {
            let mut fw: Vec<_> = self.frameworks.iter().collect();
            fw.sort_by(|a, b| b.1.cmp(a.1));
            let top: Vec<String> = fw
                .into_iter()
                .take(10)
                .map(|(k, v)| format!("{k} ({v})"))
                .collect();
            out.push_str("Tools / frameworks: ");
            out.push_str(&top.join(", "));
            out.push('\n');
        }

        let style = match self.communication_style {
            CommunicationStyle::Detailed => "Prefers detailed explanations.",
            CommunicationStyle::Concise => "Prefers concise answers.",
            CommunicationStyle::Mixed => "Mixed detail level.",
        };
        out.push_str("Communication: ");
        out.push_str(style);
        out.push('\n');

        if !self.expertise_tags.is_empty() {
            out.push_str("Domains: ");
            out.push_str(&self.expertise_tags.join(", "));
            out.push('\n');
        }

        out.push_str("Profile last updated (UTC): ");
        out.push_str(&self.last_updated.to_rfc3339());

        out
    }
}

fn count_occurrences_ci(_content: &str, lower: &str, needle: &str) -> u32 {
    let n = needle.to_lowercase();
    if n.is_empty() {
        return 0;
    }
    let mut count = 0usize;
    let mut start = 0usize;
    while let Some(i) = lower[start..].find(&n) {
        count += 1;
        start += i + n.len().max(1);
    }
    count as u32
}

fn looks_like_backend(lower: &str) -> bool {
    [
        "postgres",
        "mysql",
        "redis",
        "kafka",
        "api 网关",
        "微服务",
        "grpc",
        "rest api",
    ]
    .iter()
    .any(|k| lower.contains(k))
}

fn looks_like_frontend(lower: &str) -> bool {
    [
        "react",
        "vue",
        "svelte",
        "webpack",
        "vite",
        "css",
        "dom",
        "浏览器",
    ]
    .iter()
    .any(|k| lower.contains(k))
}

fn looks_like_devops(lower: &str) -> bool {
    [
        "kubernetes",
        "k8s",
        "docker",
        "ci/cd",
        "github actions",
        "terraform",
        "ansible",
        "helm",
    ]
    .iter()
    .any(|k| lower.contains(k))
}

fn looks_like_ml(lower: &str) -> bool {
    [
        "pytorch",
        "tensorflow",
        "transformer",
        "llm",
        "embedding",
        "fine-tune",
        "训练",
        "推理",
    ]
    .iter()
    .any(|k| lower.contains(k))
}

const DETAILED_HINTS: &[&str] = &[
    "详细",
    "展开",
    "step by step",
    "explain in detail",
    "deep dive",
    "尽可能详细",
];

const CONCISE_HINTS: &[&str] = &[
    "简洁",
    "简短",
    "一句话",
    "tl;dr",
    "tldr",
    "be brief",
    "concise",
    "不要废话",
];

const LANGUAGE_KEYWORDS: &[(&str, &[&str])] = &[
    ("Rust", &["rust", "cargo", "rustc", "tokio", "serde"]),
    (
        "Python",
        &["python", "pip", "pytest", "django", "flask", "pandas"],
    ),
    ("TypeScript", &["typescript", " ts ", ".ts", "tsx"]),
    (
        "JavaScript",
        &["javascript", " nodejs", " node.js", "npm", "yarn"],
    ),
    ("Go", &[" golang", " go mod", "goroutine"]),
    ("C++", &["c++", " cpp", "clang", "cmake"]),
    ("C", &[" c99", " c11", " libc"]),
    ("Java", &["java", "maven", "gradle", "spring"]),
    ("Kotlin", &["kotlin", "gradle.kts"]),
    ("Swift", &["swift", "swiftui", "xcode"]),
    ("Ruby", &["ruby", "rails", "gemfile"]),
    ("PHP", &["php", "laravel", "composer"]),
    ("Shell", &["bash", "zsh", "shell script", "sh "]),
    ("SQL", &["sql", "select ", "postgres", "mysql"]),
];

const FRAMEWORK_KEYWORDS: &[(&str, &[&str])] = &[
    ("React", &["react", "jsx", "usestate", "next.js", "nextjs"]),
    ("Vue", &["vue", "nuxt", "vuex", "pinia"]),
    ("Angular", &["angular"]),
    ("Axum", &["axum"]),
    ("Actix", &["actix"]),
    ("Tokio", &["tokio"]),
    ("Kubernetes", &["kubernetes", "k8s"]),
    ("Docker", &["docker", "dockerfile"]),
    ("Terraform", &["terraform"]),
    ("AWS", &["aws", "amazon s3", "lambda", "ec2"]),
    ("GCP", &["gcp", "google cloud"]),
    ("Azure", &["azure"]),
    ("Linux", &["linux", "ubuntu", "debian", "systemd"]),
];

const DOMAIN_TAGS: &[(&str, &[&str])] = &[
    ("web", &["http", "https", "rest", "graphql", "websocket"]),
    (
        "database",
        &["database", "sql", "事务", "索引", "migration"],
    ),
    ("security", &["oauth", "jwt", "加密", "漏洞", "csrf", "xss"]),
    ("mobile", &["ios", "android", "flutter", "react native"]),
    ("embedded", &["嵌入式", "stm32", "arduino", "mcu", "固件"]),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_languages_and_frameworks() {
        let mut p = UserProfile::default();
        p.extract_from_message(
            "We use Rust with tokio and axum for the service, plus a bit of TypeScript in the frontend.",
        );
        assert!(p.languages.get("Rust").copied().unwrap_or(0) >= 1);
        assert!(p.frameworks.get("Axum").is_some() || p.frameworks.get("Tokio").is_some());
    }

    #[test]
    fn to_prompt_text_non_empty() {
        let mut p = UserProfile::default();
        p.extract_from_message("Help me debug Python pandas code.");
        let t = p.to_prompt_text();
        assert!(t.contains("Python"));
        assert!(t.contains("UTC"));
    }
}
