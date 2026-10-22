//! Grid catalogue and `get_grid_info`, following LLGridManager::gridInfoResponderCB
//! (indra/newview/fsgridhandler.cpp) and LLSLURL (fsslurl.cpp, originally LGPL 2.1).
//! Network discovery is explicit and asynchronous; the catalogue contains no accounts.

use crate::settings::{GridChoice, Settings};
use reqwest::Url;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Grid {
    pub name: String,
    pub nickname: String,
    /// Discovery service, kept independently of the advertised login endpoint.
    pub service: String,
    pub login: String,
    pub gatekeeper: String,
    pub welcome: String,
    pub register: String,
    pub password: String,
    pub help: String,
    pub about: String,
    pub search: String,
    pub profile: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    Address,
    Metadata,
    Network,
    TooLarge,
    Protected,
    CrossGrid,
}

impl Error {
    pub fn text(self) -> &'static str {
        aurora_i18n::text(match self {
            Self::Address => "grids-error-address",
            Self::Metadata => "grids-error-metadata",
            Self::Network => "grids-error-network",
            Self::TooLarge => "grids-error-size",
            Self::Protected => "grids-error-protected",
            Self::CrossGrid => "grids-error-cross-grid",
        })
    }
}

/// HTTP(S) endpoints only, without embedded credentials, fragments or queries.
pub fn service(input: &str) -> Result<String, Error> {
    let input = input.trim();
    let address = if input.contains("://") {
        input.to_owned()
    } else {
        format!("http://{input}")
    };
    let url = web_url(&address).ok_or(Error::Address)?;
    if url.query().is_some() || url.fragment().is_some() {
        return Err(Error::Address);
    }
    Ok(url.as_str().trim_end_matches('/').to_owned())
}

fn web_url(input: &str) -> Option<Url> {
    let url = Url::parse(input.trim()).ok()?;
    (matches!(url.scheme(), "http" | "https") && url.host_str().is_some() && url.username().is_empty() && url.password().is_none())
        .then_some(url)
}

pub fn authority(input: &str) -> Option<String> {
    let url = web_url(&service(input).ok()?)?;
    Some(url.origin().ascii_serialization().split_once("://")?.1.to_owned())
}

fn protected(uri: &str) -> bool {
    web_url(uri)
        .and_then(|url| url.host_str().map(str::to_owned))
        .is_some_and(|host| host == "lindenlab.com" || host.ends_with(".lindenlab.com"))
}

impl Grid {
    pub fn manual(address: &str) -> Result<Self, Error> {
        let address = service(address)?;
        if protected(&address) {
            return Err(Error::Protected);
        }
        Ok(Self {
            name: authority(&address).ok_or(Error::Address)?,
            service: address.clone(),
            login: format!("{address}/"),
            ..Self::default()
        })
    }

    pub fn validate(&mut self) -> Result<(), Error> {
        self.service = service(&self.service)?;
        let login = service(&self.login)?;
        if protected(&self.service) || protected(&login) {
            return Err(Error::Protected);
        }
        self.login = web_url(&self.login).ok_or(Error::Address)?.to_string();
        self.name = self.name.trim().to_owned();
        if self.name.is_empty() {
            self.name = authority(&self.service).ok_or(Error::Address)?;
        }
        if !self.gatekeeper.trim().is_empty() {
            self.gatekeeper = service(&self.gatekeeper)?;
        }
        for page in [
            &self.welcome,
            &self.register,
            &self.password,
            &self.help,
            &self.about,
            &self.search,
            &self.profile,
        ] {
            if !page.trim().is_empty() && web_url(page).is_none() {
                return Err(Error::Address);
            }
        }
        Ok(())
    }

    pub fn search_url(&self, query: &str, language: &str, maturity: u8) -> Option<String> {
        if self.search.is_empty() {
            return None;
        }
        // FSPanelSearchWeb::loadURL / LLWeb::expandURLSubstitutions. Never send
        // a Second Life session or OpenID cookie to another grid's search site.
        let url = self
            .search
            .replace("[QUERY]", &crate::slurl::encode(query))
            .replace("[LANGUAGE]", language)
            .replace("[MATURITY]", &maturity.to_string());
        web_url(&url).map(|_| url)
    }
}

pub fn parse_info(address: &str, bytes: &[u8]) -> Result<Grid, Error> {
    let mut grid = Grid::manual(address)?;
    let root = aurora_llsd::xml::parse(bytes).map_err(|_| Error::Metadata)?;
    if root.name != "gridinfo" {
        return Err(Error::Metadata);
    }
    for node in root.elements() {
        let value = node.text().trim().to_owned();
        match node.name.as_str() {
            "gridname" => grid.name = value,
            "gridnick" => grid.nickname = value,
            "login" if !value.is_empty() => grid.login = value,
            "gatekeeper" => grid.gatekeeper = value,
            "welcome" => grid.welcome = value,
            "register" => grid.register = value,
            "password" => grid.password = value,
            "help" => grid.help = value,
            "about" => grid.about = value,
            "search" => grid.search = value,
            "web_profile_url" | "profileuri" => grid.profile = value,
            _ => {}
        }
    }
    grid.validate()?;
    Ok(grid)
}

pub async fn discover(address: &str) -> Result<Grid, Error> {
    let grid = Grid::manual(address)?;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .user_agent(aurora_version::user_agent())
        .build()
        .map_err(|_| Error::Network)?;
    let mut response = client
        .get(format!("{}/get_grid_info", grid.service))
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|_| Error::Network)?;
    const LIMIT: usize = 64 * 1024;
    if response.content_length().is_some_and(|len| len > LIMIT as u64) {
        return Err(Error::TooLarge);
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| Error::Network)? {
        if bytes.len() + chunk.len() > LIMIT {
            return Err(Error::TooLarge);
        }
        bytes.extend_from_slice(&chunk);
    }
    parse_info(&grid.service, &bytes)
}

pub fn active(settings: &Settings) -> Option<&Grid> {
    if settings.grid != GridChoice::Custom {
        return None;
    }
    let login = service(&settings.custom_login_uri).ok()?;
    settings
        .grids
        .iter()
        .find(|grid| service(&grid.login).ok().as_ref() == Some(&login))
}

pub fn store(settings: &mut Settings, mut grid: Grid, previous: Option<&str>) -> Result<(), Error> {
    grid.validate()?;
    let login = service(&grid.login)?;
    // One entry per discovery service or login endpoint, including spelling/trailing-slash variants.
    settings.grids.retain(|old| {
        Some(old.service.as_str()) != previous && old.service != grid.service && service(&old.login).ok().as_ref() != Some(&login)
    });
    settings.grids.push(grid);
    settings.grids.sort_by_key(|grid| grid.name.to_lowercase());
    Ok(())
}

/// Preserve the destination grid through context menus and place windows that
/// carry a region string. Only the app translates it into the simulator's name.
pub fn target(grid: &str, region: &str) -> String {
    format!("hop://{grid}/{}", crate::slurl::encode(region))
}

pub fn resolve(settings: &Settings, destination: &str) -> Result<String, Error> {
    let Some(location) = crate::slurl::parse_hypergrid(destination) else {
        if destination.to_ascii_lowercase().starts_with("hop://") || destination.to_ascii_lowercase().starts_with("x-grid-location-info://")
        {
            return Err(Error::Address);
        }
        return Ok(destination.to_owned());
    };
    let target_authority = location.grid.as_deref().ok_or(Error::Address)?;
    if settings.grid != GridChoice::Custom {
        return Err(Error::CrossGrid);
    }
    let current = active(settings);
    let same_grid = authority(&settings.custom_login_uri).as_deref() == Some(target_authority)
        || current.is_some_and(|grid| {
            authority(&grid.service).as_deref() == Some(target_authority)
                || authority(&grid.gatekeeper).as_deref() == Some(target_authority)
        });
    if same_grid {
        return Ok(location.region);
    }
    let other = settings.grids.iter().find(|grid| {
        authority(&grid.service).as_deref() == Some(target_authority)
            || authority(&grid.login).as_deref() == Some(target_authority)
            || authority(&grid.gatekeeper).as_deref() == Some(target_authority)
    });
    let gatekeeper = other
        .and_then(|grid| authority(&grid.gatekeeper))
        .unwrap_or_else(|| target_authority.to_owned());
    if protected(&format!("http://{gatekeeper}")) {
        return Err(Error::CrossGrid);
    }
    Ok(format!("{gatekeeper}:{}", location.region))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_info_fields_and_manual_fallback() {
        let grid = parse_info(
            "https://grid.example:8002/service/",
            br#"<?xml version="1.0"?><gridinfo>
            <gridname>Example &amp; Grid</gridname><gridnick>example</gridnick>
            <login>https://login.example/endpoint</login><gatekeeper>https://gate.example:8003/</gatekeeper>
            <welcome>https://grid.example/welcome</welcome><register>https://grid.example/register</register>
            <password>https://grid.example/password</password><help>https://grid.example/help</help>
            <about>https://grid.example/</about><search>https://grid.example/search?q=[QUERY]&amp;m=[MATURITY]&amp;l=[LANGUAGE]</search>
            <profileuri>https://profiles.example/</profileuri><unknown>ignored</unknown>
            </gridinfo>"#,
        )
        .expect("grid information");
        assert_eq!(grid.name, "Example & Grid");
        assert_eq!(grid.nickname, "example");
        assert_eq!(grid.service, "https://grid.example:8002/service");
        assert_eq!(grid.login, "https://login.example/endpoint");
        assert_eq!(grid.gatekeeper, "https://gate.example:8003");
        assert_eq!(grid.profile, "https://profiles.example/");
        for page in [&grid.welcome, &grid.register, &grid.password, &grid.help, &grid.about] {
            assert!(!page.is_empty());
        }
        assert_eq!(
            grid.search_url("café & music", "fr", 21).as_deref(),
            Some("https://grid.example/search?q=caf%C3%A9%20%26%20music&m=21&l=fr")
        );
        let fallback = parse_info("localhost:9000", b"<gridinfo/>").expect("manual endpoint fallback");
        assert_eq!(fallback.login, "http://localhost:9000/");
        assert_eq!(fallback.name, "localhost:9000");
        assert!(fallback.search_url("test", "en", 13).is_none());
    }

    #[test]
    fn grid_info_rejects_invalid_and_protected_endpoints() {
        for input in [
            "",
            "file:///C:/grid",
            "http://user:password@grid.example/",
            "https://grid.example/?token=secret",
        ] {
            assert!(Grid::manual(input).is_err(), "{input}");
        }
        assert_eq!(Grid::manual(aurora_net::login::SL_MAIN_GRID), Err(Error::Protected));
        for body in [
            b"<html/>".as_slice(),
            b"<gridinfo>",
            b"<gridinfo><login>file:///tmp/grid</login></gridinfo>",
            b"<gridinfo><login>https://login.aditi.lindenlab.com/cgi-bin/login.cgi</login></gridinfo>",
            b"<gridinfo><welcome>javascript:alert(1)</welcome></gridinfo>",
        ] {
            assert!(parse_info("grid.example", body).is_err());
        }
    }

    #[test]
    fn grid_catalogue_deduplicates_and_survives_settings_reset() {
        let mut settings = Settings::default();
        let mut grid = Grid::manual("GRID.example:8002/").expect("grid");
        store(&mut settings, grid.clone(), None).expect("store");
        grid.name = "Updated".into();
        store(&mut settings, grid.clone(), None).expect("update");
        assert_eq!(settings.grids.len(), 1);
        settings.grid = GridChoice::Custom;
        settings.custom_login_uri = grid.login.clone();
        assert_eq!(active(&settings).map(|grid| grid.name.as_str()), Some("Updated"));
        let json = serde_json::to_vec(&settings).expect("settings");
        let mut restored: Settings = serde_json::from_slice(&json).expect("settings round trip");
        restored.reset_keeping_account();
        assert_eq!(restored.grids, settings.grids);
        let previous = grid.service.clone();
        grid.service = "https://new.example".into();
        store(&mut restored, grid, Some(&previous)).expect("changed service");
        assert_eq!(restored.grids.len(), 1);
        assert!(
            store(
                &mut restored,
                Grid {
                    login: aurora_net::login::SL_MAIN_GRID.into(),
                    ..Grid::manual("example.org").expect("grid")
                },
                None
            )
            .is_err()
        );
        assert_eq!(restored.grids.len(), 1);
    }

    #[test]
    fn grid_destinations_preserve_authority_and_use_gatekeeper() {
        let mut settings = Settings {
            grid: GridChoice::Custom,
            custom_login_uri: "https://login.example:8002/".into(),
            ..Default::default()
        };
        let mut home = Grid::manual("https://grid.example").expect("home grid");
        home.login = settings.custom_login_uri.clone();
        home.gatekeeper = "https://gate.example:8003".into();
        store(&mut settings, home, None).expect("home");
        let mut other = Grid::manual("https://other.example:8002").expect("other grid");
        other.gatekeeper = "https://other-gate.example:8003/".into();
        store(&mut settings, other, None).expect("other");
        for source in ["grid.example", "login.example:8002", "gate.example:8003"] {
            assert_eq!(resolve(&settings, &target(source, "My Region")), Ok("My Region".into()));
        }
        assert_eq!(
            resolve(&settings, "hop://other.example:8002/My%20Region/1/2/3"),
            Ok("other-gate.example:8003:My Region".into())
        );
        assert_eq!(
            resolve(&settings, "hop://unknown.example:9000/My%20Region"),
            Ok("unknown.example:9000:My Region".into())
        );
        assert_eq!(
            resolve(&settings, "x-grid-location-info://other.example:8002/region/My%20Region/1/2/3"),
            Ok("other-gate.example:8003:My Region".into())
        );
        assert_eq!(resolve(&settings, "hop://other.example/Region/NaN/1/2"), Err(Error::Address));
        assert_eq!(resolve(&settings, "Local Region"), Ok("Local Region".into()));
        settings.grid = GridChoice::SecondLife;
        assert_eq!(resolve(&settings, "hop://other.example:8002/Region/1/2/3"), Err(Error::CrossGrid));
        settings.grid = GridChoice::SecondLifeBeta;
        assert_eq!(resolve(&settings, "hop://other.example:8002/Region/1/2/3"), Err(Error::CrossGrid));
    }

    #[test]
    fn grid_discovery_requests_only_metadata_and_bounds_response() {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().expect("runtime");
        for (status, body, result) in [
            (
                "200 OK",
                "<gridinfo><gridname>Local Test</gridname></gridinfo>".to_owned(),
                Ok("Local Test".to_owned()),
            ),
            ("404 Not Found", "not found".to_owned(), Err(Error::Network)),
            ("200 OK", "x".repeat(65537), Err(Error::TooLarge)),
        ] {
            let server = TcpListener::bind("127.0.0.1:0").expect("local test server");
            let address = format!("http://{}/service", server.local_addr().expect("address"));
            let thread = std::thread::spawn(move || {
                let (mut stream, _) = server.accept().expect("request");
                stream.set_read_timeout(Some(std::time::Duration::from_secs(5))).expect("timeout");
                let mut bytes = [0; 4096];
                let count = stream.read(&mut bytes).expect("read");
                let request = String::from_utf8_lossy(&bytes[..count]);
                assert!(request.starts_with("GET /service/get_grid_info HTTP/1.1\r\n"));
                assert!(!request.to_ascii_lowercase().contains("authorization:"));
                let response = format!(
                    "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes());
            });
            assert_eq!(runtime.block_on(discover(&address)).map(|grid| grid.name), result);
            thread.join().expect("server");
        }
    }
}
