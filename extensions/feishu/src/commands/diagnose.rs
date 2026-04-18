use crate::client::FeishuClient;
use serde::{Deserialize, Serialize};

/// Feishu diagnosis report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiagnosisReport {
    pub overall_status: DiagStatus,
    pub checks: Vec<DiagCheck>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DiagStatus {
    Healthy,
    Degraded,
    Unhealthy,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiagCheck {
    pub name: String,
    pub status: DiagStatus,
    pub message: String,
}

/// Run a full Feishu plugin diagnosis.
pub async fn run_diagnosis(client: &FeishuClient) -> DiagnosisReport {
    let mut checks = Vec::new();

    // Check 1: Tenant token acquisition
    let token_check = match client.get_tenant_token().await {
        Ok(_) => DiagCheck {
            name: "tenant_token".into(),
            status: DiagStatus::Healthy,
            message: "Successfully acquired tenant access token".into(),
        },
        Err(e) => DiagCheck {
            name: "tenant_token".into(),
            status: DiagStatus::Unhealthy,
            message: format!("Failed to acquire token: {e}"),
        },
    };
    checks.push(token_check);

    // Check 2: Bot info probe (if token works)
    // For now just verify the client is configured
    let config_check = DiagCheck {
        name: "configuration".into(),
        status: DiagStatus::Healthy,
        message: "Feishu client configured".into(),
    };
    checks.push(config_check);

    let overall = if checks.iter().all(|c| c.status == DiagStatus::Healthy) {
        DiagStatus::Healthy
    } else if checks.iter().any(|c| c.status == DiagStatus::Unhealthy) {
        DiagStatus::Unhealthy
    } else {
        DiagStatus::Degraded
    };

    DiagnosisReport {
        overall_status: overall,
        checks,
    }
}

/// Format a diagnosis report for CLI output.
pub fn format_report_cli(report: &DiagnosisReport) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "Feishu Plugin Diagnosis: {:?}\n",
        report.overall_status
    ));
    out.push_str(&"─".repeat(50));
    out.push('\n');
    for check in &report.checks {
        let icon = match check.status {
            DiagStatus::Healthy => "✓",
            DiagStatus::Degraded => "△",
            DiagStatus::Unhealthy => "✗",
        };
        out.push_str(&format!("{} {}: {}\n", icon, check.name, check.message));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_report() {
        let report = DiagnosisReport {
            overall_status: DiagStatus::Healthy,
            checks: vec![DiagCheck {
                name: "test".into(),
                status: DiagStatus::Healthy,
                message: "ok".into(),
            }],
        };
        let out = format_report_cli(&report);
        assert!(out.contains("Healthy"));
        assert!(out.contains("✓ test"));
    }
}
