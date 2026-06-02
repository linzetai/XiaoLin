use crate::policy::is_non_public_ip;
use std::io;
use std::net::SocketAddr;
use tokio::net::TcpStream;

/// Pre-connection policy for TCP targets.
///
/// Before establishing a TCP connection, this checks whether the target
/// address is allowed by policy (e.g., blocking connections to non-public
/// IPs when `allow_local_binding` is false).
#[derive(Clone, Debug)]
pub struct TargetCheckedTcpConnector {
    allow_local_binding: bool,
    has_upstream_proxy: bool,
}

impl TargetCheckedTcpConnector {
    pub fn new(allow_local_binding: bool) -> Self {
        Self {
            allow_local_binding,
            has_upstream_proxy: false,
        }
    }

    pub fn with_upstream_proxy(mut self, has_upstream: bool) -> Self {
        self.has_upstream_proxy = has_upstream;
        self
    }

    /// Connect to the target address after checking the policy.
    ///
    /// When an upstream proxy is configured, the IP check is skipped because
    /// the actual TCP target will be the upstream proxy, not the logical
    /// destination.
    pub async fn connect(&self, addr: SocketAddr) -> io::Result<TcpStream> {
        if !self.has_upstream_proxy
            && !self.allow_local_binding
            && is_non_public_ip(addr.ip())
        {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "network target rejected by policy",
            ));
        }
        TcpStream::connect(addr).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;
    use tokio::net::TcpListener;

    #[tokio::test]
    async fn rejects_non_public_target_when_local_binding_disabled() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0u16))
            .await
            .expect("bind local listener");
        let target = listener.local_addr().expect("local addr");

        let connector = TargetCheckedTcpConnector::new(false);
        let err = connector.connect(target).await.expect_err("should reject");
        assert_eq!(err.kind(), io::ErrorKind::PermissionDenied);
        assert!(
            format!("{err}").contains("network target rejected by policy"),
            "unexpected error: {err}"
        );
    }

    #[tokio::test]
    async fn allows_non_public_target_when_local_binding_enabled() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0u16))
            .await
            .expect("bind local listener");
        let target = listener.local_addr().expect("local addr");

        let connector = TargetCheckedTcpConnector::new(true);
        let result = connector.connect(target).await;
        assert!(result.is_ok(), "local target should be allowed: {result:?}");
    }

    #[tokio::test]
    async fn allows_non_public_when_upstream_proxy_present() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0u16))
            .await
            .expect("bind local listener");
        let target = listener.local_addr().expect("local addr");

        let connector = TargetCheckedTcpConnector::new(false).with_upstream_proxy(true);
        let result = connector.connect(target).await;
        assert!(
            result.is_ok(),
            "should allow local target with upstream proxy: {result:?}"
        );
    }

    #[tokio::test]
    async fn rejects_private_ip_ranges() {
        let connector = TargetCheckedTcpConnector::new(false);
        let addrs: Vec<SocketAddr> = vec![
            "10.0.0.1:80".parse().unwrap(),
            "172.16.0.1:80".parse().unwrap(),
            "192.168.1.1:80".parse().unwrap(),
        ];
        for addr in addrs {
            let err = connector.connect(addr).await.expect_err(&format!("should reject {addr}"));
            assert_eq!(err.kind(), io::ErrorKind::PermissionDenied);
        }
    }
}
