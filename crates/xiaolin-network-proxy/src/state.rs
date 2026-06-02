use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::config::{NetworkMode, NetworkProxySettings};

/// Constraints on how the proxy may be configured.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NetworkProxyConstraints {
    #[serde(default)]
    pub max_mode: Option<NetworkMode>,
    #[serde(default)]
    pub require_proxy: bool,
    #[serde(default)]
    pub deny_local_binding: bool,
}

/// A partial proxy configuration used for incremental/override config.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PartialNetworkProxyConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<NetworkMode>,
}

/// A partial network configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PartialNetworkConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy: Option<PartialNetworkProxyConfig>,
}

/// Errors from constraint validation.
#[derive(Debug, Clone, Error)]
pub enum NetworkProxyConstraintError {
    #[error("proxy is required but not configured")]
    ProxyRequired,

    #[error("local binding is denied by constraints")]
    LocalBindingDenied,

    #[error("mode '{requested:?}' exceeds max mode '{max:?}'")]
    ModeExceedsMax {
        requested: NetworkMode,
        max: NetworkMode,
    },

    #[error("proxy is disabled but required by constraints")]
    ProxyDisabled,
}

fn network_mode_rank(mode: NetworkMode) -> u8 {
    match mode {
        NetworkMode::Limited => 0,
        NetworkMode::Full => 1,
        NetworkMode::Audit => 2,
        NetworkMode::Off => 3,
    }
}

/// Validate that the given settings satisfy the constraints.
pub fn validate_policy_against_constraints(
    settings: &NetworkProxySettings,
    constraints: &NetworkProxyConstraints,
) -> Result<(), Vec<NetworkProxyConstraintError>> {
    let mut errors = Vec::new();

    if constraints.require_proxy && !settings.enabled {
        errors.push(NetworkProxyConstraintError::ProxyDisabled);
    }

    if constraints.require_proxy && settings.enabled && settings.proxy_url.is_none() {
        errors.push(NetworkProxyConstraintError::ProxyRequired);
    }

    if constraints.deny_local_binding && settings.allow_local_binding {
        errors.push(NetworkProxyConstraintError::LocalBindingDenied);
    }

    if let (Some(max_mode), Some(requested_mode)) = (constraints.max_mode, settings.mode) {
        if network_mode_rank(requested_mode) > network_mode_rank(max_mode) {
            errors.push(NetworkProxyConstraintError::ModeExceedsMax {
                requested: requested_mode,
                max: max_mode,
            });
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_default_settings_pass_default_constraints() {
        let settings = NetworkProxySettings::default();
        let constraints = NetworkProxyConstraints::default();
        assert!(validate_policy_against_constraints(&settings, &constraints).is_ok());
    }

    #[test]
    fn validate_proxy_required_but_disabled() {
        let mut settings = NetworkProxySettings::default();
        settings.enabled = false;
        let constraints = NetworkProxyConstraints {
            require_proxy: true,
            ..Default::default()
        };
        let errors = validate_policy_against_constraints(&settings, &constraints).unwrap_err();
        assert!(errors
            .iter()
            .any(|e| matches!(e, NetworkProxyConstraintError::ProxyDisabled)));
    }

    #[test]
    fn validate_proxy_required_but_no_url() {
        let settings = NetworkProxySettings {
            enabled: true,
            proxy_url: None,
            ..Default::default()
        };
        let constraints = NetworkProxyConstraints {
            require_proxy: true,
            ..Default::default()
        };
        let errors = validate_policy_against_constraints(&settings, &constraints).unwrap_err();
        assert!(errors
            .iter()
            .any(|e| matches!(e, NetworkProxyConstraintError::ProxyRequired)));
    }

    #[test]
    fn validate_local_binding_denied() {
        let mut settings = NetworkProxySettings::default();
        settings.allow_local_binding = true;
        let constraints = NetworkProxyConstraints {
            deny_local_binding: true,
            ..Default::default()
        };
        let errors = validate_policy_against_constraints(&settings, &constraints).unwrap_err();
        assert!(errors
            .iter()
            .any(|e| matches!(e, NetworkProxyConstraintError::LocalBindingDenied)));
    }

    #[test]
    fn validate_mode_exceeds_max() {
        let mut settings = NetworkProxySettings::default();
        settings.mode = Some(NetworkMode::Full);
        let constraints = NetworkProxyConstraints {
            max_mode: Some(NetworkMode::Limited),
            ..Default::default()
        };
        let errors = validate_policy_against_constraints(&settings, &constraints).unwrap_err();
        assert!(errors
            .iter()
            .any(|e| matches!(e, NetworkProxyConstraintError::ModeExceedsMax { .. })));
    }

    #[test]
    fn validate_mode_within_max() {
        let mut settings = NetworkProxySettings::default();
        settings.mode = Some(NetworkMode::Limited);
        let constraints = NetworkProxyConstraints {
            max_mode: Some(NetworkMode::Full),
            ..Default::default()
        };
        assert!(validate_policy_against_constraints(&settings, &constraints).is_ok());
    }

    #[test]
    fn network_mode_rank_ordering() {
        assert!(network_mode_rank(NetworkMode::Limited) < network_mode_rank(NetworkMode::Full));
    }

    #[test]
    fn constraint_error_display() {
        let err = NetworkProxyConstraintError::ProxyRequired;
        assert!(err.to_string().contains("required"));
    }
}
