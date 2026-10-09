//! Preferences floater: categories on the left, grouped settings on the
//! right (label column + control column), flat Aurora style.

mod camera;

use super::fonts::{self, SystemFont};
use super::icons::Icons;
use super::widgets::Floater;
use crate::settings::Settings;
use crate::theme::Palette;
use egui::{Color32, CornerRadius, RichText, Sense, Vec2};

#[derive(Default)]
pub struct OptionsUi {
    pub tab: usize,
    system_fonts: Option<Vec<SystemFont>>,
    font_filter: String,
    /// Disk cache usage (bytes) and whether a clear is running; set by the app.
    pub cache_usage: u64,
    pub cache_clearing: bool,
    /// Detected video memory (MB) and integrated GPU; set by the app.
    pub vram: (Option<u64>, bool),
    /// Set by the "describe nearby objects" button, cleared by the app.
    pub describe_nearby: bool,
    /// Audio devices (set by the app) and microphone level (0..1).
    pub output_devices: Vec<String>,
    pub input_devices: Vec<String>,
    pub mic_level: f32,
    /// Media plugins folder found (empty: none) and plugins running.
    pub media_plugins: String,
    pub media_running: usize,
    /// Shortcut waiting for a key / mouse button (action, slot); the app
    /// captures the next input.
    pub capture: Option<(crate::keybinds::Action, usize)>,
    clear: Option<(crate::keybinds::Action, usize)>,
    /// "Tout réinitialiser" confirmation open.
    confirm_reset: bool,
    /// Sound assets being typed (Firestorm name -> text).
    ui_sound_text: std::collections::HashMap<&'static str, String>,
}

#[derive(Default)]
pub struct OptionsResult {
    pub changed: bool,
    pub font_changed: bool,
    pub clear_cache: bool,
    /// Devices, volumes or voice settings changed: apply to the audio engine.
    pub audio_changed: bool,
    /// List the audio devices again.
    pub refresh_devices: bool,
    /// Every setting went back to its default.
    pub reset: bool,
}

/// Category tabs (index = `OptionsUi::tab`).
const CATEGORIES: [(&str, &str); 11] = [
    ("Graphismes", "Command_Environments_Icon"),
    ("Interface", "Command_Appearance_Icon"),
    ("Polices", "Command_Chat_Icon"),
    ("Chat", "nearbychat_18"),
    ("Contrôles", "Command_Move_Icon"),
    ("Couleurs", "drop"),
    ("Réseau et cache", "Command_Inventory_Icon"),
    ("Son et voix", "speaker-high"),
    ("Raccourcis", "keyboard"),
    ("Debug", "code"),
    ("Caméra", "camera"),
];

/// Order of the tabs in the sidebar.
const ORDER: [usize; 11] = [0, 1, 2, 3, 7, 4, TAB_CAMERA, 8, 5, 6, 9];

pub const TAB_AUDIO: usize = 7;
pub const TAB_CHAT: usize = 3;
pub const TAB_CAMERA: usize = 10;

const LABEL_W: f32 = 180.0;

fn category(ui: &mut egui::Ui, p: &Palette, icons: &Icons, label: &str, icon: &str, selected: bool) -> bool {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 28.0), Sense::click());
    let fill = if selected {
        p.violet.gamma_multiply(0.28)
    } else if resp.hovered() {
        p.raised
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, CornerRadius::same(2), fill);
    if selected {
        ui.painter()
            .rect_filled(egui::Rect::from_min_size(rect.min, Vec2::new(2.0, rect.height())), 0.0, p.violet);
    }
    if let Some(t) = icons.get(icon) {
        let ir = egui::Rect::from_center_size(rect.left_center() + Vec2::new(18.0, 0.0), Vec2::splat(16.0));
        ui.painter().image(
            t.id(),
            ir,
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            if selected { p.ink } else { p.muted },
        );
    }
    ui.painter().text(
        rect.left_center() + Vec2::new(34.0, 0.0),
        egui::Align2::LEFT_CENTER,
        label,
        egui::FontId::proportional(13.0),
        if selected { p.ink } else { p.muted },
    );
    resp.clicked()
}

/// A titled group of settings drawn as a flat card.
/// Phosphor icon of a settings group (by its title).
fn group_icon(title: &str) -> Option<&'static str> {
    Some(match title {
        "Qualité" => "sliders",
        "Lumière" => "sun",
        "Reflets" => "drop",
        "Image" => "image",
        "Fluidité" => "speedometer",
        "Disposition" => "layout",
        "Skin" => "palette",
        "Taille et aperçu" => "text-aa",
        "Polices intégrées" => "text-t",
        "Polices du système" => "monitor",
        "Chat local" => "chat-circle-dots",
        "Messages" => "chat-text",
        "Émetteur" => "user-circle",
        "Mentions dans le chat" => "at",
        "Souris" => "mouse",
        "Clavier" => "keyboard",
        "Couleur des étiquettes" => "tag",
        "Repères sur la mini-carte" => "map-trifold",
        "Cache disque" => "hard-drives",
        "Périphériques" => "speaker-hifi",
        "Volumes" => "speaker-high",
        "Voix" => "waveform",
        "Push-to-talk" => "microphone",
        "Déplacements" => "person-simple-walk",
        "Caméra" | "Point de vue" => "camera",
        "Mouvements de la caméra" => "sliders",
        "Comportement" => "arrows-clockwise",
        "Limites" => "prohibit",
        "Vue subjective" => "crosshair",
        "Regard" => "eye",
        "Communication" => "chats-circle",
        "Groupes et blocage" => "prohibit",
        "Réponses automatiques" => "paper-plane-tilt",
        "Refus automatiques" => "hand-waving",
        "Fenêtres" => "squares-four",
        _ => return None,
    })
}

fn group(ui: &mut egui::Ui, p: &Palette, title: &str, body: impl FnOnce(&mut egui::Ui)) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 5.0;
        if let Some(t) = group_icon(title).and_then(super::icons::global) {
            let (r, _) = ui.allocate_exact_size(Vec2::splat(14.0), Sense::hover());
            ui.painter().image(
                t.id(),
                r,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                p.violet_light,
            );
        }
        ui.label(RichText::new(title.to_uppercase()).size(11.0).strong().color(p.violet_light));
    });
    ui.add_space(2.0);
    egui::Frame::new()
        .fill(p.field)
        .corner_radius(CornerRadius::same(2))
        .inner_margin(egui::Margin::symmetric(10, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 6.0;
            body(ui);
        });
    ui.add_space(10.0);
}

/// One row: label (with optional hint) on the left, control on the right.
fn row(ui: &mut egui::Ui, p: &Palette, label: &str, hint: &str, control: impl FnOnce(&mut egui::Ui) -> bool) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        let (rect, resp) = ui.allocate_exact_size(Vec2::new(LABEL_W, 22.0), Sense::hover());
        ui.painter().text(
            rect.left_center(),
            egui::Align2::LEFT_CENTER,
            label,
            egui::FontId::proportional(13.0),
            p.ink,
        );
        if !hint.is_empty() {
            resp.on_hover_text(hint);
        }
        changed = control(ui);
    });
    changed
}

const SLIDER_W: f32 = 230.0;

fn slider<T: egui::emath::Numeric>(ui: &mut egui::Ui, v: &mut T, range: std::ops::RangeInclusive<T>, suffix: &str) -> bool {
    ui.spacing_mut().slider_width = SLIDER_W;
    ui.add(egui::Slider::new(v, range).suffix(suffix).trailing_fill(true)).changed()
}

/// Stepped slider with a named level shown next to it.
fn stepped(ui: &mut egui::Ui, p: &Palette, v: &mut u8, labels: &[&str]) -> bool {
    ui.spacing_mut().slider_width = SLIDER_W;
    let max = labels.len().saturating_sub(1) as u8;
    let r = ui.add(egui::Slider::new(v, 0..=max).step_by(1.0).trailing_fill(true).show_value(false));
    let label = labels.get((*v).min(max) as usize).copied().unwrap_or("");
    ui.label(RichText::new(label).size(12.0).color(p.violet_pale));
    r.changed()
}

/// Color swatch with a picker popup (opaque).
fn swatch(ui: &mut egui::Ui, v: &mut crate::settings::Rgba) -> bool {
    let mut col = Color32::from_rgba_unmultiplied(v[0], v[1], v[2], v[3]);
    ui.spacing_mut().interact_size = Vec2::new(38.0, 18.0);
    let changed = egui::widgets::color_picker::color_edit_button_srgba(ui, &mut col, egui::widgets::color_picker::Alpha::Opaque).changed();
    if changed {
        *v = col.to_array();
    }
    changed
}

/// Grid of labelled swatches, three per row (Firestorm color preferences).
fn swatches(ui: &mut egui::Ui, id: &str, items: &mut [(&str, &mut crate::settings::Rgba)]) -> bool {
    let mut changed = false;
    egui::Grid::new(id).num_columns(3).spacing([24.0, 8.0]).show(ui, |ui| {
        for (i, (label, v)) in items.iter_mut().enumerate() {
            ui.horizontal(|ui| {
                ui.set_min_width(150.0);
                changed |= swatch(ui, v);
                ui.label(RichText::new(*label).size(12.5));
            });
            if i % 3 == 2 {
                ui.end_row();
            }
        }
    });
    changed
}

fn format_bytes(b: u64) -> String {
    let mb = b as f64 / (1024.0 * 1024.0);
    if mb >= 1024.0 {
        format!("{:.1} Go", mb / 1024.0)
    } else {
        format!("{mb:.0} Mo")
    }
}

/// Debug overlays (not saved: off again at the next launch).
fn debug_page(ui: &mut egui::Ui, p: &Palette, s: &mut Settings) -> bool {
    let mut c = false;
    let d = &mut s.debug;
    ui.label(
        RichText::new("Outils de diagnostic affichés par-dessus la vue ; ils se coupent au prochain lancement.")
            .size(12.0)
            .color(p.muted),
    );
    ui.add_space(6.0);
    group(ui, p, "Avatars", |ui| {
        c |= row(
            ui,
            p,
            "Complexité dans les étiquettes",
            "Sous le nom, du vert au rouge selon la limite de complexité (comme Firestorm) ; gris pour soi",
            |ui| toggle(ui, p, &mut d.complexity_tags),
        );
        c |= row(ui, p, "Squelettes", "Os des avatars proches (32 m), après animation", |ui| {
            toggle(ui, p, &mut d.skeletons)
        });
    });
    group(ui, p, "Rendu", |ui| {
        c |= row(
            ui,
            p,
            "Afficher la transparence",
            "Ctrl+Alt+T : faces transparentes, masquées et invisibles en rouge, masques de matériaux en bleu (retrouver les objets invisibles)",
            |ui| toggle(ui, p, &mut d.show_alpha),
        );
        c |= row(
            ui,
            p,
            "… objets riggés aussi",
            "Inclut les corps, vêtements et objets portés riggés",
            |ui| toggle(ui, p, &mut d.show_alpha_rigged),
        );
        c |= row(ui, p, "Fil de fer", "Arêtes des triangles par-dessus la scène", |ui| {
            toggle(ui, p, &mut d.wireframe)
        });
        c |= row(
            ui,
            p,
            "Visualiser le glow",
            "Quantité de glow à la place de l'image (rouge = glow × 10, vert = × 2)",
            |ui| toggle(ui, p, &mut d.glow_view),
        );
        c |= row(
            ui,
            p,
            "Objets avec glow",
            "Encadre en orange les objets qui ont des faces avec glow",
            |ui| toggle(ui, p, &mut d.glow_objects),
        );
        c |= row(ui, p, "Lumières", "Position et rayon des lumières (dans leur couleur)", |ui| {
            toggle(ui, p, &mut d.lights)
        });
        c |= row(
            ui,
            p,
            "Sondes de réflexion",
            "Influence des sondes : turquoise automatiques, violet manuelles, gris pas encore capturées",
            |ui| toggle(ui, p, &mut d.probes),
        );
    });
    group(ui, p, "Objets et culling", |ui| {
        c |= row(ui, p, "Boîtes englobantes", "Boîte de chaque objet à moins de 96 m", |ui| {
            toggle(ui, p, &mut d.bounds)
        });
        c |= row(
            ui,
            p,
            "Culling",
            "Sphère testée de chaque objet : vert dessiné, rouge hors champ, orange trop loin, gris trop petit",
            |ui| toggle(ui, p, &mut d.culling),
        );
        c |= row(
            ui,
            p,
            "Figer le culling",
            "Garde la vue de culling actuelle : déplacez la caméra pour voir ce qui n'est pas dessiné",
            |ui| toggle(ui, p, &mut d.freeze_culling),
        );
    });
    c
}

/// A label and a multi-line response text under it.
fn response_text(ui: &mut egui::Ui, p: &Palette, label: &str, text: &mut String) -> bool {
    ui.label(RichText::new(label).size(12.5).color(p.ink));
    ui.add(egui::TextEdit::multiline(text).desired_rows(2).desired_width(f32::INFINITY))
        .changed()
}

fn toggle(ui: &mut egui::Ui, p: &Palette, v: &mut bool) -> bool {
    super::widgets::switch(ui, p, v).changed()
}

pub fn show(ctx: &egui::Context, p: &Palette, icons: &Icons, s: &mut Settings, st: &mut OptionsUi, open: &mut bool) -> OptionsResult {
    let mut r = OptionsResult::default();
    let screen = ctx.content_rect();
    Floater::new(
        "options",
        "Préférences",
        egui::pos2(screen.center().x - 330.0, 80.0),
        Vec2::new(660.0, 470.0),
    )
    .help("Les changements sont appliqués immédiatement et enregistrés à la fermeture")
    .show(ctx, p, open, |ui| {
        let h = (ui.available_height() - 4.0).max(200.0);
        ui.horizontal_top(|ui| {
            // ---- categories
            ui.allocate_ui_with_layout(Vec2::new(150.0, h), egui::Layout::top_down(egui::Align::Min), |ui| {
                ui.set_min_height(h);
                ui.spacing_mut().item_spacing.y = 2.0;
                for i in ORDER {
                    let (label, icon) = CATEGORIES[i];
                    if category(ui, p, icons, label, icon, st.tab == i) {
                        st.capture = None;
                        st.tab = i;
                    }
                }
                // reset everything, at the bottom of the column
                ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
                    ui.add_space(4.0);
                    let b = ui.add(egui::Button::new(RichText::new("Tout réinitialiser…").size(12.0).color(p.muted)).frame(false));
                    if b.on_hover_text("Remettre tous les réglages par défaut").clicked() {
                        st.confirm_reset = true;
                    }
                });
            });
            ui.add_space(6.0);
            // ---- content
            ui.vertical(|ui| {
                ui.label(
                    RichText::new(CATEGORIES[st.tab.min(CATEGORIES.len() - 1)].0)
                        .size(17.0)
                        .color(p.ink),
                );
                ui.add_space(6.0);
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .max_height(h - 34.0)
                    .min_scrolled_height(h - 34.0)
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width() - 8.0);
                        r.changed |= match st.tab {
                            TAB_AUDIO => audio_page(ui, p, s, st, &mut r.audio_changed, &mut r.refresh_devices),
                            8 => keys_page(ui, p, s, st),
                            9 => debug_page(ui, p, s),
                            TAB_CAMERA => camera::page(ui, p, s),
                            _ => content(ui, p, s, st, &mut r.font_changed, &mut r.clear_cache),
                        };
                    });
            });
        });
    });
    if st.confirm_reset && *open {
        let modal = egui::Modal::new(egui::Id::new("options_reset")).show(ctx, |ui| {
            ui.set_width(340.0);
            ui.label(RichText::new("Tout réinitialiser ?").size(15.0).strong().color(p.ink));
            ui.add_space(6.0);
            ui.label(
                RichText::new(
                    "Tous les réglages (graphismes, interface, polices, chat, son et voix, contrôles, \
                     raccourcis, couleurs, cache) reprennent leur valeur par défaut. \
                     Le nom d'utilisateur, la grille et la taille de la fenêtre sont conservés.",
                )
                .size(12.5)
                .color(p.muted),
            );
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                let yes = ui.add(
                    egui::Button::new(RichText::new("Réinitialiser").color(Color32::WHITE))
                        .fill(p.danger)
                        .corner_radius(CornerRadius::same(2)),
                );
                if yes.clicked() {
                    s.reset_keeping_account();
                    st.capture = None;
                    r.changed = true;
                    r.font_changed = true;
                    r.audio_changed = true;
                    r.reset = true;
                    st.confirm_reset = false;
                }
                if super::widgets::flat_button(ui, p, "Annuler").clicked() {
                    st.confirm_reset = false;
                }
            });
        });
        if modal.should_close() {
            st.confirm_reset = false;
        }
    }
    r
}

fn content(ui: &mut egui::Ui, p: &Palette, s: &mut Settings, st: &mut OptionsUi, font_changed: &mut bool, clear_cache: &mut bool) -> bool {
    let mut c = false;
    match st.tab {
        0 => {
            group(ui, p, "Qualité", |ui| {
                c |= row(
                    ui,
                    p,
                    "Distance d'affichage",
                    "Au-delà, les objets ne sont pas dessinés (96 m par défaut)",
                    |ui| slider(ui, &mut s.draw_distance, 32.0..=512.0, " m"),
                );
                c |= row(
                    ui,
                    p,
                    "Niveau de détail (LOD)",
                    "Facteur LOD des objets et mesh de SL (2 par défaut, 4 = max)",
                    |ui| {
                        ui.spacing_mut().slider_width = SLIDER_W;
                        ui.add(egui::Slider::new(&mut s.lod_factor, 0.5..=4.0).step_by(0.125).trailing_fill(true))
                            .changed()
                    },
                );
                c |= row(
                    ui,
                    p,
                    "Avatars en entier",
                    "Les plus proches sont dessinés en entier, les autres en imposteurs : une image d'eux rafraîchie de temps en temps (RenderAvatarMaxNonImpostors, comme Firestorm)",
                    |ui| slider(ui, &mut s.max_avatars, 1..=100, ""),
                );
                c |= row(
                    ui,
                    p,
                    "Complexité max. des avatars",
                    "Au-delà, un avatar s'affiche en silhouette grise sans ses objets (comme Firestorm ; tout à droite : sans limite)",
                    |ui| {
                        use crate::scene::complexity::slider;
                        ui.spacing_mut().slider_width = SLIDER_W;
                        let mut pos = slider::from_limit(s.max_complexity);
                        let text = |v: f64| {
                            let limit = slider::to_limit(v.round() as u32);
                            if limit == 0 {
                                "Sans limite".to_owned()
                            } else {
                                format!("{} k", limit / 1000)
                            }
                        };
                        let r = ui.add(
                            egui::Slider::new(&mut pos, 1..=slider::OFF)
                                .custom_formatter(move |v, _| text(v))
                                .trailing_fill(true),
                        );
                        if r.changed() {
                            s.max_complexity = slider::to_limit(pos);
                        }
                        r.changed()
                    },
                );
                c |= row(
                    ui,
                    p,
                    "Surface max. des objets portés",
                    "Au-delà, l'avatar s'affiche aussi en silhouette (1000 m² comme Firestorm ; 0 = sans limite ; ignorée sans limite de complexité)",
                    |ui| {
                        ui.spacing_mut().slider_width = SLIDER_W;
                        ui.add(
                            egui::Slider::new(&mut s.max_attachment_area, 0.0..=10_000.0)
                                .logarithmic(true)
                                .step_by(10.0)
                                .custom_formatter(|v, _| if v < 1.0 { "Sans limite".to_owned() } else { format!("{v:.0} m²") })
                                .trailing_fill(true),
                        )
                        .changed()
                    },
                );
                c |= row(
                    ui,
                    p,
                    "Avatars en chargement",
                    "Nuage de particules jusqu'à ce que l'avatar et ses objets soient chargés (comme Firestorm), ou affichés au fur et à mesure ; barre de progression sous le nom",
                    |ui| stepped(ui, p, &mut s.loading_avatars, &["Nuage + barre", "Barre seule", "Rien"]),
                );
                c |= row(
                    ui,
                    p,
                    "Limite appliquée",
                    "Les amis peuvent toujours être affichés en entier, ou seuls les amis le sont (RenderAvatarComplexityMode)",
                    |ui| stepped(ui, p, &mut s.complexity_mode, &["À tous", "Sauf aux amis", "Amis seulement"]),
                );
                c |= row(
                    ui,
                    p,
                    "Occlusion",
                    "Les objets cachés derrière d'autres (murs, sols) ne sont pas dessinés ; test sur la carte graphique",
                    |ui| {
                        let changed = toggle(ui, p, &mut s.gpu_occlusion);
                        super::widgets::warning_hint(
                            ui,
                            p,
                            "Expérimental : pour l'instant, l'occlusion fait perdre des images par seconde au lieu d'en gagner (désactivée par défaut).",
                        );
                        changed
                    },
                );
                c |= row(ui, p, "Particules", "Nombre maximal de particules (0 = désactivées)", |ui| {
                    ui.spacing_mut().slider_width = SLIDER_W;
                    let r = ui.add(egui::Slider::new(&mut s.max_particles, 0..=8192).step_by(256.0).trailing_fill(true));
                    r.changed()
                });
                c |= row(ui, p, "Mémoire textures auto", "Selon la mémoire vidéo détectée", |ui| {
                    toggle(ui, p, &mut s.texture_budget_auto)
                });
                if s.texture_budget_auto {
                    let (vram, integrated) = st.vram;
                    let budget = crate::settings::auto_texture_budget(vram, integrated);
                    let gpu = match vram {
                        Some(v) if !integrated => format!("{} Mo de mémoire vidéo détectés", v),
                        _ if integrated => "Carte graphique intégrée".to_owned(),
                        _ => "Mémoire vidéo non détectée".to_owned(),
                    };
                    row(ui, p, "Mémoire textures", &gpu, |ui| {
                        ui.label(RichText::new(format!("{budget} Mo")).color(p.ink));
                        false
                    });
                } else {
                    c |= row(
                        ui,
                        p,
                        "Mémoire textures",
                        "Au-delà, les textures lointaines sont réduites",
                        |ui| {
                            ui.spacing_mut().slider_width = SLIDER_W;
                            ui.add(
                                egui::Slider::new(&mut s.texture_budget_mb, 256..=32768)
                                    .logarithmic(true)
                                    .suffix(" Mo")
                                    .trailing_fill(true),
                            )
                            .changed()
                        },
                    );
                }
                c |= row(ui, p, "Filtrage anisotrope", "Textures nettes vues de biais", |ui| {
                    let label = |v: u16| if v <= 1 { "Désactivé".to_owned() } else { format!("{v}×") };
                    let mut changed = false;
                    egui::ComboBox::from_id_salt("aniso")
                        .width(SLIDER_W)
                        .selected_text(label(s.anisotropy))
                        .show_ui(ui, |ui| {
                            for v in [1u16, 2, 4, 8, 16] {
                                changed |= ui.selectable_value(&mut s.anisotropy, v, label(v)).changed();
                            }
                        });
                    changed
                });
            });
            group(ui, p, "Lumière", |ui| {
                c |= row(ui, p, "Qualité des ombres", "Résolution et distance des ombres en cascade", |ui| {
                    ui.spacing_mut().slider_width = SLIDER_W;
                    let r = ui.add(
                        egui::Slider::new(&mut s.shadow_quality, 0..=4)
                            .step_by(1.0)
                            .trailing_fill(true)
                            .show_value(false),
                    );
                    let label = ["Désactivées", "Basse", "Moyenne", "Haute", "Ultra"][s.shadow_quality.min(4) as usize];
                    ui.label(RichText::new(label).size(12.0).color(p.violet_pale));
                    s.shadows = s.shadow_quality > 0;
                    r.changed()
                });
                c |= row(ui, p, "Occlusion ambiante", "Assombrit les recoins et les contacts (SSAO)", |ui| {
                    stepped(ui, p, &mut s.ssao, &["Désactivée", "Basse", "Haute"])
                });
                c |= row(
                    ui,
                    p,
                    "Heure du jour",
                    "Environnement partagé = cycle EEP de la région / parcelle",
                    |ui| {
                        let names = super::bars::TIME_OF_DAY;
                        let mut changed = false;
                        egui::ComboBox::from_id_salt("tod")
                            .width(SLIDER_W)
                            .selected_text(names[s.time_of_day.min(4) as usize])
                            .show_ui(ui, |ui| {
                                for (i, n) in names.iter().enumerate() {
                                    changed |= ui.selectable_value(&mut s.time_of_day, i as u8, *n).changed();
                                }
                            });
                        changed
                    },
                );
                c |= row(ui, p, "Exposition", "", |ui| {
                    ui.spacing_mut().slider_width = SLIDER_W;
                    ui.add(egui::Slider::new(&mut s.exposure, 0.25..=4.0).logarithmic(true).trailing_fill(true))
                        .changed()
                });
                c |= row(
                    ui,
                    p,
                    "Netteté",
                    "Renforce les détails fins (AMD CAS, 0,4 comme Firestorm ; 0 = désactivée)",
                    |ui| {
                        ui.spacing_mut().slider_width = SLIDER_W;
                        ui.add(egui::Slider::new(&mut s.sharpen, 0.0..=1.0).step_by(0.05).trailing_fill(true))
                            .changed()
                    },
                );
                c |= row(ui, p, "Glow", "Halo lumineux des faces avec du glow (comme Firestorm)", |ui| {
                    toggle(ui, p, &mut s.glow)
                });
            });
            group(ui, p, "Reflets", |ui| {
                let levels = ["Désactivés", "Basse (¼)", "Moyenne (½)", "Haute (¾)", "Ultra (pleine)"];
                c |= row(
                    ui,
                    p,
                    "Reflets de l'eau",
                    "Reflet plan du décor ; Haute et Ultra coûtent cher",
                    |ui| stepped(ui, p, &mut s.water_reflections, &levels),
                );
                c |= row(
                    ui,
                    p,
                    "Miroirs",
                    "Sondes miroir de SL ; résolution du rendu (Basse par défaut)",
                    |ui| stepped(ui, p, &mut s.mirrors, &levels),
                );
                c |= row(
                    ui,
                    p,
                    "Reflets écran (SSR)",
                    "Surfaces brillantes ; coûteux, désactivé par défaut",
                    |ui| toggle(ui, p, &mut s.ssr),
                );
                c |= row(
                    ui,
                    p,
                    "Sondes de réflexion",
                    "Ce que reflètent les surfaces brillantes (comme Firestorm)",
                    |ui| {
                        stepped(
                            ui,
                            p,
                            &mut s.reflection_probes,
                            &["Ciel seul", "Manuelles", "Manuelles + terrain", "Scène complète"],
                        )
                    },
                );
                c |= row(ui, p, "Nombre de sondes", "Sondes actives autour de vous (mémoire vidéo)", |ui| {
                    ui.spacing_mut().slider_width = SLIDER_W;
                    ui.add(egui::Slider::new(&mut s.probe_count, 8..=64).step_by(8.0).trailing_fill(true))
                        .changed()
                });
            });
            group(ui, p, "Image", |ui| {
                c |= row(
                    ui,
                    p,
                    "Anticrénelage",
                    "MSAA : net mais coûteux · FXAA : léger · SMAA : léger et net · TAA : le plus lisse",
                    |ui| {
                        let names = ["Aucun", "FXAA", "MSAA 2×", "MSAA 4×", "MSAA 8×", "TAA", "SMAA"];
                        let mut changed = false;
                        egui::ComboBox::from_id_salt("aa")
                            .width(SLIDER_W)
                            .selected_text(names[(s.antialiasing as usize).min(names.len() - 1)])
                            .show_ui(ui, |ui| {
                                for (i, n) in names.iter().enumerate() {
                                    changed |= ui.selectable_value(&mut s.antialiasing, i as u8, *n).changed();
                                }
                            });
                        changed
                    },
                );
                c |= row(ui, p, "Tone mapping", "Khronos PBR Neutral est celui de Second Life", |ui| {
                    let names = ["Khronos PBR Neutral", "ACES"];
                    let mut changed = false;
                    egui::ComboBox::from_id_salt("tonemap")
                        .width(SLIDER_W)
                        .selected_text(names[s.tonemapper.min(1) as usize])
                        .show_ui(ui, |ui| {
                            changed |= ui.selectable_value(&mut s.tonemapper, 0, names[0]).changed();
                            changed |= ui.selectable_value(&mut s.tonemapper, 1, names[1]).changed();
                        });
                    changed
                });
            });
            group(ui, p, "Fluidité", |ui| {
                c |= row(
                    ui,
                    p,
                    "Synchronisation verticale",
                    "Aligne les images sur la fréquence de l'écran (pas de déchirement, un peu plus de latence). Désactivée par défaut",
                    |ui| toggle(ui, p, &mut s.vsync),
                );
                c |= row(
                    ui,
                    p,
                    "Limiter les images/s",
                    "Plafonne le nombre d'images par seconde (moins de chauffe et de bruit). Désactivé par défaut",
                    |ui| {
                        let mut ch = toggle(ui, p, &mut s.fps_cap);
                        ui.add_enabled_ui(s.fps_cap, |ui| {
                            ui.spacing_mut().slider_width = SLIDER_W - 50.0;
                            ch |= ui
                                .add(egui::Slider::new(&mut s.fps_limit, 10..=360).suffix(" img/s").trailing_fill(true))
                                .changed();
                        });
                        ch
                    },
                );
                c |= row(
                    ui,
                    p,
                    "Limiter hors focus",
                    "Quand la fenêtre n'est pas au premier plan, le viewer ralentit pour libérer la machine. Désactivé par défaut",
                    |ui| {
                        let mut ch = toggle(ui, p, &mut s.background_fps_cap);
                        ui.add_enabled_ui(s.background_fps_cap, |ui| {
                            ui.spacing_mut().slider_width = SLIDER_W - 50.0;
                            ch |= ui
                                .add(
                                    egui::Slider::new(&mut s.background_fps_limit, 1..=120)
                                        .suffix(" img/s")
                                        .trailing_fill(true),
                                )
                                .changed();
                        });
                        ch
                    },
                );
            });
        }
        1 => {
            group(ui, p, "Disposition", |ui| {
                c |= row(ui, p, "Échelle de l'interface", "", |ui| {
                    slider(ui, &mut s.ui_scale, 0.75..=2.0, "×")
                });
                c |= row(
                    ui,
                    p,
                    "Fond des chargements",
                    "Les écrans de chargement et de téléportation montrent votre dernière vue, floutée (prise en vous téléportant ou en quittant)",
                    |ui| toggle(ui, p, &mut s.loading_backdrop),
                );
                c |= row(ui, p, "Distance des noms", "0 = masquer les noms des avatars", |ui| {
                    slider(ui, &mut s.name_tag_distance, 0.0..=128.0, " m")
                });
                c |= row(
                    ui,
                    p,
                    "Complexité dans les étiquettes",
                    "Sous le nom, du vert au rouge selon la limite de complexité (comme Firestorm)",
                    |ui| toggle(ui, p, &mut s.tag_complexity),
                );
                if s.tag_complexity {
                    c |= row(
                        ui,
                        p,
                        "… seulement les silhouettes",
                        "Seulement pour les avatars au-delà de la limite (affichés en silhouette)",
                        |ui| toggle(ui, p, &mut s.tag_complexity_too_complex_only),
                    );
                    c |= row(ui, p, "… la mienne aussi", "Votre complexité sous votre nom", |ui| {
                        toggle(ui, p, &mut s.tag_complexity_own)
                    });
                }
                c |= row(ui, p, "Performances au démarrage", "", |ui| toggle(ui, p, &mut s.show_perf));
                c |= row(ui, p, "Mini-carte au démarrage", "", |ui| toggle(ui, p, &mut s.show_minimap));
            });
            group(ui, p, "Noms", |ui| {
                c |= row(
                    ui,
                    p,
                    "Noms d'affichage",
                    "Les noms choisis par les résidents (sinon les noms de compte)",
                    |ui| toggle(ui, p, &mut s.use_display_names),
                );
                if s.use_display_names {
                    c |= row(
                        ui,
                        p,
                        "… avec le nom d'utilisateur",
                        "Sous le nom dans les étiquettes, à côté dans le chat",
                        |ui| toggle(ui, p, &mut s.show_usernames),
                    );
                    c |= row(
                        ui,
                        p,
                        "… au format Prénom Nom",
                        "« Jane (Jane Doe) » plutôt que « Jane (jane.doe) »",
                        |ui| toggle(ui, p, &mut s.legacy_username_format),
                    );
                    c |= row(
                        ui,
                        p,
                        "Annoncer les changements de nom",
                        "Dans le chat, quand un résident change de nom d'affichage",
                        |ui| toggle(ui, p, &mut s.display_name_notices),
                    );
                }
                c |= row(ui, p, "Masquer « Resident »", "Nom de famille des comptes récents", |ui| {
                    toggle(ui, p, &mut s.trim_resident)
                });
            });
            group(ui, p, "Skin", |ui| {
                ui.label(
                    RichText::new("Couleurs, icônes et barre d'outils se personnalisent dans :")
                        .size(12.0)
                        .color(p.muted),
                );
                let dir = crate::settings::config_dir().join("skins").join("aurora");
                ui.label(RichText::new(dir.display().to_string()).size(12.0).monospace().color(p.teal));
                ui.label(
                    RichText::new("theme.json · layout.json · icons/  — rechargés à chaud")
                        .size(11.0)
                        .color(p.muted_dim),
                );
            });
        }
        2 => {
            group(ui, p, "Polices intégrées", |ui| {
                for builtin in [fonts::NOTO, fonts::INTER, fonts::ROBOTO] {
                    let sel = s.font == builtin || (s.font.is_empty() && builtin == fonts::NOTO);
                    let label = if builtin == fonts::NOTO {
                        format!("{builtin}  (par défaut)")
                    } else {
                        builtin.to_owned()
                    };
                    if ui.radio(sel, RichText::new(label).size(13.0)).clicked() && !sel {
                        s.font = builtin.to_owned();
                        *font_changed = true;
                        c = true;
                    }
                }
            });
            group(ui, p, "Polices du système", |ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut st.font_filter)
                        .hint_text("Rechercher une police")
                        .desired_width(ui.available_width()),
                );
                let list = st.system_fonts.get_or_insert_with(fonts::system_fonts);
                let filter = st.font_filter.to_lowercase();
                egui::ScrollArea::vertical().id_salt("sysfonts").max_height(150.0).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    for f in list.iter().filter(|f| filter.is_empty() || f.name.to_lowercase().contains(&filter)) {
                        let key = format!("system:{}", f.path.display());
                        let sel = s.font == key;
                        if ui.radio(sel, RichText::new(&f.name).size(13.0)).clicked() && !sel {
                            s.font = key;
                            *font_changed = true;
                            c = true;
                        }
                    }
                });
            });
            group(ui, p, "Taille et aperçu", |ui| {
                c |= row(ui, p, "Taille du texte", "", |ui| slider(ui, &mut s.font_scale, 0.8..=1.4, "×"));
                ui.add_space(4.0);
                ui.label(RichText::new("Le loup violet hurle sous l'aurore boréale.").size(15.0).color(p.ink));
                ui.label(
                    RichText::new("0123456789 · éèàçù · ÆŒ · « guillemets » · L$ 1 250")
                        .size(12.0)
                        .color(p.muted),
                );
            });
        }
        3 => {
            group(ui, p, "Chat local", |ui| {
                c |= row(ui, p, "Afficher l'heure", "", |ui| toggle(ui, p, &mut s.chat_timestamps));
                c |= row(ui, p, "Durée des bulles", "0 = pas de bulles au-dessus de la barre de chat", |ui| {
                    slider(ui, &mut s.chat_toast_seconds, 0.0..=60.0, " s")
                });
                c |= row(ui, p, "Conversations au démarrage", "", |ui| toggle(ui, p, &mut s.show_chat));
            });
            group(ui, p, "Groupes et blocage", |ui| {
                c |= row(
                    ui,
                    p,
                    "Aucun chat de groupe",
                    "Refuser tous les chats de groupe (FSMuteAllGroups)",
                    |ui| toggle(ui, p, &mut s.mute_all_groups),
                );
                c |= row(
                    ui,
                    p,
                    "Groupes sans avis",
                    "Refuser le chat des groupes dont vous ne recevez pas les avis",
                    |ui| toggle(ui, p, &mut s.mute_groups_without_notices),
                );
                c |= row(
                    ui,
                    p,
                    "Signaler dans le chat",
                    "Annoncer dans le chat local les ajouts et retraits de la liste de blocage",
                    |ui| toggle(ui, p, &mut s.report_blocks),
                );
            });
            // Firestorm: Préférences > Confidentialité > Réponses auto. 1 et 2
            let a = &mut s.autoresponse;
            group(ui, p, "Réponses automatiques", |ui| {
                ui.label(
                    RichText::new("Les modes s'activent dans Communiquer > Statut de connexion.")
                        .size(11.5)
                        .color(p.muted),
                );
                c |= response_text(ui, p, "En mode « Ne pas déranger » :", &mut a.dnd_text);
                c |= response_text(ui, p, "À tous, en mode « Réponse automatique » :", &mut a.autorespond_text);
                c |= response_text(ui, p, "En mode « Réponse automatique aux non-amis » :", &mut a.nonfriends_text);
                c |= row(
                    ui,
                    p,
                    "Répondre si absent",
                    "Envoyer le texte ci-dessous quand je suis absent",
                    |ui| toggle(ui, p, &mut a.send_away),
                );
                c |= response_text(ui, p, "Quand je suis absent :", &mut a.away_text);
                c |= row(
                    ui,
                    p,
                    "Répondre aux bloqués",
                    "Envoyer le texte ci-dessous aux personnes bloquées (leur IM reste masqué)",
                    |ui| toggle(ui, p, &mut a.send_muted),
                );
                c |= response_text(ui, p, "Aux personnes bloquées :", &mut a.muted_text);
                c |= row(
                    ui,
                    p,
                    "Absent après",
                    "Passer en absent après cette durée sans clavier ni souris (0 = jamais)",
                    |ui| slider(ui, &mut a.away_after_minutes, 0..=60, " min"),
                );
            });
            group(ui, p, "Refus automatiques", |ui| {
                c |= response_text(ui, p, "En mode « Rejeter les téléportations » :", &mut a.reject_teleports_text);
                c |= row(
                    ui,
                    p,
                    "Sauf mes amis",
                    "Ne pas rejeter les téléportations de mes amis ni leur répondre",
                    |ui| toggle(ui, p, &mut a.dont_reject_friends_teleports),
                );
                c |= response_text(ui, p, "En mode « Rejeter les demandes d'amitié » :", &mut a.reject_friendship_text);
            });
        }
        4 => {
            group(ui, p, "Souris", |ui| {
                c |= row(ui, p, "Sensibilité", "", |ui| slider(ui, &mut s.mouse_sensitivity, 0.1..=5.0, "×"));
                c |= row(ui, p, "Inverser l'axe vertical", "", |ui| toggle(ui, p, &mut s.invert_mouse));
            });
            group(ui, p, "Clavier", |ui| {
                c |= row(
                    ui,
                    p,
                    "Flèches : pas latéraux",
                    "Gauche/droite déplacent latéralement au lieu de tourner (Maj inverse)",
                    |ui| toggle(ui, p, &mut s.arrows_strafe),
                );
                row(ui, p, "Raccourcis", "Touches et boutons de souris de chaque action", |ui| {
                    if super::widgets::flat_button(ui, p, "Modifier les raccourcis…").clicked() {
                        st.tab = 8;
                    }
                    false
                });
            });
            group(ui, p, "Regard", |ui| {
                c |= row(
                    ui,
                    p,
                    "Suivi du regard",
                    "La tête et les yeux de votre avatar suivent le curseur et ce que vous visez, et les autres voient où vous regardez",
                    |ui| toggle(ui, p, &mut s.look_at),
                );
                if s.look_at {
                    c |= row(
                        ui,
                        p,
                        "Portée du regard",
                        "Une cible plus lointaine est ramenée à cette distance de la tête : les autres ne voient pas ce que vous regardez au loin",
                        |ui| {
                            let unlimited = s.look_at_range >= crate::settings::LOOK_AT_UNLIMITED;

                            slider(
                                ui,
                                &mut s.look_at_range,
                                1.0..=crate::settings::LOOK_AT_UNLIMITED,
                                if unlimited { " m (illimitée)" } else { " m" },
                            )
                        },
                    );
                }
            });
        }
        5 => {
            let k = &mut s.colors;
            group(ui, p, "Couleur des étiquettes", |ui| {
                c |= swatches(
                    ui,
                    "tags",
                    &mut [
                        ("Mon nom", &mut k.tag_me),
                        ("Amis", &mut k.tag_friend),
                        ("Ignorés", &mut k.tag_muted),
                        ("Linden Lab", &mut k.tag_linden),
                        ("Correspondances", &mut k.tag_match),
                        ("Non-correspondances", &mut k.tag_mismatch),
                    ],
                );
            });
            group(ui, p, "Repères sur la mini-carte", |ui| {
                c |= swatches(
                    ui,
                    "map",
                    &mut [
                        ("Moi", &mut k.map_me),
                        ("Autres", &mut k.map_other),
                        ("Amis", &mut k.map_friend),
                        ("Linden Lab", &mut k.map_linden),
                        ("Ignorés", &mut k.map_muted),
                    ],
                );
                ui.add_space(4.0);
                c |= ui
                    .checkbox(
                        &mut k.map_ranges,
                        RichText::new("Afficher les portées de murmure, voix normale et cri sur la mini-carte")
                            .size(12.5)
                            .color(p.ink),
                    )
                    .changed();
                ui.add_enabled_ui(k.map_ranges, |ui| {
                    egui::Grid::new("ranges").num_columns(3).spacing([24.0, 6.0]).show(ui, |ui| {
                        for (label, on, col) in [
                            ("Murmure", &mut k.map_whisper_on, &mut k.map_whisper),
                            ("Voix normale", &mut k.map_say_on, &mut k.map_say),
                            ("Cri", &mut k.map_shout_on, &mut k.map_shout),
                        ] {
                            ui.horizontal(|ui| {
                                c |= ui.checkbox(on, "").changed();
                                c |= swatch(ui, col);
                                ui.label(RichText::new(label).size(12.5).color(p.ink));
                            });
                        }
                        ui.end_row();
                    });
                });
            });
            group(ui, p, "Messages", |ui| {
                c |= swatches(
                    ui,
                    "msgs",
                    &mut [
                        ("Mes messages", &mut k.chat_mine),
                        ("Autres", &mut k.chat_others),
                        ("Objets", &mut k.chat_objects),
                        ("Amis", &mut k.chat_friends),
                        ("Linden Lab", &mut k.chat_linden),
                        ("Ignorés", &mut k.chat_muted),
                        ("Système", &mut k.chat_system),
                        ("Erreurs", &mut k.chat_errors),
                        ("IMs des objets", &mut k.chat_object_im),
                        ("Propriétaire", &mut k.chat_owner),
                        ("URLs", &mut k.chat_urls),
                        ("Chemin d'URL", &mut k.chat_slurl),
                        ("Direct", &mut k.chat_direct),
                    ],
                );
            });
            group(ui, p, "Émetteur", |ui| {
                c |= swatches(
                    ui,
                    "sender",
                    &mut [("Avatars", &mut k.sender_avatar), ("Objets", &mut k.sender_object)],
                );
            });
            group(ui, p, "Mentions dans le chat", |ui| {
                c |= swatches(
                    ui,
                    "mentions",
                    &mut [
                        ("Police", &mut k.mention_text),
                        ("Résidents", &mut k.mention_residents),
                        ("Moi", &mut k.mention_me),
                    ],
                );
            });
            ui.add_space(6.0);
            if ui.button(RichText::new("Couleurs par défaut").size(12.5)).clicked() {
                *k = crate::settings::ColorSettings::default();
                c = true;
            }
        }
        _ => {
            group(ui, p, "Cache disque", |ui| {
                c |= row(
                    ui,
                    p,
                    "Taille maximale",
                    "Textures, mesh, animations (partagés entre régions), objets par région, inventaire",
                    |ui| {
                        ui.spacing_mut().slider_width = SLIDER_W;
                        ui.add(
                            egui::Slider::new(&mut s.cache_size_mb, 512..=65536)
                                .logarithmic(true)
                                .suffix(" Mo")
                                .trailing_fill(true),
                        )
                        .changed()
                    },
                );
                row(
                    ui,
                    p,
                    "Utilisation",
                    "Les fichiers les moins récemment utilisés sont supprimés au-delà de la limite",
                    |ui| {
                        let used = st.cache_usage;
                        let limit = s.cache_size_mb as u64 * 1024 * 1024;
                        let frac = (used as f32 / limit.max(1) as f32).clamp(0.0, 1.0);
                        ui.add(egui::ProgressBar::new(frac).desired_width(SLIDER_W).fill(p.violet).text(format!(
                            "{} / {}",
                            format_bytes(used),
                            format_bytes(limit)
                        )));
                        false
                    },
                );
                row(ui, p, "Vider le cache", "Tout sera retéléchargé au besoin", |ui| {
                    let label = if st.cache_clearing { "Suppression…" } else { "Vider le cache" };
                    let b = ui.add_enabled(!st.cache_clearing, egui::Button::new(RichText::new(label).color(p.ink)));
                    if b.clicked() {
                        *clear_cache = true;
                    }
                    false
                });
            });
            group(ui, p, "Journaux", |ui| {
                row(
                    ui,
                    p,
                    "Journal de la session",
                    "aurora.log (la session précédente : aurora.previous.log)",
                    |ui| {
                        if ui.button(RichText::new("Ouvrir le dossier").color(p.ink)).clicked() {
                            crate::logging::open_logs_dir();
                        }
                        false
                    },
                );
                row(
                    ui,
                    p,
                    "Objets proches",
                    "Écrit les faces, lumières et matériaux à moins de 12 m dans le journal",
                    |ui| {
                        if ui.button(RichText::new("Décrire").color(p.ink)).clicked() {
                            st.describe_nearby = true;
                        }
                        false
                    },
                );
            });
        }
    }
    c
}

/// Device combo: "" = system default.
fn device_combo(ui: &mut egui::Ui, id: &str, value: &mut String, devices: &[String]) -> bool {
    let mut changed = false;
    let shown = if value.is_empty() {
        "Par défaut du système".to_owned()
    } else {
        value.clone()
    };
    egui::ComboBox::from_id_salt(id)
        .width(SLIDER_W)
        .selected_text(shown)
        .truncate()
        .show_ui(ui, |ui| {
            changed |= ui.selectable_value(value, String::new(), "Par défaut du système").changed();
            for d in devices {
                changed |= ui.selectable_value(value, d.clone(), d).changed();
            }
            if !value.is_empty() && !devices.contains(value) {
                ui.label(RichText::new(format!("{value} (absent)")).size(12.0).weak());
            }
        });
    changed
}

/// "Son et voix": devices, the volumes of the top-right panel (same
/// settings, so both stay in sync), voice and microphone behaviour.
fn audio_page(ui: &mut egui::Ui, p: &Palette, s: &mut Settings, st: &mut OptionsUi, audio_changed: &mut bool, refresh: &mut bool) -> bool {
    let mut c = false;
    let a = &mut s.audio;
    group(ui, p, "Périphériques", |ui| {
        let out = row(ui, p, "Sortie audio", "Haut-parleurs ou casque utilisés par le viewer", |ui| {
            device_combo(ui, "audio_out", &mut a.output_device, &st.output_devices)
        });
        let inp = row(ui, p, "Microphone", "Entrée utilisée pour la voix", |ui| {
            device_combo(ui, "audio_in", &mut a.input_device, &st.input_devices)
        });
        c |= out | inp;
        *audio_changed |= out | inp;
        row(ui, p, "", "", |ui| {
            if super::widgets::flat_button(ui, p, "Actualiser la liste").clicked() {
                *refresh = true;
            }
            false
        });
    });
    group(ui, p, "Volumes", |ui| {
        for (i, name) in crate::settings::AUDIO_CHANNELS.iter().enumerate() {
            let changed = row(ui, p, name, "", |ui| {
                let mut ch = false;
                ui.spacing_mut().slider_width = SLIDER_W - 40.0;
                ch |= ui
                    .add_enabled(
                        !a.muted[i] && a.enabled[i],
                        egui::Slider::new(&mut a.volume[i], 0.0..=1.0).trailing_fill(true).show_value(false),
                    )
                    .changed();
                ui.label(RichText::new(format!("{:>3.0} %", a.volume[i] * 100.0)).size(12.0).color(p.muted));
                let mut on = !a.muted[i];
                if super::widgets::switch(ui, p, &mut on)
                    .on_hover_text(if on { "Couper" } else { "Rétablir" })
                    .changed()
                {
                    a.muted[i] = !on;
                    ch = true;
                }
                if i > 0 {
                    ch |= ui.checkbox(&mut a.enabled[i], "").on_hover_text("Activer ce canal").changed();
                }
                ch
            });
            c |= changed;
            *audio_changed |= changed;
        }
        c |= row(ui, p, "Musique auto.", "Lire la musique de la parcelle en y entrant", |ui| {
            toggle(ui, p, &mut a.music_autoplay)
        });
        c |= row(
            ui,
            p,
            "Sons de collision",
            "Chocs entre objets et avatars (comme Firestorm, activés par défaut)",
            |ui| toggle(ui, p, &mut a.collision_sounds),
        );
        c |= row(ui, p, "Sons des gestes", "Sons joués par les gestes des autres avatars", |ui| {
            toggle(ui, p, &mut a.gesture_sounds)
        });
        c |= row(
            ui,
            p,
            "Sons de l'interface",
            "Tous les sons de l'interface : clics, fenêtres, conversations, L$, téléportation, menus, alertes… (réglage par son plus bas)",
            |ui| toggle(ui, p, &mut a.ui_sounds),
        );
        c |= row(
            ui,
            p,
            "Média auto.",
            "Lire automatiquement le média de la parcelle (après 5 s immobile) et les médias sur prims réglés en lecture auto. (ParcelMediaAutoPlayEnable, désactivé par défaut)",
            |ui| toggle(ui, p, &mut a.media_autoplay),
        );
    });
    c |= ui_sounds_group(ui, p, a, st);
    group(ui, p, "Voix", |ui| {
        let v = row(ui, p, "Activer la voix", "Connexion au chat vocal (WebRTC) des régions", |ui| {
            toggle(ui, p, &mut a.voice_enabled)
        });
        c |= v;
        *audio_changed |= v;
        c |= row(
            ui,
            p,
            "Bouton du micro",
            "Basculer : un clic active, un clic coupe · Maintenir : parler tant que le bouton ou la touche est enfoncé",
            |ui| {
                let mut ch = false;
                ch |= ui.selectable_value(&mut a.mic_hold, false, "Basculer").changed();
                ch |= ui.selectable_value(&mut a.mic_hold, true, "Maintenir pour parler").changed();
                ch
            },
        );
        let g = row(ui, p, "Gain du micro", "", |ui| slider(ui, &mut a.mic_gain, 0.0..=2.0, "×"));
        c |= g;
        *audio_changed |= g;
        row(ui, p, "Niveau du micro", "Parlez pour tester", |ui| {
            let lvl = st.mic_level.clamp(0.0, 1.0);
            let col = if lvl > 0.85 {
                p.danger
            } else if lvl > 0.6 {
                p.amber
            } else {
                p.teal
            };
            ui.add(egui::ProgressBar::new(lvl).desired_width(SLIDER_W).desired_height(8.0).fill(col));
            false
        });
    });
    group(ui, p, "Push-to-talk", |ui| {
        let kb = s.keybinds.clone();
        row(
            ui,
            p,
            "Touche pour parler",
            "Touche ou bouton de souris (clic molette par défaut) ; aussi dans Raccourcis",
            |ui| {
                for slot in 0..2 {
                    binding_cell(ui, p, &kb, st, crate::keybinds::Action::PushToTalk, slot);
                }
                false
            },
        );
    });
    let m = &mut s.media;
    group(ui, p, "Média (web et vidéo)", |ui| {
        c |= row(
            ui,
            p,
            "Activer les médias",
            "Pages web et vidéos de la parcelle et des prims (AudioStreamingMedia)",
            |ui| toggle(ui, p, &mut m.enabled),
        );
        c |= row(
            ui,
            p,
            "Médias sur prims",
            "Médias partagés affichés sur les faces des objets (PrimMediaMasterEnabled)",
            |ui| toggle(ui, p, &mut m.prim_media),
        );
        c |= row(
            ui,
            p,
            "Sur les autres avatars",
            "Médias des objets portés par les autres (MediaShowOnOthers, désactivé par défaut)",
            |ui| toggle(ui, p, &mut m.show_on_others),
        );
        c |= row(
            ui,
            p,
            "Interaction au 1er clic",
            "Le premier clic va directement à la page quand l'objet l'autorise (MediaFirstClickInteract)",
            |ui| toggle(ui, p, &mut m.first_click_interact),
        );
        c |= row(
            ui,
            p,
            "Médias des HUD",
            "Lire automatiquement les médias des HUD (MediaAutoPlayHuds)",
            |ui| toggle(ui, p, &mut m.autoplay_huds),
        );
        c |= row(
            ui,
            p,
            "Médias par script",
            "Laisser les scripts démarrer le média de la parcelle (PermAllowScriptedMedia, désactivé par défaut comme Firestorm)",
            |ui| toggle(ui, p, &mut m.allow_scripted),
        );
        c |= row(
            ui,
            p,
            "Lecteurs simultanés",
            "Nombre maximal de pages / vidéos chargées en même temps (PluginInstancesTotal)",
            |ui| {
                ui.spacing_mut().slider_width = SLIDER_W - 40.0;
                let mut n = m.max_instances as u32;
                let ch = ui.add(egui::Slider::new(&mut n, 1..=16).trailing_fill(true)).changed();
                m.max_instances = n as usize;
                ch
            },
        );
        c |= row(
            ui,
            p,
            "Dossier des plugins",
            "Dossier contenant SLPlugin et llplugin\\ (vide : le viewer, puis Firestorm / Second Life installés)",
            |ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut m.plugin_dir)
                        .desired_width(SLIDER_W)
                        .hint_text("automatique"),
                )
                .changed()
            },
        );
        let status = if st.media_plugins.is_empty() {
            "Introuvables : installez Firestorm ou copiez son dossier llplugin et SLPlugin.exe à côté du viewer".to_string()
        } else {
            format!("{} · {} lecteur(s) actif(s)", st.media_plugins, st.media_running)
        };
        row(ui, p, "Plugins", "", |ui| {
            ui.label(RichText::new(status).size(12.0).color(p.muted));
            false
        });
    });
    if let Some((a, slot)) = st.clear.take() {
        s.keybinds.set(a, slot, None);
        c = true;
    }
    c
}

/// One binding button: click to capture, right click to clear.
fn binding_cell(
    ui: &mut egui::Ui,
    p: &Palette,
    kb: &crate::keybinds::KeyBindings,
    st: &mut OptionsUi,
    a: crate::keybinds::Action,
    slot: usize,
) {
    let capturing = st.capture == Some((a, slot));
    let b = kb.get(a)[slot].clone();
    let (text, col) = if capturing {
        ("Appuyez sur une touche…".to_owned(), Color32::WHITE)
    } else if let Some(b) = &b {
        (b.label(), p.ink)
    } else {
        ("—".to_owned(), p.muted_dim)
    };
    let fill = if capturing { p.violet } else { p.raised };
    let resp = ui.add(
        egui::Button::new(RichText::new(text).size(12.0).color(col))
            .fill(fill)
            .corner_radius(CornerRadius::same(2))
            .min_size(Vec2::new(132.0, 20.0)),
    );
    if resp.clicked() {
        st.capture = if capturing { None } else { Some((a, slot)) };
    }
    if resp.secondary_clicked() && b.is_some() {
        st.clear = Some((a, slot));
    }
    let others: Vec<&str> = b
        .as_ref()
        .map(|b| kb.users(b).into_iter().filter(|x| *x != a).map(crate::keybinds::label).collect())
        .unwrap_or_default();
    let tip = if capturing {
        "Touche, combinaison (Ctrl, Maj, Alt) ou bouton de souris (molette, côtés) · Échap ou clic : annuler".to_owned()
    } else if others.is_empty() {
        "Clic : changer · Clic droit : effacer".to_owned()
    } else {
        format!("Clic : changer · Clic droit : effacer\nAussi utilisé par : {}", others.join(", "))
    };
    // conflict marker
    if !capturing && !others.is_empty() {
        ui.painter()
            .circle_filled(resp.rect.right_top() + Vec2::new(-4.0, 4.0), 3.0, p.amber);
    }
    resp.on_hover_text(tip);
}

/// "Raccourcis": every rebindable action, two bindings each.
fn keys_page(ui: &mut egui::Ui, p: &Palette, s: &mut Settings, st: &mut OptionsUi) -> bool {
    let mut c = false;
    ui.label(
        RichText::new("Cliquez sur un raccourci puis appuyez sur la touche ou le bouton de souris voulu. Clic droit pour effacer.")
            .size(12.0)
            .color(p.muted),
    );
    ui.add_space(6.0);
    let kb = s.keybinds.clone();
    for (section, actions) in crate::keybinds::SECTIONS {
        group(ui, p, section, |ui| {
            for (a, label) in actions.iter() {
                ui.horizontal(|ui| {
                    let (rect, _) = ui.allocate_exact_size(Vec2::new(LABEL_W, 22.0), Sense::hover());
                    ui.painter().text(
                        rect.left_center(),
                        egui::Align2::LEFT_CENTER,
                        *label,
                        egui::FontId::proportional(13.0),
                        p.ink,
                    );
                    for slot in 0..2 {
                        binding_cell(ui, p, &kb, st, *a, slot);
                    }
                });
            }
        });
    }
    if super::widgets::flat_button(ui, p, "Rétablir les raccourcis par défaut").clicked() {
        s.keybinds = Default::default();
        st.capture = None;
        c = true;
    }
    if let Some((a, slot)) = st.clear.take() {
        s.keybinds.set(a, slot, None);
        c = true;
    }
    c
}

/// Préférences › Son et voix › Sons de l'interface: each sound on / off, its
/// asset and a preview, the IM modes and the L$ threshold
/// (FSPanelPreferenceUISounds, fspanelpreferenceuisounds.cpp).
fn ui_sounds_group(ui: &mut egui::Ui, p: &Palette, a: &mut crate::settings::AudioSettings, st: &mut OptionsUi) -> bool {
    use crate::ui_sound::{CATALOG, ImSoundMode, UiSound};
    let mut c = false;
    group(ui, p, "Sons de l'interface", |ui| {
        ui.label(
            RichText::new(
                "Mêmes sons et mêmes choix par défaut que Firestorm. Ce sont des sons de Second Life : ils se chargent une fois connecté.",
            )
            .size(12.0)
            .color(p.muted),
        );
        ui.add_space(4.0);
        let u = &mut a.ui;
        let modes = [
            (
                "Messages privés",
                "PlayModeUISndNewIncomingIMSession",
                &mut u.im_mode,
                UiSound::NewIncomingImSession,
            ),
            (
                "Messages de groupe",
                "PlayModeUISndNewIncomingGroupIMSession",
                &mut u.group_mode,
                UiSound::NewIncomingGroupImSession,
            ),
            (
                "Conférences",
                "PlayModeUISndNewIncomingConfIMSession",
                &mut u.conf_mode,
                UiSound::NewIncomingConfImSession,
            ),
        ];
        let mut previews = Vec::new();
        for (label, hint, mode, sound) in modes {
            c |= row(ui, p, label, hint, |ui| {
                let mut ch = false;
                egui::ComboBox::from_id_salt(hint)
                    .width(SLIDER_W)
                    .selected_text(mode.label())
                    .show_ui(ui, |ui| {
                        for m in ImSoundMode::ALL {
                            ch |= ui.selectable_value(mode, m, m.label()).changed();
                        }
                    });
                if super::widgets::flat_button(ui, p, "Écouter").clicked() {
                    previews.push(sound);
                }
                ch
            });
        }
        c |= row(
            ui,
            p,
            "Seuil des sons L$",
            "Écart de solde au-delà duquel les sons L$ reçus / dépensés sont joués (UISndMoneyChangeThreshold)",
            |ui| {
                ui.add(
                    egui::DragValue::new(&mut u.money_threshold)
                        .range(0.0..=100_000.0)
                        .speed(1.0)
                        .suffix(" L$"),
                )
                .changed()
            },
        );
        egui::CollapsingHeader::new(RichText::new("Choix par son").size(13.0).color(p.ink))
            .id_salt("ui_sounds_each")
            .show(ui, |ui| {
                for e in CATALOG
                    .iter()
                    .filter(|e| !e.sound.caller_gated() || e.sound == UiSound::TrackerBeacon)
                {
                    let hint = format!("UISnd{} : asset du son (vide = aucun son) · PlayModeUISnd{}", e.name, e.name);
                    c |= row(ui, p, e.label, &hint, |ui| {
                        let mut on = u.plays(e.sound);
                        let mut ch = super::widgets::switch(ui, p, &mut on).changed();
                        if ch {
                            u.set_plays(e.sound, on);
                        }
                        let text = st.ui_sound_text.entry(e.name).or_insert_with(|| uuid_text(u.uuid(e.sound)));
                        let parsed = parse_uuid_text(text);
                        let edit = ui.add(
                            egui::TextEdit::singleline(text)
                                .desired_width(290.0)
                                .font(egui::TextStyle::Monospace)
                                .text_color(if parsed.is_some() { p.ink } else { p.danger })
                                .hint_text("aucun son"),
                        );
                        if edit.changed()
                            && let Some(id) = parse_uuid_text(text)
                        {
                            u.set_uuid(e.sound, id);
                            ch = true;
                        }
                        if super::widgets::flat_button(ui, p, "Écouter").clicked() {
                            previews.push(e.sound);
                        }
                        if u.uuid(e.sound) != e.uuid && super::widgets::flat_button(ui, p, "Défaut").clicked() {
                            u.set_uuid(e.sound, e.uuid);
                            *text = uuid_text(e.uuid);
                            ch = true;
                        }
                        ch
                    });
                }
            });
        // force_sound: heard even when switched off
        for s in previews {
            if let Some(id) = u.resolve(s, true) {
                super::sound_cues::preview(ui.ctx(), id);
            }
        }
    });
    c
}

fn uuid_text(id: uuid::Uuid) -> String {
    if id.is_nil() { String::new() } else { id.to_string() }
}

/// A typed sound asset: a UUID, or empty for no sound.
fn parse_uuid_text(text: &str) -> Option<uuid::Uuid> {
    let t = text.trim();
    if t.is_empty() {
        Some(uuid::Uuid::nil())
    } else {
        uuid::Uuid::parse_str(t).ok()
    }
}
