//! Grid catalogue and `get_grid_info`, following LLGridManager::gridInfoResponderCB
//! (indra/newview/fsgridhandler.cpp) and LLSLURL (fsslurl.cpp, originally LGPL 2.1).
//! Network discovery is explicit and asynchronous; the catalogue contains no accounts.

use crate::settings::{GridChoice, Settings};
use aurora_net::OpenSimExtras;
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
    /// The currency helper's base (`helperuri` or `economy`): Buy L$ and
    /// Buy Land post to its `currency.php` and `landtool.php`.
    pub helper: String,
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
            &self.helper,
        ] {
            if !page.trim().is_empty() && web_url(page).is_none() {
                return Err(Error::Address);
            }
        }
        Ok(())
    }

    /// LLGridManager::getHelperURI: the helper's base, which its page names
    /// are appended to (a missing final slash is added).
    pub fn helper_page(&self) -> Option<String> {
        web_url(&self.helper)?;
        let helper = self.helper.trim();
        Some(if helper.ends_with('/') {
            helper.to_owned()
        } else {
            format!("{helper}/")
        })
    }

    /// A Retrieve's answer laid over what the user had: a field the grid does
    /// not publish keeps the user's value instead of being blanked (OSgrid
    /// and Kitely, for instance, publish no search page).
    pub fn refreshed(self, mut retrieved: Grid) -> Grid {
        let kept = [
            (&mut retrieved.nickname, self.nickname),
            (&mut retrieved.gatekeeper, self.gatekeeper),
            (&mut retrieved.welcome, self.welcome),
            (&mut retrieved.register, self.register),
            (&mut retrieved.password, self.password),
            (&mut retrieved.help, self.help),
            (&mut retrieved.about, self.about),
            (&mut retrieved.search, self.search),
            (&mut retrieved.profile, self.profile),
            (&mut retrieved.helper, self.helper),
        ];
        for (field, mine) in kept {
            if field.trim().is_empty() {
                *field = mine;
            }
        }
        retrieved
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
            // gridInfoResponderCB: "sic! economy and helperuri is 2 names
            // for the same". A malformed one only leaves Buy L$ unavailable.
            "helperuri" | "economy" if web_url(&value).is_some() => grid.helper = value,
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

/// The saved grid known by this Hypergrid address (`host[:port]`), through
/// its discovery, login or gatekeeper address.
pub fn known<'a>(settings: &'a Settings, address: &str) -> Option<&'a Grid> {
    let address = authority(address)?;
    settings.grids.iter().find(|grid| {
        [&grid.service, &grid.login, &grid.gatekeeper]
            .into_iter()
            .any(|a| authority(a).as_deref() == Some(address.as_str()))
    })
}

/// The login grid as a Hypergrid address (`host[:port]`): its gatekeeper,
/// else its login address. None on Second Life.
pub fn home(settings: &Settings) -> Option<String> {
    if settings.grid != GridChoice::Custom {
        return None;
    }
    active(settings)
        .and_then(|grid| authority(&grid.gatekeeper))
        .or_else(|| authority(&settings.custom_login_uri))
}

/// The grid a region is on, from its GridURL, else the login grid (`home`).
pub fn here(home: Option<&str>, extras: Option<&OpenSimExtras>) -> Option<String> {
    let home = home?;
    Some(extras.and_then(|e| authority(&e.grid_url)).unwrap_or_else(|| home.to_owned()))
}

/// The grid the agent is on now, as a Hypergrid address (`host[:port]`), on
/// an OpenSim grid; None on Second Life. After a Hypergrid teleport it is
/// the visited grid, not the login one. Port of
/// LFSimFeatureHandler::hyperGridURL: the region's GridURL, else the login
/// grid's gatekeeper, else the login grid.
pub fn current(settings: &Settings, extras: Option<&OpenSimExtras>) -> Option<String> {
    here(home(settings).as_deref(), extras)
}

/// The grid of a Hypergrid link region, from the name OpenSim's
/// HypergridLinker gives it on the map ("http://host:port/ Region" or
/// "host:port:Region"); None for a region of this grid.
pub fn link_grid(region_name: &str) -> Option<String> {
    let name = region_name.trim();
    if name.starts_with("http://") || name.starts_with("https://") {
        let url = name.split_whitespace().next()?;
        return authority(url);
    }
    // host:port:Region (a region name has no colon)
    let (address, region) = name.rsplit_once(':')?;
    if region.trim().is_empty() || !address.contains(':') && !address.contains('.') {
        return None;
    }
    authority(address)
}

/// Whether two Hypergrid addresses name the same grid, by any of the
/// addresses saved for it.
pub fn same_grid(settings: &Settings, a: &str, b: &str) -> bool {
    let (Some(a), Some(b)) = (authority(a), authority(b)) else {
        return false;
    };
    if a == b {
        return true;
    }
    if is_login_grid(settings, &a) || is_login_grid(settings, &b) {
        return is_login_grid(settings, &a) && is_login_grid(settings, &b);
    }
    known(settings, &a).is_some_and(|grid| known(settings, &b) == Some(grid))
}

/// Whether a Hypergrid address is the grid we logged in to, by any of its
/// addresses (login, discovery or gatekeeper).
fn is_login_grid(settings: &Settings, address: &str) -> bool {
    if authority(&settings.custom_login_uri).as_deref() == Some(address) {
        return true;
    }
    active(settings).is_some_and(|grid| {
        [&grid.service, &grid.login, &grid.gatekeeper]
            .into_iter()
            .any(|a| authority(a).as_deref() == Some(address))
    })
}

/// The name to show for the grid the agent is on now: its saved name, else
/// its Hypergrid address. None on Second Life.
pub fn current_label(settings: &Settings, extras: Option<&OpenSimExtras>) -> Option<String> {
    let address = current(settings, extras)?;
    if let Some(grid) = known(settings, &address) {
        return Some(grid.name.clone());
    }
    if is_login_grid(settings, &address)
        && let Some(grid) = active(settings)
    {
        return Some(grid.name.clone());
    }
    Some(address)
}

/// LLSD default "SearchURLOpenSim": the search page of an OpenSim grid that
/// names none.
pub const OPENSIM_SEARCH: &str = "http://search.metaverseink.com/opensim/results.jsp?query=[QUERY]&submit=[CATEGORY]";

/// The web search page on an OpenSim grid (None on Second Life), as
/// LFSimFeatureHandler::searchURL picks it: the region's search-server-url,
/// else the login response's "search", else the saved grid's page while on
/// it, else the OpenSim default. Then FSPanelSearchWeb::loadURL's
/// substitutions, with OpenSim's maturity (13, 21, 42) and an empty
/// [CATEGORY] (what that code ends up sending). No session or token is ever
/// added for another grid's site.
pub fn opensim_search_url(
    settings: &Settings,
    extras: Option<&OpenSimExtras>,
    login_search: &str,
    query: &str,
    language: &str,
    maturity: u8,
) -> Option<String> {
    if settings.grid != GridChoice::Custom {
        return None;
    }
    let on_login_grid = current(settings, extras).is_some_and(|address| is_login_grid(settings, &address));
    let saved = active(settings).filter(|_| on_login_grid).map(|grid| grid.search.as_str());
    let template = [extras.map(|e| e.search_url.as_str()), Some(login_search), saved]
        .into_iter()
        .flatten()
        .map(str::trim)
        .find(|url| web_url(url).is_some())
        .unwrap_or(OPENSIM_SEARCH);
    let url = template
        .replace("[QUERY]", &crate::slurl::encode(query))
        .replace("[LANGUAGE]", language)
        .replace("[MATURITY]", &maturity.to_string())
        .replace("[CATEGORY]", "");
    web_url(&url).map(|_| url)
}

/// The map tile server: the region's map-server-url on an OpenSim grid,
/// else the login response's, else Second Life's
/// (LFSimFeatureHandler::mapServerURL, else CurrentMapServerURL).
pub fn map_server<'a>(extras: Option<&'a OpenSimExtras>, login_map: &'a str) -> &'a str {
    [extras.map(|e| e.map_url.as_str()), Some(login_map)]
        .into_iter()
        .flatten()
        .map(str::trim)
        .find(|url| web_url(url).is_some())
        .unwrap_or("https://map.secondlife.com/")
}

/// Preserve the destination grid through context menus and place windows that
/// carry a region string. Only the app translates it into the simulator's name.
pub fn target(grid: &str, region: &str) -> String {
    format!("hop://{grid}/{}", crate::slurl::encode(region))
}

/// What a destination becomes for the simulator: a region name on the grid
/// we are on, `gatekeeper:region` for another one (LLTeleportHistory::goToItem
/// and LLSLURL's Hypergrid handling). `extras` is the agent's region's
/// OpenSimExtras, so that after a Hypergrid teleport "this grid" is the
/// visited one: the login grid's own links then go back through its
/// gatekeeper.
pub fn resolve(settings: &Settings, extras: Option<&OpenSimExtras>, destination: &str) -> Result<String, Error> {
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
    let same_grid = current(settings, extras).is_some_and(|here| same_grid(settings, &here, target_authority));
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
    fn a_hypergrid_link_region_names_its_grid() {
        assert_eq!(
            link_grid("http://hg.example-grid.org:80/ harbor plaza").as_deref(),
            // port 80 is http's own: the address has none
            Some("hg.example-grid.org")
        );
        assert_eq!(
            link_grid("grid.other.example:8002:Sandbox").as_deref(),
            Some("grid.other.example:8002")
        );
        assert_eq!(
            link_grid("hg.example-grid.org:Harbor Plaza").as_deref(),
            Some("hg.example-grid.org")
        );
        for here in ["Harbor Plaza", "THE QUIET DEN", "  ", "Sandbox:"] {
            assert_eq!(link_grid(here), None, "{here}");
        }
    }

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
            <economy>https://grid.example/currency</economy>
            </gridinfo>"#,
        )
        .expect("grid information");
        assert_eq!(grid.name, "Example & Grid");
        assert_eq!(grid.nickname, "example");
        assert_eq!(grid.service, "https://grid.example:8002/service");
        assert_eq!(grid.login, "https://login.example/endpoint");
        assert_eq!(grid.gatekeeper, "https://gate.example:8003");
        assert_eq!(grid.profile, "https://profiles.example/");
        assert_eq!(grid.helper_page().as_deref(), Some("https://grid.example/currency/"));
        for page in [&grid.welcome, &grid.register, &grid.password, &grid.help, &grid.about] {
            assert!(!page.is_empty());
        }
        assert_eq!(grid.search, "https://grid.example/search?q=[QUERY]&m=[MATURITY]&l=[LANGUAGE]");
        let fallback = parse_info("localhost:9000", b"<gridinfo/>").expect("manual endpoint fallback");
        assert_eq!(fallback.login, "http://localhost:9000/");
        assert_eq!(fallback.name, "localhost:9000");
        assert!(fallback.search.is_empty());
        assert!(fallback.helper_page().is_none(), "no helper: Buy L$ unavailable");
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
        let grid = parse_info("grid.example", b"<gridinfo><economy>javascript:alert(1)</economy></gridinfo>")
            .expect("a bad helper is dropped, not the grid");
        assert!(grid.helper_page().is_none());
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
            assert_eq!(resolve(&settings, None, &target(source, "My Region")), Ok("My Region".into()));
        }
        assert_eq!(
            resolve(&settings, None, "hop://other.example:8002/My%20Region/1/2/3"),
            Ok("other-gate.example:8003:My Region".into())
        );
        assert_eq!(
            resolve(&settings, None, "hop://unknown.example:9000/My%20Region"),
            Ok("unknown.example:9000:My Region".into())
        );
        assert_eq!(
            resolve(
                &settings,
                None,
                "x-grid-location-info://other.example:8002/region/My%20Region/1/2/3"
            ),
            Ok("other-gate.example:8003:My Region".into())
        );
        assert_eq!(resolve(&settings, None, "hop://other.example/Region/NaN/1/2"), Err(Error::Address));
        assert_eq!(resolve(&settings, None, "Local Region"), Ok("Local Region".into()));
        settings.grid = GridChoice::SecondLife;
        assert_eq!(
            resolve(&settings, None, "hop://other.example:8002/Region/1/2/3"),
            Err(Error::CrossGrid)
        );
        settings.grid = GridChoice::SecondLifeBeta;
        assert_eq!(
            resolve(&settings, None, "hop://other.example:8002/Region/1/2/3"),
            Err(Error::CrossGrid)
        );
    }

    #[test]
    fn retrieve_keeps_what_the_grid_does_not_publish() {
        let mut mine = Grid::manual("https://grid.example").expect("grid");
        mine.search = "https://my.search.example/?q=[QUERY]".into();
        mine.help = "https://my.help.example/".into();
        mine.nickname = "mine".into();
        let mut retrieved = Grid::manual("https://grid.example").expect("grid");
        retrieved.name = "Example".into();
        retrieved.help = "https://grid.example/help".into();
        let merged = mine.refreshed(retrieved);
        assert_eq!(merged.name, "Example");
        // published: the grid's value
        assert_eq!(merged.help, "https://grid.example/help");
        // not published: the user's value
        assert_eq!(merged.search, "https://my.search.example/?q=[QUERY]");
        assert_eq!(merged.nickname, "mine");
    }

    /// The login grid (with a gatekeeper) and a second saved grid, logged in
    /// to the first.
    fn two_grids() -> Settings {
        let mut settings = Settings {
            grid: GridChoice::Custom,
            custom_login_uri: "https://login.example:8002/".into(),
            ..Default::default()
        };
        let mut home = Grid::manual("https://grid.example").expect("home grid");
        home.name = "Home".into();
        home.login = settings.custom_login_uri.clone();
        home.gatekeeper = "https://gate.example:8003".into();
        home.search = "https://grid.example/search?q=[QUERY]".into();
        store(&mut settings, home, None).expect("home");
        let mut other = Grid::manual("https://other.example:8002").expect("other grid");
        other.name = "Other".into();
        other.gatekeeper = "https://other-gate.example:8003/".into();
        store(&mut settings, other, None).expect("other");
        settings
    }

    fn visiting(grid_url: &str) -> OpenSimExtras {
        OpenSimExtras {
            grid_url: grid_url.into(),
            ..Default::default()
        }
    }

    #[test]
    fn the_current_grid_follows_the_region_after_a_hypergrid_teleport() {
        let settings = two_grids();
        // before SimulatorFeatures, or a region without GridURL: the login
        // grid's gatekeeper
        assert_eq!(current(&settings, None).as_deref(), Some("gate.example:8003"));
        assert_eq!(current_label(&settings, None).as_deref(), Some("Home"));
        assert_eq!(
            current(&settings, Some(&OpenSimExtras::default())).as_deref(),
            Some("gate.example:8003")
        );
        // GridURL wins, its scheme already stripped by OpenSimExtras
        let away = visiting("other-gate.example:8003");
        assert_eq!(current(&settings, Some(&away)).as_deref(), Some("other-gate.example:8003"));
        assert_eq!(current_label(&settings, Some(&away)).as_deref(), Some("Other"));
        // an unsaved grid shows its address
        let unknown = visiting("hg.unknown.example:80");
        assert_eq!(current_label(&settings, Some(&unknown)).as_deref(), Some("hg.unknown.example"));
        // Second Life has no Hypergrid address
        let sl = Settings::default();
        assert_eq!(current(&sl, Some(&away)), None);
    }

    #[test]
    fn destinations_are_judged_from_the_grid_we_are_on() {
        let settings = two_grids();
        let away = visiting("other.example:8002");
        // on the visited grid its own regions are local, by any of its addresses
        assert_eq!(
            resolve(&settings, Some(&away), "hop://other.example:8002/Wright%20Plaza/1/2/3"),
            Ok("Wright Plaza".into())
        );
        assert_eq!(
            resolve(&settings, Some(&away), "hop://other-gate.example:8003/Wright%20Plaza"),
            Ok("Wright Plaza".into())
        );
        // and the login grid is reached back through its gatekeeper
        assert_eq!(
            resolve(&settings, Some(&away), "hop://grid.example/My%20Region"),
            Ok("gate.example:8003:My Region".into())
        );
        assert_eq!(
            resolve(&settings, Some(&away), "hop://login.example:8002/My%20Region"),
            Ok("gate.example:8003:My Region".into())
        );
        // a bare name stays on the grid we are on
        assert_eq!(resolve(&settings, Some(&away), "Wright Plaza"), Ok("Wright Plaza".into()));
        // at home, a GridURL spelled unlike the saved addresses is still home
        let home = visiting("grid.example:80");
        assert_eq!(
            resolve(&settings, Some(&home), "hop://gate.example:8003/My%20Region"),
            Ok("My Region".into())
        );
    }

    #[test]
    fn grids_compare_by_any_of_their_addresses() {
        let settings = two_grids();
        assert_eq!(home(&settings).as_deref(), Some("gate.example:8003"));
        assert_eq!(home(&Settings::default()), None);
        assert_eq!(
            here(Some("gate.example:8003"), Some(&visiting("http://other.example:8002/"))).as_deref(),
            Some("other.example:8002")
        );
        assert_eq!(here(None, Some(&visiting("other.example:8002"))), None);
        // the login grid by its login, discovery or gatekeeper address
        assert!(same_grid(&settings, "login.example:8002", "gate.example:8003"));
        assert!(same_grid(&settings, "grid.example", "gate.example:8003"));
        // another saved grid by its own addresses
        assert!(same_grid(&settings, "other.example:8002", "other-gate.example:8003"));
        assert!(!same_grid(&settings, "other.example:8002", "gate.example:8003"));
        assert!(!same_grid(&settings, "unknown.example", "other.example:8002"));
        assert!(same_grid(&settings, "unknown.example:80", "unknown.example"));
    }

    #[test]
    fn opensim_search_follows_the_region_then_the_login_then_the_default() {
        let settings = two_grids();
        let url = |extras: Option<&OpenSimExtras>, login: &str| opensim_search_url(&settings, extras, login, "café & music", "fr", 21);
        // the region's search-server-url first
        let region = OpenSimExtras {
            search_url: "https://region.example/find?q=[QUERY]&m=[MATURITY]&c=[CATEGORY]".into(),
            ..Default::default()
        };
        assert_eq!(
            url(Some(&region), "https://login.example/s?q=[QUERY]").as_deref(),
            Some("https://region.example/find?q=caf%C3%A9%20%26%20music&m=21&c=")
        );
        // then the login response's "search"
        assert_eq!(
            url(None, "https://login.example/s?q=[QUERY]").as_deref(),
            Some("https://login.example/s?q=caf%C3%A9%20%26%20music")
        );
        // then the saved grid's page, on that grid only
        assert_eq!(
            url(None, "").as_deref(),
            Some("https://grid.example/search?q=caf%C3%A9%20%26%20music")
        );
        let away = visiting("other.example:8002");
        assert_eq!(
            url(Some(&away), "").as_deref(),
            Some("http://search.metaverseink.com/opensim/results.jsp?query=caf%C3%A9%20%26%20music&submit=")
        );
        // never a page that is not http(s)
        assert_eq!(
            url(None, "javascript:alert(1)").as_deref(),
            Some("https://grid.example/search?q=caf%C3%A9%20%26%20music")
        );
        // and nothing of this on Second Life
        assert_eq!(opensim_search_url(&Settings::default(), Some(&region), "", "x", "en", 13), None);
    }

    #[test]
    fn the_map_server_follows_the_region() {
        let region = OpenSimExtras {
            map_url: "https://maps.other.example/".into(),
            ..Default::default()
        };
        assert_eq!(
            map_server(Some(&region), "https://maps.home.example/"),
            "https://maps.other.example/"
        );
        assert_eq!(
            map_server(Some(&OpenSimExtras::default()), "https://maps.home.example/"),
            "https://maps.home.example/"
        );
        assert_eq!(map_server(None, " "), "https://map.secondlife.com/");
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
