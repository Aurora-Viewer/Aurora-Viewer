//! The pages of the preferences, one module each, in the order of the
//! sidebar. A page declares its rows through `Cx` for one of its tabs.

pub mod accessibility;
pub mod appearance;
pub mod aurora;
pub mod avatars;
pub mod camera;
pub mod chat;
pub mod commands;
pub mod confirmations;
pub mod controls;
pub mod fun;
pub mod graphics;
pub mod inventory;
pub mod joystick;
pub mod layout;
pub mod media;
pub mod network;
pub mod privacy;
pub mod protections;
pub mod shortcuts;
pub mod sound;
pub mod spelling;
pub mod system;

use super::Page;
use super::cx::Cx;

/// The tabs of a page.
pub fn tabs(page: Page) -> Vec<&'static str> {
    match page {
        Page::Aurora => vec![
            aurora_i18n::tr!("common-windows"),
            aurora_i18n::tr!("ui-options-tab-world-and-interface"),
            aurora_i18n::tr!("common-behavior"),
        ],
        Page::Graphics => vec![
            aurora_i18n::tr!("ui-options-quality"),
            aurora_i18n::tr!("common-light"),
            aurora_i18n::tr!("ui-options-tab-sky-and-environment"),
            aurora_i18n::tr!("ui-options-tab-reflections-and-water"),
            aurora_i18n::tr!("common-image"),
            aurora_i18n::tr!("ui-options-smoothness"),
            aurora_i18n::tr!("region-parcels"),
        ],
        Page::Camera => vec![
            aurora_i18n::tr!("common-viewpoint"),
            aurora_i18n::tr!("common-behavior"),
            aurora_i18n::tr!("common-mouselook"),
        ],
        Page::Controls => vec![aurora_i18n::tr!("ui-options-page-controls"), aurora_i18n::tr!("joystick-title")],
        Page::Avatars => vec![
            aurora_i18n::tr!("ui-options-tab-name-tags"),
            aurora_i18n::tr!("ui-options-names"),
            aurora_i18n::tr!("ui-options-look-at-targets"),
            aurora_i18n::tr!("ui-options-aurora-animations"),
        ],
        Page::Chat => vec![
            aurora_i18n::tr!("common-nearby-chat"),
            aurora_i18n::tr!("ui-options-aurora-notifications"),
            aurora_i18n::tr!("ui-options-tab-keywords"),
            aurora_i18n::tr!("ui-options-tab-history"),
            aurora_i18n::tr!("ui-options-tab-away-and-replies"),
            aurora_i18n::tr!("ui-options-groups-and-blocking"),
            aurora_i18n::tr!("ui-options-tab-spelling"),
        ],
        Page::Sound => vec![
            aurora_i18n::tr!("ui-options-volumes"),
            aurora_i18n::tr!("ui-options-interface-sounds"),
            aurora_i18n::tr!("common-voice"),
        ],
        Page::Layout => vec![
            aurora_i18n::tr!("ui-options-tab-bars-and-windows"),
            aurora_i18n::tr!("ui-options-hover-tips"),
            aurora_i18n::tr!("ui-options-tab-startup"),
        ],
        Page::Appearance => vec![
            aurora_i18n::tr!("ui-options-font"),
            aurora_i18n::tr!("ui-options-colors"),
            aurora_i18n::tr!("extra-skin"),
        ],
        Page::Shortcuts => shortcuts::tabs(),
        _ => vec![page.name()],
    }
}

/// The rows of one tab of a page.
pub fn draw(cx: &mut Cx, page: Page, tab: usize) {
    match page {
        Page::Aurora => aurora::page(cx, tab),
        Page::Graphics => graphics::page(cx, tab),
        Page::Camera => camera::page(cx, tab),
        Page::Avatars => avatars::page(cx, tab),
        Page::Chat => chat::page(cx, tab),
        Page::Sound => sound::page(cx, tab),
        Page::Media => media::page(cx),
        Page::Privacy => privacy::page(cx),
        Page::Protections => protections::page(cx),
        Page::Layout => layout::page(cx, tab),
        Page::Appearance => appearance::page(cx, tab),
        Page::Accessibility => accessibility::page(cx),
        Page::Confirmations => confirmations::page(cx),
        Page::Inventory => inventory::page(cx),
        Page::Controls => {
            if tab == 0 {
                controls::page(cx)
            } else {
                joystick::page(cx)
            }
        }
        Page::Shortcuts => shortcuts::page(cx, tab),
        Page::Commands => commands::page(cx),
        Page::System => system::page(cx),
        Page::Network => network::page(cx),
        Page::Fun => fun::page(cx),
    }
}
