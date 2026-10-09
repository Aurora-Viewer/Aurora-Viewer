//! "Son" (LLPanelLandAudio::refresh / onCommitAny, with Firestorm's saved
//! stream list: FIRE-593 add / remove / copy, FIRE-29157 drop a URL the
//! region refused).

use super::general::set;
use super::{LandUi, View, check};
use crate::theme::Palette;
use crate::world::World;
use crate::world::land::powers;
use aurora_net::{parcel_flags as pf, region_flags as rf};
use egui::Vec2;

/// The music URL as onCommitAny sends it: trimmed, "http://" added when
/// there is no scheme.
pub(super) fn normalize_url(url: &str) -> String {
    let url = url.trim();
    if !url.is_empty() && !url.contains("://") {
        format!("http://{url}")
    } else {
        url.to_owned()
    }
}

/// Returns true when the saved stream list changed (to save the settings).
pub(super) fn show(ui: &mut egui::Ui, p: &Palette, s: &mut LandUi, v: &View, world: &mut World, streams: &mut Vec<String>) -> bool {
    let Some(parcel) = v.parcel.clone() else {
        super::no_selection(ui, p);
        return false;
    };
    let mut streams_changed = false;
    let can_media = v.can(powers::LAND_CHANGE_MEDIA);
    // FIRE-29157: the region answered with another URL than the one we set
    // (only checked on the parcel we set it on, unlike Firestorm which also
    // dropped it when another parcel was shown)
    if let Some((local_id, url)) = s.last_music_url.clone()
        && local_id == parcel.local_id
        && world.land.sel.as_ref().is_some_and(|sel| sel.revision > s.last_music_rev)
    {
        if parcel.music_url != url && !url.is_empty() {
            log::warn!("removing stream from saved streams list because it was rejected by the region: {url}");
            streams.retain(|x| *x != url);
            streams_changed = true;
        }
        s.last_music_url = None;
    }
    let mut commit: Option<String> = None;
    super::row(ui, p, "URL de la musique :", |ui| {
        s.music_url.sync(&parcel.music_url);
        let r = ui.add_enabled(
            can_media,
            egui::TextEdit::singleline(&mut s.music_url.text).desired_width(ui.available_width() - 4.0 * 26.0),
        );
        if let Some(u) = s.music_url.after(&r) {
            commit = Some(u);
        }
        // the combo's drop-down: the parcel's URL, then the saved streams
        ui.add_enabled_ui(can_media, |ui| {
            ui.menu_button("▾", |ui| {
                ui.set_min_width(320.0);
                if ui.button(parcel.music_url.as_str()).clicked() {
                    ui.close();
                }
                ui.separator();
                for url in streams.iter() {
                    if ui.button(url.as_str()).clicked() {
                        commit = Some(url.clone());
                        ui.close();
                    }
                }
            });
        });
        let current = s.music_url.text.trim().to_owned();
        if super::icon_button(ui, p, "plus", can_media)
            .on_hover_text("Ajouter l'URL du flux de musique à la liste des flux enregistrés")
            .clicked()
            && !current.is_empty()
        {
            if streams.contains(&current) {
                log::info!("could not add stream to saved streams list because it is already in the list: {current}");
            } else {
                streams.push(current.clone());
                streams_changed = true;
            }
        }
        if super::icon_button(ui, p, "minus", can_media)
            .on_hover_text("Supprimer l'URL du flux de musique de la liste des flux enregistrés")
            .clicked()
        {
            let before = streams.len();
            let no_http = current.strip_prefix("http://").unwrap_or("").to_owned();
            streams.retain(|x| *x != current && !(!x.contains("://") && *x == no_http));
            streams_changed |= streams.len() != before;
        }
        if super::icon_button(ui, p, "copy", true)
            .on_hover_text("Copier l'URL du flux de musique dans le presse-papiers")
            .clicked()
            && !current.is_empty()
        {
            ui.ctx().copy_text(current);
        }
    });
    let mut flags = parcel.flags;
    let mut any_av = parcel.any_av_sounds;
    let mut group_av = parcel.group_av_sounds || parcel.any_av_sounds;
    let mut obscure = parcel.media.obscure_moap;
    let before = (flags, any_av, group_av, obscure);
    super::row(ui, p, "Son :", |ui| {
        let (_, on) = check(
            ui,
            can_media,
            flags & pf::SOUND_LOCAL != 0,
            "Limiter les sons des gestes et des objets à cette parcelle",
        );
        flags = set(flags, pf::SOUND_LOCAL, on);
    });
    super::row(ui, p, "Sons d'avatar :", |ui| {
        let can_av = v.can(powers::LAND_OPTIONS) && parcel.have_new_parcel_limit_data;
        ui.allocate_ui_with_layout(Vec2::new(130.0, 18.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
            let (_, on) = check(ui, can_av, any_av, "Tout le monde");
            any_av = on;
        });
        // on (and greyed) while "everyone" is
        let (_, on) = check(ui, can_av && !parcel.any_av_sounds, group_av, "Groupe");
        group_av = on;
    });
    let voice_region = v.region_flag(rf::ALLOW_VOICE);
    let allow_voice = flags & pf::ALLOW_VOICE_CHAT != 0;
    super::row(ui, p, "Voix :", |ui| {
        if voice_region {
            let (_, on) = check(ui, can_media, allow_voice, "Activer le chat vocal");
            flags = set(flags, pf::ALLOW_VOICE_CHAT, on);
        } else {
            // voice off for the estate: a disabled box says so
            check(ui, false, false, "Activer la voix (contrôlé par le domaine)");
        }
    });
    ui.horizontal(|ui| {
        ui.add_space(super::KEY_W);
        let (_, on) = check(
            ui,
            voice_region && can_media && allow_voice,
            parcel.flags & pf::USE_ESTATE_VOICE_CHAN == 0,
            "Limiter le chat vocal à cette parcelle",
        );
        flags = set(flags, pf::USE_ESTATE_VOICE_CHAN, !on);
    });
    super::row(ui, p, "Média :", |ui| {
        let (r, on) = check(ui, can_media, obscure, "Masquer MOAP");
        r.on_hover_text(
            "Les médias d'un prim situé en dehors de la parcelle ne doivent pas être lus automatiquement par un agent situé dans cette parcelle et vice versa.",
        );
        obscure = on;
    });
    if commit.is_some() || before != (flags, any_av, group_av, obscure) {
        let music = normalize_url(&commit.unwrap_or_else(|| parcel.music_url.clone()));
        s.music_url.base = music.clone();
        s.music_url.text = music.clone();
        // "everyone" implies the group
        let group = any_av || group_av;
        world.land.update(|u| {
            u.flags = flags;
            u.music_url = music.clone();
            u.any_av_sounds = any_av;
            u.group_av_sounds = group;
            u.obscure_moap = obscure;
        });
        s.last_music_url = Some((parcel.local_id, music));
        s.last_music_rev = world.land.sel.as_ref().map_or(0, |sel| sel.revision);
    }
    streams_changed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn music_urls_get_a_scheme() {
        assert_eq!(normalize_url("  dj.rapa.live:8000/ "), "http://dj.rapa.live:8000/");
        assert_eq!(normalize_url("https://x.fr/s"), "https://x.fr/s");
        assert_eq!(normalize_url(""), "");
    }
}
