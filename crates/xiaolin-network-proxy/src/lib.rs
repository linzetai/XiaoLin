pub mod certs;
pub mod config;
pub mod connect_policy;
pub mod http_proxy;
pub mod mitm;
pub mod network_policy;
pub mod policy;
pub mod proxy;
pub mod proxy_env;
pub mod reasons;
pub mod responses;
pub mod runtime;
pub mod socks5;
pub mod state;
pub mod upstream;

pub use config::{
    NetworkDomainPermission, NetworkDomainPermissionEntry, NetworkDomainPermissions, NetworkMode,
    NetworkProxyConfig, NetworkProxySettings, ValidatedUnixSocketPath,
    clamp_bind_addrs, host_and_port_from_network_addr, validate_unix_socket_allowlist_paths,
};
pub use connect_policy::TargetCheckedTcpConnector;
pub use network_policy::{
    NetworkDecision, NetworkDecisionSource, NetworkPolicyDecision, NetworkPolicyRequest,
    NetworkProtocol,
};
pub use policy::{
    DomainPattern, Host, compile_allowlist_globset, compile_denylist_globset, is_loopback_host,
    is_non_public_ip, normalize_host,
};
pub use proxy::{
    NetworkProxy, NetworkProxyBuilder, NetworkProxyHandle, ProxyBuildError,
};
pub use proxy_env::{
    ALLOW_LOCAL_BINDING_ENV_KEY, DEFAULT_NO_PROXY_VALUE, NO_PROXY_ENV_KEYS,
    PROXY_ACTIVE_ENV_KEY, PROXY_ENV_KEYS, PROXY_GIT_SSH_COMMAND_MARKER, PROXY_URL_ENV_KEYS,
    has_proxy_url_env_vars, proxy_loopback_ports_from_env, proxy_url_env_value,
};
pub use certs::ManagedMitmCa;
pub use mitm::{MitmState, MitmUpstreamConfig};
pub use network_policy::{HostBlockDecision, HostBlockReason};
pub use reasons::*;
pub use runtime::{
    BlockedRequest, BlockedRequestArgs, BlockedRequestObserver, ConfigReloader, ConfigSnapshot,
    ConfigState, NetworkProxyAuditMetadata, NetworkProxyState,
};
pub use upstream::{ProxyConfig, ProxyEndpoint, UpstreamClient};
pub use state::{
    NetworkProxyConstraintError, NetworkProxyConstraints, PartialNetworkConfig,
    PartialNetworkProxyConfig, validate_policy_against_constraints,
};
