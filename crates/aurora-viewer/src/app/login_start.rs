//! The place the login asks for, from the login screen's region field or a
//! SLURL given from outside. Port of FSPanelLogin::onLocationSLURL →
//! LLStartUp::setStartSLURL (indra/newview/fspanellogin.cpp,
//! llstartup.cpp) and construct_start_string
//! (indra/newview/lllogininstance.cpp), originally LGPL 2.1: the Legacy
//! Viewer reads what is typed in its location combo as an LLSLURL, so a
//! pasted SLURL (`http://maps.secondlife.com/secondlife/…`,
//! `secondlife://…`, `hop://…`), `Region/x/y/z` or a bare region name all
//! start there, a bare name at the region's centre on the ground
//! (128, 128, 0).

use crate::settings::Settings;
use crate::slurl::Location;
use aurora_net::StartLocation;

/// The place typed in the region field, when the start menu is on Region.
pub(super) fn region(start_location: &str, typed: &str) -> Option<Location> {
    if start_location != "region" {
        return None;
    }
    crate::slurl::parse(typed)
}

/// The login's start: the region resolved against the login grid (a
/// Hypergrid link of another grid goes through its gatekeeper) and the
/// position in whole metres.
pub(super) fn start(settings: &Settings, location: &Location) -> Result<StartLocation, crate::grids::Error> {
    let name = crate::grids::resolve(settings, None, &location.target())?;
    Ok(StartLocation::Region {
        name,
        x: location.pos.x.round() as u32,
        y: location.pos.y.round() as u32,
        z: location.pos.z.round() as u32,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn login(typed: &str) -> Option<String> {
        let settings = Settings::default();
        region("region", typed).map(|l| start(&settings, &l).expect("main-grid place").to_login_string())
    }

    #[test]
    fn the_region_field_takes_a_slurl_like_the_legacy_viewer() {
        let want = Some("uri:Aurora Démo&12&34&56".to_owned());
        for typed in [
            "http://maps.secondlife.com/secondlife/Aurora%20D%C3%A9mo/12/34/56",
            "https://maps.secondlife.com/secondlife/Aurora%20D%C3%A9mo/12/34/56/?title=x",
            "  secondlife://Aurora Démo/12/34/56  ",
            "secondlife:///app/teleport/Aurora%20D%C3%A9mo/12/34/56",
            "Aurora Démo/12/34/56",
        ] {
            assert_eq!(login(typed), want, "{typed}");
        }
        // a bare name, or a SLURL without a position: the centre, on the
        // ground (LLSLURL's default position)
        assert_eq!(login("Aurora Démo"), Some("uri:Aurora Démo&128&128&0".to_owned()));
        assert_eq!(
            login("http://maps.secondlife.com/secondlife/Ahern"),
            Some("uri:Ahern&128&128&0".to_owned())
        );
        // fractional positions round to whole metres
        assert_eq!(login("Ahern/12.6/34.4/20.5"), Some("uri:Ahern&13&34&21".to_owned()));
    }

    #[test]
    fn the_region_field_counts_only_on_region() {
        assert!(region("region", "   ").is_none());
        assert!(region("last", "Ahern").is_none());
        assert!(region("home", "http://maps.secondlife.com/secondlife/Ahern/1/2/3").is_none());
    }

    #[test]
    fn a_hypergrid_link_is_refused_on_second_life() {
        let settings = Settings::default();
        let location = region("region", "hop://grid.example:8002/Region/1/2/3").expect("a Hypergrid place");
        assert!(start(&settings, &location).is_err());
    }
}
