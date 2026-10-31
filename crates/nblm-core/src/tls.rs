//! Additional trust roots (e.g. a corporate TLS-inspection CA) for all HTTP clients.
//! Certificate and hostname verification remain enabled; built-in roots are retained.
//!
//! Deliberately duplicated in `crates/gemini-chat/src/tls.rs` so the two crates stay independent;
//! keep both copies in sync (only the first environment variable name differs).

use std::path::Path;

use reqwest::{Certificate, ClientBuilder};

use crate::error::{Error, Result};

#[derive(Clone, Default)]
pub(crate) struct TlsRoots(Vec<Certificate>);

impl TlsRoots {
    pub(crate) fn from_env() -> Result<Self> {
        Self::from_lookup(|key| std::env::var_os(key).filter(|value| !value.is_empty()))
    }

    fn from_lookup(lookup: impl Fn(&str) -> Option<std::ffi::OsString>) -> Result<Self> {
        for variable in ["NBLM_CA_BUNDLE", "SSL_CERT_FILE", "REQUESTS_CA_BUNDLE"] {
            if let Some(path) = lookup(variable) {
                // Do not expose paths, file contents or parser diagnostics in errors.
                let bytes = std::fs::read(Path::new(&path)).map_err(|_| {
                    Error::validation(format!("cannot read CA bundle configured by {variable}"))
                })?;
                return Self::from_pem(&bytes, variable);
            }
        }
        Ok(Self::default())
    }

    fn from_pem(bytes: &[u8], variable: &str) -> Result<Self> {
        let certificates = Certificate::from_pem_bundle(bytes).map_err(|_| {
            Error::validation(format!("invalid PEM CA bundle configured by {variable}"))
        })?;
        if certificates.is_empty() {
            return Err(Error::validation(format!(
                "CA bundle configured by {variable} contains no certificates"
            )));
        }
        Ok(Self(certificates))
    }

    pub(crate) fn apply(&self, mut builder: ClientBuilder) -> ClientBuilder {
        for certificate in &self.0 {
            builder = builder.add_root_certificate(certificate.clone());
        }
        builder
    }
}

/// A `reqwest` client builder that also trusts the configured corporate CA bundle
/// (`NBLM_CA_BUNDLE`, then `SSL_CERT_FILE`, then `REQUESTS_CA_BUNDLE`). Use it for any
/// client created outside `NblmClient`, such as the user-OAuth token and refresh flow.
pub fn http_client_builder() -> Result<ClientBuilder> {
    Ok(TlsRoots::from_env()?.apply(reqwest::Client::builder()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const CA: &[u8] = include_bytes!("../tests/fixtures/tls/test-ca.pem");

    #[test]
    fn accepts_multiple_certificates_without_disabling_verification() {
        let roots = TlsRoots::from_pem(&[CA, CA].concat(), "test").unwrap();
        assert_eq!(roots.0.len(), 2);
        roots.apply(reqwest::Client::builder()).build().unwrap();
    }

    #[test]
    fn empty_and_malformed_bundles_fail_closed() {
        for bytes in [
            b"".as_slice(),
            b"not a certificate",
            b"-----BEGIN CERTIFICATE-----\nbad\n",
        ] {
            assert!(TlsRoots::from_pem(bytes, "test").is_err());
        }
    }

    #[test]
    fn explicit_bundle_has_precedence_and_is_not_silently_ignored() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("ca.pem");
        std::fs::write(&path, CA).unwrap();
        let roots = TlsRoots::from_lookup(|key| match key {
            "NBLM_CA_BUNDLE" => Some(path.clone().into_os_string()),
            _ => panic!("lower-priority variables should not be read"),
        })
        .unwrap();
        assert_eq!(roots.0.len(), 1);
        let error = TlsRoots::from_lookup(|_| {
            Some(
                directory
                    .path()
                    .join("missing-secret-path")
                    .into_os_string(),
            )
        })
        .err()
        .unwrap()
        .to_string();
        assert!(error.contains("NBLM_CA_BUNDLE"));
        assert!(!error.contains("missing-secret-path"));
    }

    #[test]
    fn standard_bundle_variables_and_builtin_roots_are_supported() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("ca.pem");
        std::fs::write(&path, CA).unwrap();
        for variable in ["SSL_CERT_FILE", "REQUESTS_CA_BUNDLE"] {
            let roots = TlsRoots::from_lookup(|key| {
                (key == variable).then(|| path.clone().into_os_string())
            })
            .unwrap();
            assert_eq!(roots.0.len(), 1);
        }
        assert!(TlsRoots::from_lookup(|_| None).unwrap().0.is_empty());
    }
}

#[cfg(test)]
mod builder_tests {
    use super::*;

    const CA: &[u8] = include_bytes!("../tests/fixtures/tls/test-ca.pem");

    /// Restores the CA environment variables after the test.
    struct EnvGuard(Vec<(&'static str, Option<std::ffi::OsString>)>);
    impl EnvGuard {
        fn new() -> Self {
            Self(
                ["NBLM_CA_BUNDLE", "SSL_CERT_FILE", "REQUESTS_CA_BUNDLE"]
                    .into_iter()
                    .map(|key| (key, std::env::var_os(key)))
                    .collect(),
            )
        }
    }
    impl Drop for EnvGuard {
        fn drop(&mut self) {
            for (key, value) in &self.0 {
                match value {
                    Some(value) => std::env::set_var(key, value),
                    None => std::env::remove_var(key),
                }
            }
        }
    }

    #[test]
    #[serial_test::serial]
    fn public_builder_uses_the_configured_bundle_and_fails_closed() {
        let _guard = EnvGuard::new();
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("ca.pem");
        std::fs::write(&path, CA).unwrap();
        std::env::set_var("NBLM_CA_BUNDLE", &path);
        http_client_builder().unwrap().build().unwrap();
        std::env::set_var("NBLM_CA_BUNDLE", directory.path().join("missing.pem"));
        let error = http_client_builder().err().unwrap().to_string();
        assert!(error.contains("NBLM_CA_BUNDLE"), "{error}");
    }
}
