//! Préférences › Commandes: the chat bar as a command line, Firestorm's
//! « Conversations › Commandes » tab (FSCmdLine* settings).

use super::{group, row, slider, toggle};
use crate::cmdline::ChatCommandSettings;
use crate::theme::Palette;
use egui::RichText;

/// The name of a chat command, with its usage next to it (Firestorm's
/// « Commandes » tab: « ex: cmd x y z »).
fn command_name(ui: &mut egui::Ui, p: &Palette, label: &str, usage: &str, hint: &str, name: &mut String) -> bool {
    row(ui, p, label, hint, |ui| {
        let c = ui.add(egui::TextEdit::singleline(name).desired_width(110.0)).changed();
        ui.label(RichText::new(usage).size(11.5).color(p.muted));
        c
    })
}

/// The page (panel_preferences_chat.xml, tab-CmdLine).
pub fn page(ui: &mut egui::Ui, p: &Palette, s: &mut ChatCommandSettings) -> bool {
    let mut c = false;
    group(ui, p, "Ligne de commande", |ui| {
        c |= row(
            ui,
            p,
            "Barre de chat",
            "Utiliser la barre de chat pour les lignes de commandes : une ligne qui commence par le nom d'une              commande n'est pas envoyée dans le chat",
            |ui| toggle(ui, p, &mut s.enabled),
        );
        ui.add_enabled_ui(s.enabled, |ui| {
            c |= row(
                ui,
                p,
                "Sortie sur un canal",
                "Envoyer aussi les réponses des commandes sur ce canal de script (chuchotées)",
                |ui| {
                    let mut ch = toggle(ui, p, &mut s.announce_to_channel);
                    ch |= ui
                        .add_enabled(
                            s.announce_to_channel,
                            egui::DragValue::new(&mut s.announce_channel).range(-2_147_483_648..=2_147_483_647),
                        )
                        .changed();
                    ch
                },
            );
        });
    });
    ui.add_enabled_ui(s.enabled, |ui| {
        group(ui, p, "Commandes de téléportation", |ui| {
            c |= command_name(ui, p, "Dans la région", "cmd x y z", "Se téléporter à une position de la région", &mut s.pos);
            c |= command_name(ui, p, "Au sol", "cmd", "Se téléporter au sol, sous l'avatar", &mut s.ground);
            c |= command_name(ui, p, "À l'altitude", "cmd z", "Se téléporter à cette altitude (+z ou -z : relative)", &mut s.height);
            c |= command_name(ui, p, "À la caméra", "cmd", "Se téléporter à la position de la caméra", &mut s.teleport_to_cam);
            c |= command_name(
                ui,
                p,
                "Vers un avatar",
                "cmd nom",
                "Nom partiel, sans tenir compte des majuscules, parmi les avatars à proximité",
                &mut s.tp2,
            );
            c |= command_name(ui, p, "Proposer une téléportation", "cmd clé", "Proposer une téléportation à un avatar (UUID)", &mut s.offer_tp);
            c |= command_name(ui, p, "Domicile", "cmd", "Se téléporter à son domicile", &mut s.teleport_home);
            c |= command_name(ui, p, "Vers une région", "cmd région", "Se téléporter dans une région (région|x y z pour une position)", &mut s.map_to);
            c |= row(
                ui,
                p,
                "Garder la position",
                "Utiliser la même position d'une région à l'autre (sinon 128, 128)",
                |ui| toggle(ui, p, &mut s.map_to_keep_pos),
            );
            c |= command_name(
                ui,
                p,
                "Suivre sur la carte",
                "cmd <x, y, z> | nom",
                "Place une balise sur la carte du monde à une position (format cpcampos) ou sur un avatar à proximité",
                &mut s.track_pos,
            );
        });
        group(ui, p, "Commandes de caméra et d'affichage", |ui| {
            c |= command_name(ui, p, "Distance d'affichage", "cmd mètres", "Changer la distance d'affichage (+n ou -n : relative)", &mut s.draw_distance);
            c |= command_name(ui, p, "Bande passante max.", "cmd kbps", "Pas encore disponible dans Aurora", &mut s.bandwidth);
            c |= command_name(ui, p, "Copier la caméra", "cmd", "Copier la position de la caméra dans le presse-papiers", &mut s.copy_cam);
            c |= command_name(ui, p, "Restaurer la caméra", "cmd pos|focus|roll", "Placer la caméra à une position copiée", &mut s.paste_cam);
        });
        group(ui, p, "Autres commandes", |ui| {
            c |= command_name(ui, p, "Calculatrice", "cmd SIN(2+2)", "Calculer une expression", &mut s.calc);
            c |= command_name(ui, p, "Nom d'un avatar", "cmd clé", "Obtenir le nom d'un avatar à partir de son UUID", &mut s.key_to_name);
            c |= command_name(
                ui,
                p,
                "Lancer de dés",
                "cmd dés faces",
                "cmd [nombre de dés] [nombre de faces] [type de modificateur] [valeur]. Exemples : cmd 1 20, cmd 2 40 + 5.                  Sans paramètres : un dé à 6 faces",
                &mut s.roll_dice,
            );
            c |= command_name(ui, p, "Effacer le chat", "cmd", "Effacer le chat local pour limiter les effets du spam", &mut s.clear_chat);
            c |= command_name(ui, p, "Média", "cmd url type", "Définir et jouer le média de la parcelle", &mut s.media);
            c |= command_name(ui, p, "Musique", "cmd url", "Définir et jouer la musique en streaming", &mut s.music);
            c |= command_name(ui, p, "AO", "cmd on/off", "Pas encore disponible dans Aurora", &mut s.ao);
            c |= command_name(ui, p, "Plateforme", "cmd largeur", "Créer une plateforme sous l'avatar", &mut s.rez_platform);
            c |= row(ui, p, "Largeur de la plateforme", "Largeur quand la commande n'en donne pas", |ui| {
                slider(ui, &mut s.platform_size, 5.0..=64.0, " m")
            });
        });
    });
    c
}
