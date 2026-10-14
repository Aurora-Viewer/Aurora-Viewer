//! A bounded GitHub release list, filtered by stable SemVer rather than
//! GitHub's creation date (the assets-only release is not a viewer).

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

#[derive(Clone)]
pub struct Source {
    url: String,
    loopback: bool,
}

impl Default for Source {
    fn default() -> Self {
        Self {
            url: RELEASES.into(),
            loopback: false,
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
        })
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
        let builder = Client::builder()
            .user_agent(aurora_version::user_agent())
            .https_only(!self.loopback)
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
            }));
        // An explicit local feed must not leave loopback through a system proxy.
        let builder = if self.loopback { builder.no_proxy() } else { builder };
        Ok(builder.build()?)
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

    pub async fn check(&self, current: &str) -> Result<Option<Release>> {
        let client = self.client()?;
        let releases: Vec<GithubRelease> = serde_json::from_slice(&self.bytes(&client, &self.url, 4 * 1024 * 1024).await?)?;
        let Some((v, r)) = releases
            .into_iter()
            .filter_map(|r| {
                let v = version(&r.tag_name)?;
                (r.tag_name.starts_with('v') && !r.draft && !r.prerelease && v.pre.is_empty() && v.build.is_empty()).then_some((v, r))
            })
            .max_by(|a, b| a.0.cmp_precedence(&b.0))
        else {
            return Ok(None);
        };
        if !super::is_newer(&v.to_string(), current) {
            return Ok(None);
        }
        self.validate_url(&r.html_url)?;
        let expected = format!("Aurora-Viewer-{}-windows-x64.zip", r.tag_name);
        let archive = r
            .assets
            .iter()
            .find(|a| a.name == expected && a.size > 0 && a.size <= MAX_ARCHIVE)
            .cloned();
        let checksum = r.assets.iter().find(|a| a.name == format!("{expected}.sha256")).cloned();
        let mut required = false;
        let mut maintenance = super::Maintenance::default();
        if let Some(asset) = r.assets.iter().find(|a| a.name == POLICY) {
            let policy: Policy = serde_json::from_slice(&self.bytes(&client, &asset.browser_download_url, 16 * 1024).await?)?;
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
            }
        }
        for a in [&archive, &checksum].into_iter().flatten() {
            self.validate_url(&a.browser_download_url)?;
        }
        Ok(Some(Release {
            tag: r.tag_name,
            version: v.to_string(),
            notes: r.body.unwrap_or_default(),
            url: r.html_url,
            archive,
            checksum,
            required,
            maintenance,
        }))
    }
}

pub fn version(value: &str) -> Option<Version> {
    Version::parse(value.strip_prefix('v').unwrap_or(value)).ok()
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
    pub checksum: Option<Asset>,
    pub required: bool,
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
