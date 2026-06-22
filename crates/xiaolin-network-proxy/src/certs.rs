//! MITM CA certificate management.
//!
//! Manages a local Certificate Authority for MITM TLS interception.
//! The CA is automatically generated on first use and stored in
//! `$HOME/.xiaolin/proxy/`. Each target host gets a dynamically
//! issued leaf certificate signed by this CA.

use anyhow::{Context as _, Result, anyhow};
use rcgen::{
    BasicConstraints, CertificateParams, DistinguishedName, DnType, ExtendedKeyUsagePurpose,
    IsCa, KeyPair, KeyUsagePurpose, SanType, PKCS_ECDSA_P256_SHA256,
};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, pem::PemObject};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio_rustls::TlsAcceptor;
use tracing::info;

const MANAGED_MITM_CA_DIR: &str = "proxy";
const MANAGED_MITM_CA_CERT: &str = "ca.pem";
const MANAGED_MITM_CA_KEY: &str = "ca.key";

/// Manages a MITM CA capable of dynamically issuing leaf certificates.
pub struct ManagedMitmCa {
    ca_cert_pem: String,
    ca_key_pair: KeyPair,
}

impl ManagedMitmCa {
    /// Load an existing CA or generate a new one.
    pub fn load_or_create() -> Result<Self> {
        let (ca_cert_pem, ca_key_pem) = load_or_create_ca()?;
        let ca_key_pair =
            KeyPair::from_pem(&ca_key_pem).context("failed to parse CA key")?;
        Ok(Self {
            ca_cert_pem,
            ca_key_pair,
        })
    }

    /// Create from explicit PEM strings (for testing).
    pub fn from_pem(cert_pem: &str, key_pem: &str) -> Result<Self> {
        let ca_key_pair = KeyPair::from_pem(key_pem).context("failed to parse CA key")?;
        Ok(Self {
            ca_cert_pem: cert_pem.to_string(),
            ca_key_pair,
        })
    }

    /// Return the CA certificate PEM for trust anchoring.
    pub fn ca_cert_pem(&self) -> &str {
        &self.ca_cert_pem
    }

    /// Build a TLS acceptor for the given target host.
    ///
    /// Issues a leaf certificate signed by this CA, valid for the
    /// requested host name (or IP address).
    pub fn tls_acceptor_for_host(&self, host: &str) -> Result<TlsAcceptor> {
        let (cert_pem, key_pem) = self.issue_host_certificate_pem(host)?;
        let cert = CertificateDer::from_pem_slice(cert_pem.as_bytes())
            .context("failed to parse host cert PEM")?;
        let key = PrivateKeyDer::from_pem_slice(key_pem.as_bytes())
            .context("failed to parse host key PEM")?;
        let mut server_config =
            rustls::ServerConfig::builder_with_protocol_versions(rustls::ALL_VERSIONS)
                .with_no_client_auth()
                .with_single_cert(vec![cert], key)
                .context("failed to build rustls server config")?;
        server_config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
        Ok(TlsAcceptor::from(Arc::new(server_config)))
    }

    fn issue_host_certificate_pem(&self, host: &str) -> Result<(String, String)> {
        let mut params = if let Ok(ip) = host.parse::<IpAddr>() {
            let mut params = CertificateParams::new(Vec::new())
                .map_err(|err| anyhow!("failed to create cert params: {err}"))?;
            params.subject_alt_names.push(SanType::IpAddress(ip));
            params
        } else {
            CertificateParams::new(vec![host.to_string()])
                .map_err(|err| anyhow!("failed to create cert params: {err}"))?
        };

        params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
        params.key_usages = vec![
            KeyUsagePurpose::DigitalSignature,
            KeyUsagePurpose::KeyEncipherment,
        ];

        let leaf_key = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256)
            .map_err(|err| anyhow!("failed to generate host key pair: {err}"))?;

        let ca_params = CertificateParams::from_ca_cert_pem(&self.ca_cert_pem)
            .map_err(|err| anyhow!("failed to parse CA cert for signing: {err}"))?;
        let ca_cert = ca_params
            .self_signed(&self.ca_key_pair)
            .map_err(|err| anyhow!("failed to self-sign CA cert: {err}"))?;

        let cert = params
            .signed_by(&leaf_key, &ca_cert, &self.ca_key_pair)
            .map_err(|err| anyhow!("failed to sign host cert: {err}"))?;

        Ok((cert.pem(), leaf_key.serialize_pem()))
    }
}

// ── CA persistence ──────────────────────────────────────────────────────────

fn managed_ca_paths() -> Result<(PathBuf, PathBuf)> {
    let home = dirs::home_dir()
        .ok_or_else(|| anyhow!("cannot determine home directory for managed MITM CA"))?;
    let proxy_dir = home.join(".xiaolin").join(MANAGED_MITM_CA_DIR);
    Ok((
        proxy_dir.join(MANAGED_MITM_CA_CERT),
        proxy_dir.join(MANAGED_MITM_CA_KEY),
    ))
}

fn load_or_create_ca() -> Result<(String, String)> {
    // TODO(security): encrypt the MITM CA private key at rest; file mode is 0o600 only.
    let (cert_path, key_path) = managed_ca_paths()?;

    if cert_path.exists() || key_path.exists() {
        if !cert_path.exists() || !key_path.exists() {
            return Err(anyhow!(
                "both managed MITM CA files must exist (cert={}, key={})",
                cert_path.display(),
                key_path.display()
            ));
        }
        validate_existing_ca_key_file(&key_path)?;
        let cert_pem = fs::read_to_string(&cert_path)
            .with_context(|| format!("failed to read CA cert {}", cert_path.display()))?;
        let key_pem = fs::read_to_string(&key_path)
            .with_context(|| format!("failed to read CA key {}", key_path.display()))?;
        return Ok((cert_pem, key_pem));
    }

    if let Some(parent) = cert_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    if let Some(parent) = key_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }

    let (cert_pem, key_pem) = generate_ca()?;
    write_atomic_create_new(&key_path, key_pem.as_bytes(), 0o600)
        .with_context(|| format!("failed to persist CA key {}", key_path.display()))?;
    if let Err(err) = write_atomic_create_new(&cert_path, cert_pem.as_bytes(), 0o644)
        .with_context(|| format!("failed to persist CA cert {}", cert_path.display()))
    {
        let _ = fs::remove_file(&key_path);
        return Err(err);
    }
    info!(
        "generated MITM CA (cert_path={}, key_path={})",
        cert_path.display(),
        key_path.display()
    );
    Ok((cert_pem, key_pem))
}

fn generate_ca() -> Result<(String, String)> {
    let mut params = CertificateParams::default();
    params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    params.key_usages = vec![
        KeyUsagePurpose::KeyCertSign,
        KeyUsagePurpose::DigitalSignature,
        KeyUsagePurpose::KeyEncipherment,
    ];
    let mut dn = DistinguishedName::new();
    dn.push(DnType::CommonName, "XiaoLin Network Proxy MITM CA");
    params.distinguished_name = dn;

    let key_pair = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256)
        .map_err(|err| anyhow!("failed to generate CA key pair: {err}"))?;
    let cert = params
        .self_signed(&key_pair)
        .map_err(|err| anyhow!("failed to generate CA cert: {err}"))?;
    Ok((cert.pem(), key_pair.serialize_pem()))
}

// ── Atomic file write ───────────────────────────────────────────────────────

fn write_atomic_create_new(path: &Path, contents: &[u8], mode: u32) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow!("missing parent directory"))?;

    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let pid = std::process::id();
    let file_name = path.file_name().unwrap_or_default().to_string_lossy();
    let tmp_path = parent.join(format!(".{file_name}.tmp.{pid}.{nanos}"));

    let mut file = open_create_new_with_mode(&tmp_path, mode)?;
    file.write_all(contents)
        .with_context(|| format!("failed to write {}", tmp_path.display()))?;
    file.sync_all()
        .with_context(|| format!("failed to fsync {}", tmp_path.display()))?;
    drop(file);

    match fs::hard_link(&tmp_path, path) {
        Ok(()) => {
            fs::remove_file(&tmp_path)
                .with_context(|| format!("failed to remove {}", tmp_path.display()))?;
        }
        Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
            let _ = fs::remove_file(&tmp_path);
            return Err(anyhow!(
                "refusing to overwrite existing file {}",
                path.display()
            ));
        }
        Err(_) => {
            if path.exists() {
                let _ = fs::remove_file(&tmp_path);
                return Err(anyhow!(
                    "refusing to overwrite existing file {}",
                    path.display()
                ));
            }
            fs::rename(&tmp_path, path).with_context(|| {
                format!(
                    "failed to rename {} -> {}",
                    tmp_path.display(),
                    path.display()
                )
            })?;
        }
    }

    let dir =
        File::open(parent).with_context(|| format!("failed to open {}", parent.display()))?;
    dir.sync_all()
        .with_context(|| format!("failed to fsync {}", parent.display()))?;

    Ok(())
}

// ── Validation ──────────────────────────────────────────────────────────────

#[cfg(unix)]
fn validate_existing_ca_key_file(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("failed to stat CA key {}", path.display()))?;
    if metadata.file_type().is_symlink() {
        return Err(anyhow!(
            "refusing to use symlink for managed MITM CA key {}",
            path.display()
        ));
    }
    if !metadata.is_file() {
        return Err(anyhow!(
            "managed MITM CA key is not a regular file: {}",
            path.display()
        ));
    }

    let mode = metadata.permissions().mode() & 0o777;
    if mode & 0o077 != 0 {
        return Err(anyhow!(
            "managed MITM CA key {} must not be group/world accessible (mode={mode:o}; expected <= 600)",
            path.display()
        ));
    }

    Ok(())
}

#[cfg(not(unix))]
fn validate_existing_ca_key_file(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(unix)]
fn open_create_new_with_mode(path: &Path, mode: u32) -> Result<File> {
    use std::os::unix::fs::OpenOptionsExt;

    OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(mode)
        .open(path)
        .with_context(|| format!("failed to create {}", path.display()))
}

#[cfg(not(unix))]
fn open_create_new_with_mode(path: &Path, _mode: u32) -> Result<File> {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .with_context(|| format!("failed to create {}", path.display()))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use tempfile::tempdir;

    #[test]
    fn generate_ca_produces_valid_pem() {
        let (cert_pem, key_pem) = generate_ca().unwrap();
        assert!(cert_pem.contains("BEGIN CERTIFICATE"));
        assert!(key_pem.contains("BEGIN"));
    }

    #[test]
    fn managed_mitm_ca_issue_certificate() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let (cert_pem, key_pem) = generate_ca().unwrap();
        let ca = ManagedMitmCa::from_pem(&cert_pem, &key_pem).unwrap();
        let acceptor = ca.tls_acceptor_for_host("example.com");
        assert!(acceptor.is_ok());
    }

    #[test]
    fn managed_mitm_ca_issue_ip_certificate() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let (cert_pem, key_pem) = generate_ca().unwrap();
        let ca = ManagedMitmCa::from_pem(&cert_pem, &key_pem).unwrap();
        let acceptor = ca.tls_acceptor_for_host("127.0.0.1");
        assert!(acceptor.is_ok());
    }

    #[test]
    fn validate_existing_ca_key_file_rejects_group_world_permissions() {
        let dir = tempdir().unwrap();
        let key_path = dir.path().join("ca.key");
        fs::write(&key_path, "key").unwrap();
        fs::set_permissions(&key_path, fs::Permissions::from_mode(0o644)).unwrap();

        let err = validate_existing_ca_key_file(&key_path).unwrap_err();
        assert!(err.to_string().contains("group/world accessible"));
    }

    #[test]
    fn validate_existing_ca_key_file_rejects_symlink() {
        use std::os::unix::fs::symlink;

        let dir = tempdir().unwrap();
        let target = dir.path().join("real.key");
        let link = dir.path().join("ca.key");
        fs::write(&target, "key").unwrap();
        symlink(&target, &link).unwrap();

        let err = validate_existing_ca_key_file(&link).unwrap_err();
        assert!(err.to_string().contains("symlink"));
    }

    #[test]
    fn validate_existing_ca_key_file_allows_private_permissions() {
        let dir = tempdir().unwrap();
        let key_path = dir.path().join("ca.key");
        fs::write(&key_path, "key").unwrap();
        fs::set_permissions(&key_path, fs::Permissions::from_mode(0o600)).unwrap();

        validate_existing_ca_key_file(&key_path).unwrap();
    }

    #[test]
    fn write_atomic_create_new_succeeds() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.txt");
        write_atomic_create_new(&path, b"hello", 0o600).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "hello");
    }

    #[test]
    fn write_atomic_create_new_refuses_overwrite() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("existing.txt");
        write_atomic_create_new(&path, b"first", 0o600).unwrap();
        let err = write_atomic_create_new(&path, b"second", 0o600).unwrap_err();
        assert!(err.to_string().contains("overwrite"));
    }
}
