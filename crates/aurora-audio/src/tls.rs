//! TLS of internet radio: a station whose certificate fails the platform
//! check still plays, and the viewer is told why.
//!
//! The Legacy Viewer hands the stream URL to FMOD::System::createStream
//! (LLStreamingAudio_FMODSTUDIO::startInternetStream,
//! indra/llaudio/llstreamingaudio_fmodstudio.cpp) and makes no certificate
//! check of its own. Aurora checks the certificate with the platform verifier
//! (#1628) and, instead of refusing a station whose certificate is expired,
//! self-signed or for another name, plays it and reports the problem
//! (`CertificateIssue`), so the resident knows the connection is not secure.
//! A plain `http://` station is no safer and plays without a word, and a radio
//! connection carries no credential: the audio is all it fetches. Only the
//! certificate check is relaxed: the handshake signatures are still verified
//! (the server holds the key of the certificate it shows), and every other TLS
//! failure stays an error.

use std::sync::{Arc, OnceLock};

use parking_lot::Mutex;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{CertificateError, DigitallySignedStruct, SignatureScheme};

/// Why a station's certificate failed the check, for the viewer's warning.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CertificateIssue {
    /// Past its end of validity; `not_after` in Unix seconds when readable.
    Expired { not_after: Option<i64> },
    /// Not valid yet (or the computer's clock is behind).
    NotYetValid,
    /// Self-signed, or signed by an authority the system does not trust.
    Untrusted,
    /// Made for another server name.
    WrongName,
    /// Revoked by its authority.
    Revoked,
    /// Any other certificate failure.
    Other,
}

/// The issue of one connection, written by the verifier during the handshake.
pub(crate) type IssueSlot = Arc<Mutex<Option<CertificateIssue>>>;

/// The platform verifier, built once: building it reads the system's roots.
fn platform_verifier() -> Result<Arc<rustls_platform_verifier::Verifier>, String> {
    static VERIFIER: OnceLock<Result<Arc<rustls_platform_verifier::Verifier>, String>> = OnceLock::new();
    VERIFIER
        .get_or_init(|| {
            // Some stations omit their intermediate certificate (#1628): the
            // Windows/macOS verifier can build the missing chain.
            rustls_platform_verifier::Verifier::new(provider())
                .map(Arc::new)
                .map_err(|e| e.to_string())
        })
        .clone()
}

/// Explicit provider: the workspace may enable several rustls crypto
/// backends, which makes the implicit default ambiguous.
fn provider() -> Arc<rustls::crypto::CryptoProvider> {
    static PROVIDER: OnceLock<Arc<rustls::crypto::CryptoProvider>> = OnceLock::new();
    PROVIDER
        .get_or_init(|| Arc::new(rustls::crypto::aws_lc_rs::default_provider()))
        .clone()
}

/// A client configuration of its own for one connection, with the slot its
/// verifier fills. Built per connection (once per connect or reconnect, a
/// second apart at least), so each connection reports its own certificate.
pub(crate) fn client_config() -> Result<(Arc<rustls::ClientConfig>, IssueSlot), String> {
    let issue = IssueSlot::default();
    let verifier = Lenient {
        inner: platform_verifier()?,
        issue: issue.clone(),
    };
    let config = rustls::ClientConfig::builder_with_provider(provider())
        .with_safe_default_protocol_versions()
        .map_err(|e| e.to_string())?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(verifier))
        .with_no_client_auth();
    Ok((Arc::new(config), issue))
}

/// The platform verifier, a certificate failure turned into a reported issue.
#[derive(Debug)]
struct Lenient {
    inner: Arc<rustls_platform_verifier::Verifier>,
    issue: IssueSlot,
}

impl ServerCertVerifier for Lenient {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        match self
            .inner
            .verify_server_cert(end_entity, intermediates, server_name, ocsp_response, now)
        {
            Err(rustls::Error::InvalidCertificate(error)) => {
                let issue = classify(&error, end_entity);
                log::warn!("music stream: {server_name:?} certificate refused by the system ({error}), playing anyway: {issue:?}");
                *self.issue.lock() = Some(issue);
                Ok(ServerCertVerified::assertion())
            }
            other => other,
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.inner.verify_tls12_signature(message, cert, dss)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.inner.verify_tls13_signature(message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.inner.supported_verify_schemes()
    }
}

/// The category of a certificate failure; an expiry carries its date, read
/// from the certificate when the verifier does not give it (Windows does not).
pub(crate) fn classify(error: &CertificateError, end_entity: &CertificateDer<'_>) -> CertificateIssue {
    match error {
        CertificateError::ExpiredContext { not_after, .. } => CertificateIssue::Expired {
            not_after: i64::try_from(not_after.as_secs()).ok(),
        },
        CertificateError::Expired => CertificateIssue::Expired {
            not_after: not_after(end_entity),
        },
        CertificateError::NotValidYet | CertificateError::NotValidYetContext { .. } => CertificateIssue::NotYetValid,
        CertificateError::UnknownIssuer | CertificateError::BadSignature => CertificateIssue::Untrusted,
        CertificateError::NotValidForName | CertificateError::NotValidForNameContext { .. } => CertificateIssue::WrongName,
        CertificateError::Revoked => CertificateIssue::Revoked,
        _ => CertificateIssue::Other,
    }
}

/// The end of validity of a certificate, in Unix seconds.
fn not_after(cert: &CertificateDer<'_>) -> Option<i64> {
    let (_, parsed) = x509_parser::parse_x509_certificate(cert.as_ref()).ok()?;
    Some(parsed.validity().not_after.timestamp())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn certificate(not_after: (i32, u8, u8)) -> CertificateDer<'static> {
        let mut params = rcgen::CertificateParams::new(vec!["radio.example".to_string()]).unwrap();
        params.not_before = rcgen::date_time_ymd(2020, 1, 1);
        params.not_after = rcgen::date_time_ymd(not_after.0, not_after.1, not_after.2);
        let key = rcgen::KeyPair::generate().unwrap();
        params.self_signed(&key).unwrap().der().clone()
    }

    #[test]
    fn an_expired_certificate_reports_its_end_of_validity() {
        let cert = certificate((2026, 11, 18));
        // 2026-11-18 00:00:00 UTC
        let end = Some(1_794_960_000);
        assert_eq!(
            classify(&CertificateError::Expired, &cert),
            CertificateIssue::Expired { not_after: end }
        );
        let context = CertificateError::ExpiredContext {
            time: UnixTime::since_unix_epoch(std::time::Duration::from_secs(1_795_000_000)),
            not_after: UnixTime::since_unix_epoch(std::time::Duration::from_secs(1_794_960_000)),
        };
        assert_eq!(classify(&context, &cert), CertificateIssue::Expired { not_after: end });
        let unreadable = CertificateDer::from(vec![1, 2, 3]);
        assert_eq!(
            classify(&CertificateError::Expired, &unreadable),
            CertificateIssue::Expired { not_after: None }
        );
    }

    #[test]
    fn each_certificate_failure_has_its_category() {
        let cert = certificate((2030, 1, 1));
        let cases = [
            (CertificateError::UnknownIssuer, CertificateIssue::Untrusted),
            (CertificateError::BadSignature, CertificateIssue::Untrusted),
            (CertificateError::NotValidForName, CertificateIssue::WrongName),
            (CertificateError::NotValidYet, CertificateIssue::NotYetValid),
            (CertificateError::Revoked, CertificateIssue::Revoked),
            (CertificateError::BadEncoding, CertificateIssue::Other),
        ];
        for (error, issue) in cases {
            assert_eq!(classify(&error, &cert), issue, "{error:?}");
        }
    }
}
