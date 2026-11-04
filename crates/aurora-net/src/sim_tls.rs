//! Certificate checks of the simulator side: capabilities, assets, the LSL
//! bridge's requests and the release notes capability (everything reached
//! after login; the login itself keeps the platform verifier, lib.rs).
//!
//! Like the Legacy Viewer's llcorehttp (`HttpOptions::mVerifyHost = false`,
//! indra/llcorehttp/httpoptions.cpp, originally LGPL 2.1), a certificate
//! signed by Linden Lab's own authority (lindenlab.pem) is accepted whatever
//! name it carries: simulators answer under host names their certificates do
//! not hold. Unlike it, a certificate from a public authority must be valid
//! for the host that was asked for, as a browser requires: public
//! authorities hand certificates to anyone for their own domains, so a valid
//! chain alone says nothing about who answers.
//!
//! One tolerance stays, for Linden's hosts only: a Linden host whose public
//! certificate is valid for another Linden name (a wildcard one level up, a
//! certificate shared between services) is accepted, since nobody but
//! Linden Lab can get a public certificate for its domains. Other grids
//! (OpenSim) get the browser rule: a public chain and their own host name.
//!
//! The log says once per host which rule accepted it, and once per host and
//! reason why a certificate was refused (host and certificate names only,
//! never a URL path: capability paths are secrets).

use std::collections::HashSet;
use std::sync::{Arc, OnceLock};

use parking_lot::Mutex;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{CryptoProvider, WebPkiSupportedAlgorithms};
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::server::ParsedCertificate;
use rustls::{CertificateError, DigitallySignedStruct, Error, RootCertStore, SignatureScheme};

/// Linden Lab's domains: their hosts, and only theirs, may present a public
/// certificate of another Linden name (bridge.rs checks the same list).
pub(crate) const LINDEN_DOMAINS: [&str; 3] = ["secondlife.com", "secondlife.io", "lindenlab.com"];

/// The bundle shipped with the viewer: Linden Lab's authority, plus older
/// GeoTrust certificates kept with it. Only the first is Linden's own.
const LINDEN_BUNDLE: &[u8] = include_bytes!("../lindenlab.pem");

/// Text of the subject of Linden Lab's own authority in the bundle.
const LINDEN_AUTHORITY: &[u8] = b"Linden Lab Certificate Authority";

/// Distinct hosts and reasons kept for the log; past it the set restarts
/// (a long session crosses many simulators).
const SEEN_MAX: usize = 4096;

/// `host` is one of Linden Lab's domains or below one.
pub(crate) fn is_linden_host(host: &str) -> bool {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    LINDEN_DOMAINS
        .iter()
        .any(|domain| host == *domain || host.strip_suffix(domain).is_some_and(|rest| rest.ends_with('.')))
}

/// A reqwest client configuration with the simulator rules, for
/// `ClientBuilder::tls_backend_preconfigured`. HTTP/1.1 only, as the
/// clients it replaces (reqwest is built without HTTP/2 here).
pub(crate) fn client_config() -> Result<rustls::ClientConfig, Error> {
    let provider = provider();
    let mut config = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()?
        .dangerous()
        .with_custom_certificate_verifier(verifier())
        .with_no_client_auth();
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    Ok(config)
}

/// The process's crypto provider if one was installed, else aws-lc-rs, the
/// one reqwest itself falls back on.
fn provider() -> Arc<CryptoProvider> {
    CryptoProvider::get_default()
        .cloned()
        .unwrap_or_else(|| Arc::new(rustls::crypto::aws_lc_rs::default_provider()))
}

/// One verifier for every simulator client, so that the log names each
/// host once per session whichever client reached it.
fn verifier() -> Arc<SimulatorVerifier> {
    static VERIFIER: OnceLock<Arc<SimulatorVerifier>> = OnceLock::new();
    VERIFIER
        .get_or_init(|| {
            let (linden, public) = bundled_roots();
            Arc::new(SimulatorVerifier::new(linden, public, provider().signature_verification_algorithms))
        })
        .clone()
}

/// Linden Lab's authority alone, and the public roots with the rest of the
/// bundle (trusted as before, but under the public rule now).
fn bundled_roots() -> (RootCertStore, RootCertStore) {
    let mut linden = RootCertStore::empty();
    let mut public = RootCertStore::empty();
    for cert in webpki_root_certs::TLS_SERVER_ROOT_CERTS {
        if public.add(cert.clone()).is_err() {
            log::debug!("simulator TLS: a public root was not usable");
        }
    }
    for cert in CertificateDer::pem_slice_iter(LINDEN_BUNDLE).flatten() {
        let is_linden = webpki::anchor_from_trusted_cert(&cert).is_ok_and(|anchor| contains(anchor.subject.as_ref(), LINDEN_AUTHORITY));
        let store = if is_linden { &mut linden } else { &mut public };
        if store.add(cert).is_err() {
            log::warn!("simulator TLS: a certificate of lindenlab.pem was not usable");
        }
    }
    if linden.is_empty() {
        log::warn!("simulator TLS: Linden Lab's authority missing from lindenlab.pem");
    }
    (linden, public)
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

/// Which rule accepted a certificate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Accepted {
    /// Signed by Linden Lab's authority (any name, as the Legacy Viewer).
    LindenAuthority,
    /// A public chain, valid for the host asked for.
    PublicName,
    /// A public chain for another Linden name, on a Linden host.
    LindenName,
}

impl Accepted {
    fn label(self) -> &'static str {
        match self {
            Accepted::LindenAuthority => "Linden Lab authority",
            Accepted::PublicName => "public authority, host name checked",
            Accepted::LindenName => "public authority, certificate of another Linden name",
        }
    }
}

#[derive(Debug)]
pub(crate) struct SimulatorVerifier {
    linden: RootCertStore,
    public: RootCertStore,
    algorithms: WebPkiSupportedAlgorithms,
    /// (host, rule or reason) already logged
    seen: Mutex<HashSet<(String, String)>>,
}

impl SimulatorVerifier {
    pub(crate) fn new(linden: RootCertStore, public: RootCertStore, algorithms: WebPkiSupportedAlgorithms) -> Self {
        SimulatorVerifier {
            linden,
            public,
            algorithms,
            seen: Mutex::new(HashSet::new()),
        }
    }

    /// The rule that accepts `end_entity` for `server_name`, or the error.
    pub(crate) fn decide(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        now: UnixTime,
    ) -> Result<Accepted, Error> {
        let cert = ParsedCertificate::try_from(end_entity)?;
        let signed_by = |roots: &RootCertStore| {
            rustls::client::verify_server_cert_signed_by_trust_anchor(&cert, roots, intermediates, now, self.algorithms.all)
        };
        if !self.linden.is_empty() && signed_by(&self.linden).is_ok() {
            return Ok(Accepted::LindenAuthority);
        }
        signed_by(&self.public)?;
        let name_error = match rustls::client::verify_server_name(&cert, server_name) {
            Ok(()) => return Ok(Accepted::PublicName),
            Err(e) => e,
        };
        let linden_host = matches!(server_name, ServerName::DnsName(host) if is_linden_host(host.as_ref()));
        if linden_host
            && cert_names(end_entity)
                .iter()
                .any(|name| is_linden_host(name.trim_start_matches("*.")))
        {
            return Ok(Accepted::LindenName);
        }
        Err(name_error)
    }

    /// Log `what` for `host` the first time only.
    fn log_once(&self, host: &str, what: &str, refused: bool, names: impl FnOnce() -> String) {
        let mut seen = self.seen.lock();
        if seen.len() >= SEEN_MAX {
            seen.clear();
        }
        if seen.insert((host.to_owned(), what.to_owned())) {
            if refused {
                log::warn!("simulator TLS: {host} refused ({what}); certificate names: {}", names());
            } else {
                log::info!("simulator TLS: {host} accepted ({what})");
            }
        }
    }
}

/// The DNS names a certificate holds, for the log and the Linden-name rule
/// (never to check a host name: verify_server_name does that).
fn cert_names(end_entity: &CertificateDer<'_>) -> Vec<String> {
    webpki::EndEntityCert::try_from(end_entity)
        .map(|cert| cert.valid_dns_names().map(str::to_owned).collect())
        .unwrap_or_default()
}

fn host_of(server_name: &ServerName<'_>) -> String {
    match server_name {
        ServerName::DnsName(host) => host.as_ref().to_owned(),
        ServerName::IpAddress(ip) => format!("{:?}", std::net::IpAddr::from(*ip)),
        _ => "(unknown host)".to_owned(),
    }
}

impl ServerCertVerifier for SimulatorVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, Error> {
        let host = host_of(server_name);
        match self.decide(end_entity, intermediates, server_name, now) {
            Ok(rule) => {
                self.log_once(&host, rule.label(), false, String::new);
                Ok(ServerCertVerified::assertion())
            }
            Err(e) => {
                let reason = match &e {
                    Error::InvalidCertificate(CertificateError::NotValidForName)
                    | Error::InvalidCertificate(CertificateError::NotValidForNameContext { .. }) => {
                        "certificate not valid for this host".to_owned()
                    }
                    other => other.to_string(),
                };
                self.log_once(&host, &reason, true, || {
                    let names = cert_names(end_entity);
                    if names.is_empty() { "none".to_owned() } else { names.join(", ") }
                });
                Err(e)
            }
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        rustls::crypto::verify_tls12_signature(message, cert, dss, &self.algorithms)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        rustls::crypto::verify_tls13_signature(message, cert, dss, &self.algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.algorithms.supported_schemes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rcgen::{BasicConstraints, CertificateParams, CertifiedIssuer, DnType, IsCa, KeyPair};

    struct Ca {
        issuer: CertifiedIssuer<'static, KeyPair>,
    }

    impl Ca {
        fn new(name: &str) -> Ca {
            let mut params = CertificateParams::new(Vec::<String>::new()).unwrap();
            params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
            params.distinguished_name.push(DnType::CommonName, name);
            Ca {
                issuer: CertifiedIssuer::self_signed(params, KeyPair::generate().unwrap()).unwrap(),
            }
        }

        fn store(&self) -> RootCertStore {
            let mut store = RootCertStore::empty();
            store.add(self.issuer.der().clone()).unwrap();
            store
        }

        fn leaf(&self, names: &[&str]) -> CertificateDer<'static> {
            let params = CertificateParams::new(names.iter().map(|n| n.to_string()).collect::<Vec<_>>()).unwrap();
            params.signed_by(&KeyPair::generate().unwrap(), &self.issuer).unwrap().der().clone()
        }
    }

    struct Setup {
        linden: Ca,
        public: Ca,
        verifier: SimulatorVerifier,
    }

    fn setup() -> Setup {
        let linden = Ca::new("Test Linden authority");
        let public = Ca::new("Test public authority");
        let verifier = SimulatorVerifier::new(linden.store(), public.store(), provider().signature_verification_algorithms);
        Setup { linden, public, verifier }
    }

    fn decide(s: &Setup, leaf: &CertificateDer<'_>, host: &str) -> Result<Accepted, Error> {
        let name = ServerName::try_from(host.to_owned()).unwrap();
        s.verifier.decide(leaf, &[], &name, UnixTime::now())
    }

    #[test]
    fn linden_authority_needs_no_matching_name() {
        let s = setup();
        let leaf = s.linden.leaf(&["simhost.internal"]);
        assert_eq!(decide(&s, &leaf, "simhost-0a1b.agni.secondlife.io"), Ok(Accepted::LindenAuthority));
        // as the Legacy Viewer: Linden's authority is trusted for any host
        assert_eq!(decide(&s, &leaf, "grid.example"), Ok(Accepted::LindenAuthority));
    }

    #[test]
    fn a_public_certificate_of_another_domain_is_refused() {
        let s = setup();
        let leaf = s.public.leaf(&["attacker.example"]);
        assert!(decide(&s, &leaf, "simhost-0a1b.agni.secondlife.io").is_err());
        assert!(decide(&s, &leaf, "asset-cdn.glb.agni.lindenlab.com").is_err());
        // a name that only ends like a Linden domain is not one
        let leaf = s.public.leaf(&["secondlife.io.attacker.example", "attacker-secondlife.io"]);
        assert!(decide(&s, &leaf, "simhost-0a1b.agni.secondlife.io").is_err());
    }

    #[test]
    fn a_public_certificate_of_the_host_is_accepted() {
        let s = setup();
        let leaf = s.public.leaf(&["asset-cdn.glb.agni.lindenlab.com"]);
        assert_eq!(decide(&s, &leaf, "asset-cdn.glb.agni.lindenlab.com"), Ok(Accepted::PublicName));
        let leaf = s.public.leaf(&["*.agni.secondlife.io"]);
        assert_eq!(decide(&s, &leaf, "simhost-0a1b.agni.secondlife.io"), Ok(Accepted::PublicName));
    }

    #[test]
    fn a_linden_host_may_present_another_linden_name() {
        let s = setup();
        // a wildcard one level too high: not valid for the host, but Linden's
        let leaf = s.public.leaf(&["*.secondlife.io"]);
        assert_eq!(decide(&s, &leaf, "simhost-0a1b.agni.secondlife.io"), Ok(Accepted::LindenName));
        let leaf = s.public.leaf(&["map.secondlife.com"]);
        assert_eq!(decide(&s, &leaf, "slm-assets.secondlife.com"), Ok(Accepted::LindenName));
        // never for a host outside Linden's domains
        assert!(decide(&s, &leaf, "grid.example").is_err());
    }

    #[test]
    fn other_grids_get_the_browser_rule() {
        let s = setup();
        let leaf = s.public.leaf(&["sim.grid.example"]);
        assert_eq!(decide(&s, &leaf, "sim.grid.example"), Ok(Accepted::PublicName));
        assert!(decide(&s, &leaf, "other.grid.example").is_err());
    }

    #[test]
    fn an_unknown_authority_is_refused() {
        let s = setup();
        let stranger = Ca::new("Unknown authority");
        let leaf = stranger.leaf(&["simhost-0a1b.agni.secondlife.io"]);
        assert!(decide(&s, &leaf, "simhost-0a1b.agni.secondlife.io").is_err());
    }

    #[test]
    fn the_trait_path_accepts_and_refuses_like_decide() {
        let s = setup();
        let name = ServerName::try_from("simhost-0a1b.agni.secondlife.io").unwrap();
        let good = s.linden.leaf(&["x"]);
        assert!(s.verifier.verify_server_cert(&good, &[], &name, &[], UnixTime::now()).is_ok());
        let bad = s.public.leaf(&["attacker.example"]);
        assert!(s.verifier.verify_server_cert(&bad, &[], &name, &[], UnixTime::now()).is_err());
    }

    #[test]
    fn the_bundle_holds_linden_authority_alone_on_the_linden_side() {
        let (linden, public) = bundled_roots();
        assert_eq!(linden.len(), 1);
        assert!(public.len() > 100, "public roots: {}", public.len());
    }

    #[test]
    fn linden_hosts() {
        for host in [
            "secondlife.com",
            "simhost-0a1b.agni.secondlife.io",
            "ASSET-CDN.glb.agni.lindenlab.com",
            "map.secondlife.com.",
        ] {
            assert!(is_linden_host(host), "{host}");
        }
        for host in [
            "secondlife.io.attacker.example",
            "attacker-secondlife.io",
            "notlindenlab.com",
            "example.com",
            "",
        ] {
            assert!(!is_linden_host(host), "{host}");
        }
    }

    #[test]
    fn reqwest_takes_the_configuration() {
        let config = client_config().unwrap();
        assert_eq!(config.alpn_protocols, vec![b"http/1.1".to_vec()]);
        // an unknown preconfigured backend would fail here
        reqwest::Client::builder().tls_backend_preconfigured(config).build().unwrap();
    }
}
