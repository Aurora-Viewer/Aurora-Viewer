//! Grid manager, following LLPanelPreferenceOpensim (indra/newview/llfloaterpreference.cpp).
//! Stored entries and account selection are separate; connected sessions cannot change grids.

use crate::grids::{self, Error, Grid};
use crate::settings::Settings;
use crate::theme::Palette;
use crate::ui::{context, widgets};
use egui::{Context, RichText, Vec2};

fn request_id() -> egui::Id {
    egui::Id::unique("open_grid_manager")
}

/// The carousel also reads raw wheel/key events, outside egui's modal routing.
pub fn is_open(ctx: &Context) -> bool {
    ctx.data(|data| {
        data.get_temp::<bool>(request_id()).unwrap_or(false)
            || data.get_temp::<bool>(egui::Id::unique("grid_manager_open")).unwrap_or(false)
    })
}

pub fn request_open(ctx: &Context) {
    ctx.data_mut(|data| data.insert_temp(request_id(), true));
    ctx.request_repaint();
}

/// Shared by the old login form and the account cards, retaining a manually typed URI.
pub fn selector(ui: &mut egui::Ui, catalogue: &[Grid], uri: &mut String) {
    let selected_uri = grids::service(uri).ok();
    let selected = match crate::login_accounts::grid(uri) {
        crate::settings::GridChoice::SecondLife => "Second Life",
        crate::settings::GridChoice::SecondLifeBeta => "Aditi",
        crate::settings::GridChoice::Custom => catalogue
            .iter()
            .find(|grid| selected_uri.is_some() && grids::service(&grid.login).ok() == selected_uri)
            .map_or(aurora_i18n::tr!("grids-custom"), |grid| grid.name.as_str()),
    }
    .to_owned();
    egui::ComboBox::from_id_salt(ui.scope_id().with("grid_catalogue"))
        .width(ui.available_width())
        .truncate()
        .selected_text(selected)
        .show_ui(ui, |ui| {
            ui.selectable_value(uri, aurora_net::login::SL_MAIN_GRID.to_owned(), "Second Life");
            ui.selectable_value(uri, aurora_net::login::SL_BETA_GRID.to_owned(), "Aditi");
            for grid in catalogue {
                ui.selectable_value(uri, grid.login.clone(), &grid.name);
            }
            if ui.button(aurora_i18n::tr!("grids-manage")).clicked() {
                request_open(ui.ctx());
            }
        });
}

#[derive(Default)]
pub struct Manager {
    pub open: bool,
    previous: Option<String>,
    draft: Grid,
    result: Option<crossbeam_channel::Receiver<Result<Grid, Error>>>,
    error: Option<Error>,
    remove: Option<(String, String)>,
    tab: usize,
}

impl Manager {
    pub fn demo(&mut self, settings: &mut Settings) {
        let grid = grids::parse_info(
            "http://grid.example:8002",
            br#"<gridinfo>
            <gridname>Aurora Grid</gridname><gridnick>aurora</gridnick>
            <login>http://grid.example:8002/</login><gatekeeper>http://gate.example:8002/</gatekeeper>
            <welcome>https://grid.example/welcome</welcome><register>https://grid.example/register</register>
            <password>https://grid.example/password</password><help>https://grid.example/help</help>
            <about>https://grid.example/</about><search>https://grid.example/search?q=[QUERY]</search>
            </gridinfo>"#,
        );
        if let Ok(grid) = grid {
            self.previous = Some(grid.service.clone());
            self.draft = grid.clone();
            let _ = grids::store(settings, grid, None);
        }
        self.tab = usize::from(std::env::var("AURORA_DEMO_GRIDS").is_ok_and(|mode| mode == "pages"));
        self.open = true;
    }

    pub fn show(&mut self, ctx: &Context, p: &Palette, settings: &mut Settings, can_edit: bool, runtime: &tokio::runtime::Handle) {
        if ctx.data_mut(|data| data.remove_temp::<bool>(request_id())).unwrap_or(false) {
            self.open = true;
            if self.draft.service.is_empty() {
                self.draft = grids::active(settings)
                    .cloned()
                    .or_else(|| {
                        (settings.grid == crate::settings::GridChoice::Custom)
                            .then(|| grids::Grid::manual(&settings.custom_login_uri).ok())
                            .flatten()
                    })
                    .unwrap_or_default();
                self.previous = settings
                    .grids
                    .iter()
                    .find(|grid| grid.service == self.draft.service)
                    .map(|grid| grid.service.clone());
            }
        }
        if let Some(rx) = &self.result {
            match rx.try_recv() {
                Ok(result) => {
                    self.result = None;
                    match result {
                        Ok(grid) => {
                            self.draft = grid;
                            self.error = None;
                        }
                        Err(error) => self.error = Some(error),
                    }
                }
                Err(crossbeam_channel::TryRecvError::Disconnected) => {
                    self.result = None;
                    self.error = Some(Error::Network);
                }
                Err(crossbeam_channel::TryRecvError::Empty) => {}
            }
        }
        ctx.data_mut(|data| data.insert_temp(egui::Id::unique("grid_manager_open"), self.open));
        if !self.open {
            return;
        }
        // Like the account manager, this dialog owns input and uses login glass
        // over the account cards. Its editor scrolls independently of the list.
        let width = (ctx.content_rect().width() - 64.0).clamp(320.0, 860.0);
        let height = (ctx.content_rect().height() - 260.0).clamp(140.0, 390.0);
        let editable = can_edit && self.result.is_none();
        let mut close = false;
        let dialog = widgets::Modal::new(egui::Id::unique("grid_manager"))
            .login_surface(p)
            .stronger_tint();
        let response = dialog.show(ctx, |ui| {
            ui.set_width(width);
            ui.spacing_mut().item_spacing.y = 8.0;
            ui.horizontal(|ui| {
                widgets::glyph(ui, "world-filled", p.violet_light, 32.0);
                ui.vertical(|ui| {
                    ui.label(RichText::new(aurora_i18n::tr!("grids-manage")).size(21.0).strong().color(p.ink));
                    ui.label(RichText::new(aurora_i18n::tr!("grids-description")).size(12.0).color(p.muted));
                });
            });
            if !can_edit {
                ui.label(RichText::new(aurora_i18n::tr!("grids-logout-to-edit")).color(p.muted));
            }
            ui.add_space(4.0);
            ui.horizontal_top(|ui| {
                let sidebar = (width * 0.24).clamp(150.0, 210.0);
                ui.allocate_ui_with_layout(Vec2::new(sidebar, height + 38.0), egui::Layout::top_down(egui::Align::Min), |ui| {
                    ui.set_width(sidebar);
                    ui.label(RichText::new(aurora_i18n::tr!("grids-built-in")).size(11.5).color(p.muted));
                    for name in ["Second Life", "Aditi"] {
                        ui.add_enabled(false, egui::Button::new(name).min_size(Vec2::new(sidebar, 28.0)))
                            .on_hover_text(aurora_i18n::tr!("grids-error-protected"));
                    }
                    ui.add_space(8.0);
                    ui.label(RichText::new(aurora_i18n::tr!("grids-saved")).size(11.5).color(p.muted));
                    egui::ScrollArea::vertical()
                        .id_salt("grid_list")
                        .max_height((height - 153.0).max(40.0))
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            if settings.grids.is_empty() {
                                ui.label(RichText::new(aurora_i18n::tr!("grids-empty")).size(12.0).color(p.muted));
                            }
                            for grid in &settings.grids {
                                if ui
                                    .add_enabled_ui(self.result.is_none(), |ui| {
                                        ui.add_sized(
                                            Vec2::new(ui.available_width(), 32.0),
                                            egui::Button::selectable(self.previous.as_deref() == Some(&grid.service), &grid.name)
                                                .truncate(),
                                        )
                                    })
                                    .inner
                                    .on_hover_text(&grid.service)
                                    .clicked()
                                {
                                    self.draft = grid.clone();
                                    self.previous = Some(grid.service.clone());
                                    self.error = None;
                                    self.tab = 0;
                                }
                            }
                        });
                    if ui
                        .add_enabled_ui(editable, |ui| {
                            widgets::action_button(
                                ui,
                                p,
                                "plus",
                                aurora_i18n::tr!("grids-add"),
                                widgets::ActionKind::Flat,
                                Vec2::new(sidebar, 30.0),
                            )
                        })
                        .inner
                        .clicked()
                    {
                        self.draft = Grid::default();
                        self.previous = None;
                        self.error = None;
                        self.tab = 0;
                    }
                });
                ui.add_space(10.0);
                ui.vertical(|ui| {
                    ui.set_width((width - sidebar - 18.0).max(150.0));
                    widgets::segmented(
                        ui,
                        p,
                        &mut self.tab,
                        &[
                            (aurora_i18n::tr!("grids-connection"), true),
                            (aurora_i18n::tr!("grids-pages"), true),
                        ],
                    );
                    egui::ScrollArea::vertical()
                        .id_salt(("grid_fields", self.tab))
                        .max_height(height)
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            ui.spacing_mut().item_spacing.y = 10.0;
                            if self.tab == 0 {
                                ui.add_enabled_ui(editable, |ui| {
                                    edit(ui, p, aurora_i18n::tr!("grids-service"), &mut self.draft.service);
                                    if widgets::action_button(
                                        ui,
                                        p,
                                        "refresh",
                                        aurora_i18n::tr!("grids-fetch"),
                                        widgets::ActionKind::Flat,
                                        Vec2::new(ui.available_width(), 30.0),
                                    )
                                    .clicked()
                                    {
                                        self.error = None;
                                        let address = self.draft.service.clone();
                                        let (tx, rx) = crossbeam_channel::bounded(1);
                                        self.result = Some(rx);
                                        let ctx = ctx.clone();
                                        runtime.spawn(async move {
                                            let _ = tx.send(grids::discover(&address).await);
                                            ctx.request_repaint();
                                        });
                                    }
                                    ui.label(RichText::new(aurora_i18n::tr!("grids-manual-hint")).size(11.5).color(p.muted));
                                    ui.columns(2, |cols| {
                                        edit(&mut cols[0], p, aurora_i18n::tr!("common-name"), &mut self.draft.name);
                                        edit(&mut cols[1], p, aurora_i18n::tr!("grids-nickname"), &mut self.draft.nickname);
                                    });
                                    edit(ui, p, aurora_i18n::tr!("grids-login"), &mut self.draft.login);
                                    edit(ui, p, aurora_i18n::tr!("grids-gatekeeper"), &mut self.draft.gatekeeper);
                                });
                            } else {
                                web_edit(ui, p, aurora_i18n::tr!("grids-welcome"), &mut self.draft.welcome, editable);
                                web_edit(ui, p, aurora_i18n::tr!("grids-register"), &mut self.draft.register, editable);
                                web_edit(ui, p, aurora_i18n::tr!("grids-password"), &mut self.draft.password, editable);
                                web_edit(ui, p, aurora_i18n::tr!("grids-help"), &mut self.draft.help, editable);
                                web_edit(ui, p, aurora_i18n::tr!("grids-about"), &mut self.draft.about, editable);
                                ui.add_enabled_ui(editable, |ui| {
                                    edit(ui, p, aurora_i18n::tr!("grids-search"), &mut self.draft.search);
                                    edit(ui, p, aurora_i18n::tr!("grids-profile"), &mut self.draft.profile);
                                });
                            }
                        });
                });
            });
            if self.result.is_some() {
                ui.label(aurora_i18n::tr!("grids-fetching"));
            }
            if let Some(error) = self.error {
                ui.label(RichText::new(error.text()).color(p.danger));
            }
            widgets::divider(ui, p);
            ui.horizontal(|ui| {
                if ui
                    .add_enabled_ui(editable && self.previous.is_some(), |ui| {
                        widgets::action_button(
                            ui,
                            p,
                            "trash-filled",
                            aurora_i18n::tr!("common-remove"),
                            widgets::ActionKind::Danger,
                            Vec2::new(0.0, 30.0),
                        )
                    })
                    .inner
                    .clicked()
                {
                    self.remove = self.previous.clone().map(|service| (service, self.draft.name.clone()));
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .add_enabled_ui(editable, |ui| {
                            widgets::action_button(
                                ui,
                                p,
                                "device-floppy-filled",
                                aurora_i18n::tr!("common-save"),
                                widgets::ActionKind::Primary,
                                Vec2::new(0.0, 30.0),
                            )
                        })
                        .inner
                        .clicked()
                    {
                        let mut grid = self.draft.clone();
                        if grid.login.trim().is_empty() {
                            grid.login = grid.service.clone();
                        }
                        match grids::store(settings, grid, self.previous.as_deref()) {
                            Ok(()) => {
                                self.draft.service = grids::service(&self.draft.service).unwrap_or_default();
                                if let Some(grid) = settings.grids.iter().find(|grid| grid.service == self.draft.service) {
                                    self.draft = grid.clone();
                                }
                                self.previous = Some(self.draft.service.clone());
                                self.error = None;
                                settings.save();
                            }
                            Err(error) => self.error = Some(error),
                        }
                    }
                    if ui
                        .add_sized(Vec2::new(85.0, 30.0), egui::Button::new(aurora_i18n::tr!("common-close")))
                        .clicked()
                    {
                        close = true;
                    }
                });
            });
        });
        if close || response.should_close() && self.remove.is_none() {
            self.open = false;
        }
        if let Some((service, name)) = self.remove.clone() {
            let mut answer = None;
            let modal = widgets::Modal::new(egui::Id::unique("remove_grid")).show(ctx, |ui| {
                ui.set_width(360.0);
                ui.label(aurora_i18n::tr!("grids-remove-question", name = name));
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled_ui(can_edit, |ui| {
                            widgets::action_button(
                                ui,
                                p,
                                "trash-filled",
                                aurora_i18n::tr!("common-remove"),
                                widgets::ActionKind::Danger,
                                Vec2::new(0.0, 26.0),
                            )
                        })
                        .inner
                        .clicked()
                    {
                        answer = Some(true);
                    }
                    if ui.button(aurora_i18n::tr!("common-cancel")).clicked() {
                        answer = Some(false);
                    }
                });
            });
            if answer.is_some() || modal.should_close() {
                self.remove = None;
                if answer == Some(true) {
                    settings.grids.retain(|grid| grid.service != service);
                    self.previous = None;
                    self.draft = Grid::default();
                    self.error = None;
                    settings.save();
                }
            }
        }
        ctx.data_mut(|data| data.insert_temp(egui::Id::unique("grid_manager_open"), self.open));
    }
}

fn edit(ui: &mut egui::Ui, p: &Palette, label: &str, value: &mut String) {
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 4.0;
        ui.label(RichText::new(label).size(12.0).color(p.muted));
        ui.add_sized(
            Vec2::new(ui.available_width(), 28.0),
            egui::TextEdit::singleline(value).char_limit(2048),
        );
    });
}

fn web_edit(ui: &mut egui::Ui, p: &Palette, label: &str, value: &mut String, editable: bool) {
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 4.0;
        ui.label(RichText::new(label).size(12.0).color(p.muted));
        ui.horizontal(|ui| {
            ui.add_enabled_ui(editable, |ui| {
                ui.add_sized(
                    Vec2::new((ui.available_width() - 34.0).max(40.0), 28.0),
                    egui::TextEdit::singleline(value).char_limit(2048),
                )
            });
            if widgets::icon_button(ui, p, "external-link-filled", aurora_i18n::tr!("common-open"), !value.is_empty()) {
                context::request(ui.ctx(), context::CtxAction::OpenUrl(value.clone()));
            }
        });
    });
}
