//! A bounded GitHub release list, filtered by SemVer rather than GitHub's
//! creation date (the assets-only release is not a viewer): stable versions,
//! plus the `-beta.…` pre-releases for the Beta channel.

use crate::signature;
use anyhow::{Context, Result, ensure};
use reqwest::{Client, Url};
use semver::Version;
use serde::Deserialize;
use std::time::Duration;

pub const RELEASES: &str = "https://api.github.com/repos/Aurora-Viewer/Aurora-Viewer/releases?per_page=100";
pub const RELEASE_PAGE: &str = "https://github.com/Aurora-Viewer/Aurora-Viewer/releases";
const DOWNLOADS: &str = "https://github.com/Aurora-Viewer/Aurora-Viewer/releases/download";
pub const POLICY: &str = "aurora-update-policy.json";
pub const MAX_ARCHIVE: u64 = 2 * 1024 * 1024 * 1024;
/// How long an installation below a new minimum may still sign in, when
/// the policy does not say (#3014): a show or a DJ set under way ends
/// before the update is needed.
pub const DEFAULT_GRACE: Duration = Duration::from_secs(12 * 3600);
/// The longest grace a policy may give.
const MAX_GRACE_HOURS: u64 = 72;

/// Which releases a viewer follows. Beta also takes the stable versions: a
/// stable release ranks above the betas of its own number (SemVer), so a
/// beta tester moves on to it once it is out.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Channel {
    #[default]
    Stable,
    Beta,
}

/// The pre-release label of the Beta channel (`v0.7.7-beta.20261025.1`,
/// built by the Nightly workflow).
pub const BETA: &str = "beta";

impl Channel {
    /// Whether a release version belongs to this channel. Build metadata
    /// never does: it marks a development build, not a release.
    pub fn accepts(self, v: &Version) -> bool {
        v.build.is_empty() && (v.pre.is_empty() || (self == Channel::Beta && is_beta(v)))
    }
}

/// A Beta channel pre-release: `-beta`, `-beta.<…>`.
pub fn is_beta(v: &Version) -> bool {
    v.pre.as_str().split('.').next() == Some(BETA)
}

#[derive(Clone)]
pub struct Source {
    url: String,
    loopback: bool,
    /// The release list last read and its ETag, shared by the clones: the
    /// next check asks GitHub whether it changed (`If-None-Match`). A "304
    /// Not Modified" answer is a few bytes and does not count against the 60
    /// requests an hour GitHub allows an address without an account, so a
    /// viewer can check every few minutes, even with others behind the same
    /// address (a household, a mobile network, a VPN).
    list: std::sync::Arc<std::sync::Mutex<Option<(String, Vec<u8>)>>>,
    /// The keys a release's files must be signed with ([`signature`]): the
    /// release keys for GitHub, none for a loopback feed unless a test names
    /// its own ([`Source::trusting`]).
    keys: std::sync::Arc<[signature::Key]>,
    /// The release whose signatures last checked out, with what they cover:
    /// a periodic check does not download them again while it stays the
    /// newest. Safe to keep: a file replaced since still has to match it.
    signed: std::sync::Arc<std::sync::Mutex<Option<(String, Signed)>>>,
}

/// What a release's signatures cover.
#[derive(Clone)]
struct Signed {
    /// The SHA-256 of this system's package (or disk image on macOS).
    sha256: [u8; 32],
    /// The update policy, when the release has one.
    policy: Option<Vec<u8>>,
}

/// How many releases newer than the installed one a check looks at, newest
/// first, for one whose signatures check out: a release made by hand (or a
/// Linux archive not uploaded yet) never hides the genuine one below it.
const CANDIDATES: usize = 3;

impl Default for Source {
    fn default() -> Self {
        Self {
            url: RELEASES.into(),
            loopback: false,
            list: Default::default(),
            keys: signature::release_keys().into(),
            signed: Default::default(),
        }
    }
}

impl Source {
    /// Test feeds are explicit loopback HTTP only; no arbitrary remote override.
    pub fn local(url: &str) -> Result<Self> {
        let url = Url::parse(url)?;
        ensure!(
            url.scheme() == "http" && matches!(url.host_str(), Some("127.0.0.1" | "[::1]" | "localhost")),
            "test feed must use loopback HTTP"
        );
        ensure!(url.username().is_empty() && url.password().is_none(), "credentials in feed URL");
        Ok(Self {
            url: url.into(),
            loopback: true,
            list: Default::default(),
            keys: std::sync::Arc::new([]),
            signed: Default::default(),
        })
    }

    /// A test feed whose releases are signed with `key` (the test's own,
    /// `lib.rs::test_environment`). GitHub's feed trusts the release keys
    /// only.
    pub fn trusting(mut self, key: signature::Key) -> Result<Self> {
        ensure!(self.loopback, "a test key for a public feed");
        self.keys = std::sync::Arc::new([key]);
        Ok(self)
    }

    pub fn validate_url(&self, value: &str) -> Result<()> {
        let url = Url::parse(value)?;
        ensure!(url.username().is_empty() && url.password().is_none(), "credentials in update URL");
        if self.loopback {
            let base = Url::parse(&self.url)?;
            ensure!(url.origin() == base.origin(), "test asset outside loopback feed");
        } else {
            ensure!(
                url.scheme() == "https"
                    && matches!(
                        url.host_str(),
                        Some("api.github.com" | "github.com" | "release-assets.githubusercontent.com" | "objects.githubusercontent.com")
                    ),
                "untrusted update URL"
            );
        }
        Ok(())
    }

    /// Address of a file of the release `tag`, known without asking the feed
    /// (a test feed serves it from its own origin).
    pub fn asset_url(&self, tag: &str, name: &str) -> Result<String> {
        let path = format!("{tag}/{name}");
        if self.loopback {
            Ok(Url::parse(&self.url)?.join(&format!("/download/{path}"))?.into())
        } else {
            Ok(format!("{DOWNLOADS}/{path}"))
        }
    }

    pub fn client(&self) -> Result<Client> {
        let source = self.clone();
        let loopback = self.loopback;
        // An explicit local feed never leaves loopback: the rule never
        // proxies it.
        let client = aurora_proxy::http_client(|builder| {
            builder
                .user_agent(aurora_version::user_agent())
                .https_only(!loopback)
                .connect_timeout(Duration::from_secs(8))
                .read_timeout(Duration::from_secs(20))
                .redirect(reqwest::redirect::Policy::custom(move |attempt| {
                    if attempt.previous().len() >= 5 {
                        attempt.error("too many redirects")
                    } else if source.validate_url(attempt.url().as_str()).is_ok() {
                        attempt.follow()
                    } else {
                        attempt.error("untrusted redirect")
                    }
                }))
        })?;
        Ok(client)
    }

    pub async fn bytes(&self, client: &Client, url: &str, limit: usize) -> Result<Vec<u8>> {
        self.validate_url(url)?;
        let mut response = client
            .get(url)
            .header("Accept", "application/vnd.github+json")
            .send()
            .await?
            .error_for_status()?;
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await? {
            ensure!(bytes.len().saturating_add(chunk.len()) <= limit, "update metadata too large");
            bytes.extend_from_slice(&chunk);
        }
        Ok(bytes)
    }

    /// The release list: the one kept when GitHub says it has not changed
    /// since (`list`), else the new one, kept with its ETag.
    async fn releases(&self, client: &Client) -> Result<Vec<u8>> {
        const LIMIT: usize = 4 * 1024 * 1024;
        self.validate_url(&self.url)?;
        let kept = self.list.lock().map_err(|_| anyhow::anyhow!("release list lock"))?.clone();
        let mut request = client.get(&self.url).header("Accept", "application/vnd.github+json");
        if let Some((etag, _)) = &kept {
            request = request.header(reqwest::header::IF_NONE_MATCH, etag);
        }
        let mut response = request.send().await?;
        if response.status() == reqwest::StatusCode::NOT_MODIFIED
            && let Some((_, bytes)) = kept
        {
            return Ok(bytes);
        }
        response = response.error_for_status()?;
        let etag = response
            .headers()
            .get(reqwest::header::ETAG)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await? {
            ensure!(bytes.len().saturating_add(chunk.len()) <= LIMIT, "update metadata too large");
            bytes.extend_from_slice(&chunk);
        }
        if let Ok(mut list) = self.list.lock() {
            *list = etag.map(|e| (e, bytes.clone()));
        }
        Ok(bytes)
    }

    /// The newest release of `channel` above `current` whose signatures
    /// check out ([`signature`]). A newer one that is not signed, or whose
    /// signature does not match, is skipped and logged: anyone with write
    /// access can make a release by hand.
    pub async fn check(&self, current: &str, channel: Channel) -> Result<Option<Release>> {
        let client = self.client()?;
        let releases: Vec<GithubRelease> = serde_json::from_slice(&self.releases(&client).await?)?;
        let mut newer: Vec<(Version, GithubRelease)> = releases
            .into_iter()
            .filter_map(|r| {
                let v = version(&r.tag_name)?;
                // GitHub's pre-release flag must agree with the version: a
                // stable number marked pre-release (or the reverse) is skipped.
                (r.tag_name.starts_with('v') && !r.draft && r.prerelease == !v.pre.is_empty() && channel.accepts(&v)).then_some((v, r))
            })
            .filter(|(v, _)| super::is_newer(&v.to_string(), current))
            .collect();
        newer.sort_by(|a, b| b.0.cmp_precedence(&a.0));
        for (v, r) in newer.into_iter().take(CANDIDATES) {
            if let Some(signed) = self.signed(&client, &r).await? {
                return self.release(v, r, signed, current).map(Some);
            }
        }
        Ok(None)
    }

    /// What the signatures of `r` cover, or `None` when a file is missing or
    /// a signature does not match (logged). A network error is an error:
    /// the check is tried again, rather than an older release offered.
    async fn signed(&self, client: &Client, r: &GithubRelease) -> Result<Option<Signed>> {
        if let Ok(kept) = self.signed.lock()
            && let Some((tag, signed)) = kept.as_ref()
            && *tag == r.tag_name
        {
            return Ok(Some(signed.clone()));
        }
        let Some(name) = signed_name(&r.tag_name) else {
            return Ok(None);
        };
        let asset = |name: &str| r.assets.iter().find(|a| a.name == name);
        let refuse = |why: &str| {
            log::warn!("update {}: not offered, {why}", r.tag_name);
            Ok(None)
        };
        let sidecar = format!("{name}.sha256");
        let (Some(file), Some(sig)) = (asset(&sidecar), asset(&format!("{sidecar}{}", signature::SUFFIX))) else {
            return refuse(&format!("{sidecar} or its signature is missing"));
        };
        let file = self.bytes(client, &file.browser_download_url, 1024).await?;
        let sig = self.bytes(client, &sig.browser_download_url, 1024).await?;
        if let Err(e) = signature::verify(&self.keys, &file, &sig) {
            return refuse(&format!("{sidecar}: {e}"));
        }
        let sha256 = match crate::package::parse_checksum(&file, &name) {
            Ok(sha256) => sha256,
            Err(e) => return refuse(&format!("{sidecar}: {e}")),
        };
        let policy = match asset(POLICY) {
            None => None,
            Some(policy) => {
                let Some(sig) = asset(&format!("{POLICY}{}", signature::SUFFIX)) else {
                    return refuse("the update policy is not signed");
                };
                let policy = self.bytes(client, &policy.browser_download_url, 16 * 1024).await?;
                let sig = self.bytes(client, &sig.browser_download_url, 1024).await?;
                if let Err(e) = signature::verify(&self.keys, &policy, &sig) {
                    return refuse(&format!("{POLICY}: {e}"));
                }
                Some(policy)
            }
        };
        let signed = Signed { sha256, policy };
        if let Ok(mut kept) = self.signed.lock() {
            *kept = Some((r.tag_name.clone(), signed.clone()));
        }
        Ok(Some(signed))
    }

    /// The release `r`, once its signatures checked out.
    fn release(&self, v: Version, r: GithubRelease, signed: Signed, current: &str) -> Result<Release> {
        self.validate_url(&r.html_url)?;
        let archive = package_name(&r.tag_name)
            .and_then(|name| r.assets.iter().find(|a| a.name == name && a.size > 0 && a.size <= MAX_ARCHIVE))
            .cloned();
        if let Some(a) = &archive {
            self.validate_url(&a.browser_download_url)?;
        }
        let mut required = false;
        let mut minimum = None;
        let mut grace = DEFAULT_GRACE;
        let mut maintenance = super::Maintenance::default();
        if let Some(policy) = &signed.policy {
            let policy: Policy = serde_json::from_slice(policy)?;
            ensure!(policy.schema == 1 && policy.version == v.to_string(), "incompatible update policy");
            maintenance.clear_cache = threshold_applies(policy.cache_invalidation_before.as_deref(), current, &v)?;
            maintenance.reset_settings = threshold_applies(policy.settings_reset_before.as_deref(), current, &v)?;
            if let Some(min) = policy.minimum_supported_version {
                let min = version(&min).context("invalid minimum supported version")?;
                ensure!(
                    min.pre.is_empty() && min.build.is_empty() && !min.cmp_precedence(&v).is_gt(),
                    "invalid update floor"
                );
                required = version(current).is_some_and(|c| c.cmp_precedence(&min).is_lt());
                minimum = Some(min.to_string());
            }
            if let Some(hours) = policy.grace_hours {
                ensure!(hours <= MAX_GRACE_HOURS, "invalid grace period");
                grace = Duration::from_secs(hours * 3600);
            }
        }
        Ok(Release {
            tag: r.tag_name,
            version: v.to_string(),
            notes: r.body.unwrap_or_default(),
            url: r.html_url,
            sha256: archive.is_some().then_some(signed.sha256),
            archive,
            required,
            minimum,
            grace,
            maintenance,
        })
    }
}

pub fn version(value: &str) -> Option<Version> {
    Version::parse(value.strip_prefix('v').unwrap_or(value)).ok()
}

/// The file whose signed `.sha256` vouches for a release on this system:
/// its package, or the disk image on macOS (installed by hand, but the
/// viewer still points only to a genuine release).
fn signed_name(tag: &str) -> Option<String> {
    if cfg!(target_os = "macos") {
        Some(format!("Aurora-Viewer-{tag}-macos-arm64.dmg"))
    } else {
        package_name(tag)
    }
}

/// The package this system installs, as the release names it (`package.ps1`):
/// the Windows ZIP, or the Linux tar.gz. macOS installs its application by
/// hand, so a release offers it no package to install.
fn package_name(tag: &str) -> Option<String> {
    if cfg!(windows) {
        Some(format!("Aurora-Viewer-{tag}-windows-x64.zip"))
    } else if cfg!(target_os = "linux") {
        Some(format!("Aurora-Viewer-{tag}-linux-x64.tar.gz"))
    } else {
        None
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct Asset {
    pub name: String,
    pub size: u64,
    pub browser_download_url: String,
}

#[derive(Deserialize)]
struct GithubRelease {
    tag_name: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    body: Option<String>,
    html_url: String,
    #[serde(default)]
    assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct Policy {
    schema: u32,
    version: String,
    minimum_supported_version: Option<String>,
    /// Hours an installation below the minimum may still sign in, counted
    /// from the first time it sees this minimum; absent: `DEFAULT_GRACE`,
    /// 0: blocked at once (a release that cannot wait).
    #[serde(default)]
    grace_hours: Option<u64>,
    cache_invalidation_before: Option<String>,
    settings_reset_before: Option<String>,
}

fn threshold_applies(threshold: Option<&str>, current: &str, latest: &Version) -> Result<bool> {
    let Some(threshold) = threshold else { return Ok(false) };
    let cutoff = version(threshold).context("invalid maintenance threshold")?;
    ensure!(
        cutoff.pre.is_empty() && cutoff.build.is_empty() && !cutoff.cmp_precedence(latest).is_gt(),
        "invalid maintenance threshold"
    );
    Ok(version(current).is_some_and(|installed| installed.cmp_precedence(&cutoff).is_lt()))
}

#[derive(Clone, Debug)]
pub struct Release {
    pub tag: String,
    pub version: String,
    pub notes: String,
    pub url: String,
    pub archive: Option<Asset>,
    /// The SHA-256 of `archive`, from the release's signed `.sha256` file
    /// (`Source::check`): the download must match it.
    pub sha256: Option<[u8; 32]>,
    /// Below the minimum supported version: the update is needed to sign
    /// in, once `grace` is over.
    pub required: bool,
    /// That minimum, which the grace period is counted for.
    pub minimum: Option<String>,
    pub grace: Duration,
    pub maintenance: super::Maintenance,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn maintenance_thresholds_survive_skipped_releases_and_do_not_repeat() {
        let latest = Version::parse("0.5.0").expect("fixture");
        assert!(threshold_applies(Some("0.3.0"), "0.2.0+8.gabcdef", &latest).expect("policy"));
        assert!(!threshold_applies(Some("0.3.0"), "0.3.0", &latest).expect("policy"));
        assert!(!threshold_applies(None, "0.2.0", &latest).expect("old policy"));
        for invalid in ["0.6.0", "garbage", "0.3.0-rc.1", "0.3.0+build"] {
            assert!(threshold_applies(Some(invalid), "0.2.0", &latest).is_err());
        }
    }
    #[test]
    fn strict_semver_and_provenance() {
        assert!(crate::is_newer("v0.3.0", "0.2.0+8.gabc123"));
        assert!(!crate::is_newer("v0.2.0", "0.2.0+8.gabc123"));
        assert!(crate::is_newer("v1.0.0", "1.0.0-rc.1"));
        for tag in ["assets-1", "1.0", "v01.2.3", "1.2.3garbage"] {
            assert!(!crate::is_newer(tag, "0.1.0"));
        }
    }
    #[test]
    fn channels_take_stable_versions_and_beta_only_its_own_prereleases() {
        let v = |s: &str| version(s).expect("fixture");
        for (tag, stable, beta) in [
            ("v0.7.6", true, true),
            ("v0.7.7-beta.20261025.1", false, true),
            ("v0.7.7-beta", false, true),
            ("v0.7.7-rc.1", false, false),
            ("v0.7.7-betafoo.1", false, false),
            ("v0.7.6+3.gabcdef", false, false),
            ("v0.7.7-beta.1+3.gabcdef", false, false),
        ] {
            assert_eq!(Channel::Stable.accepts(&v(tag)), stable, "{tag}");
            assert_eq!(Channel::Beta.accepts(&v(tag)), beta, "{tag}");
        }
        // A stable release ranks above the betas of its number.
        assert!(crate::is_newer("v0.7.7", "0.7.7-beta.20261025.9"));
        assert!(crate::is_newer("v0.7.7-beta.20261026.1", "0.7.7-beta.20261025.9"));
        assert!(crate::is_newer("v0.7.7-beta.20261025.10", "0.7.7-beta.20261025.9"));
    }

    #[test]
    fn the_package_of_this_system_is_the_one_the_release_publishes() {
        let name = package_name("v0.9.0");
        if cfg!(windows) {
            assert_eq!(name.as_deref(), Some("Aurora-Viewer-v0.9.0-windows-x64.zip"));
        } else if cfg!(target_os = "linux") {
            assert_eq!(name.as_deref(), Some("Aurora-Viewer-v0.9.0-linux-x64.tar.gz"));
        } else {
            // macOS: installed by hand, no package to install
            assert_eq!(name, None);
        }
    }

    #[test]
    fn reject_external_or_credentialled_assets() {
        let production = Source::default();
        assert!(
            production
                .validate_url("https://github.com/Aurora-Viewer/Aurora-Viewer/releases")
                .is_ok()
        );
        for url in [
            "http://github.com/a",
            "https://example.org/a",
            "https://name:password@github.com/a",
            "file:///a",
        ] {
            assert!(production.validate_url(url).is_err());
        }
        let local = Source::local("http://127.0.0.1:8099/releases").expect("loopback");
        assert!(local.validate_url("http://127.0.0.1:8099/package.zip").is_ok());
        assert!(local.validate_url("http://127.0.0.1:8100/package.zip").is_err());
        assert!(Source::local("https://example.org/feed").is_err());
    }
}
