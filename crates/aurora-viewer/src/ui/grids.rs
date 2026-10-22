//! Grid manager, following LLPanelPreferenceOpensim (indra/newview/llfloaterpreference.cpp).
//! Stored entries and account selection are separate; connected sessions cannot change grids.

use crate::grids::{self, Error, Grid};
use crate::settings::Settings;
use crate::theme::Palette;
use crate::ui::{context, widgets};
use egui::{Context, RichText, Vec2};

fn request_id() -> egui::Id {
    egui::Id::new("open_grid_manager")
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
            .map_or(aurora_i18n::tr!("ui-login-other-grid"), |grid| grid.name.as_str()),
    }
    .to_owned();
    egui::ComboBox::from_id_salt(ui.id().with("grid_catalogue"))
        .width(ui.available_width())
        .truncate()
        .selected_text(selected)
        .show_ui(ui, |ui| {
            ui.selectable_value(uri, aurora_net::login::SL_MAIN_GRID.to_owned(), "Second Life");
            ui.selectable_value(uri, aurora_net::login::SL_BETA_GRID.to_owned(), "Aditi");
            for grid in catalogue {
                ui.selectable_value(uri, grid.login.clone(), &grid.name);
            }
            if ui.selectable_label(false, aurora_i18n::tr!("ui-login-other-grid")).clicked() {
                uri.clear();
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
        if !self.open {
            return;
        }
        let mut open = self.open;
        widgets::Floater::new(
            "grid_manager",
            aurora_i18n::tr!("grids-manage"),
            egui::pos2(160.0, 80.0),
            Vec2::new(760.0, 640.0),
        )
        .show(ctx, p, &mut open, |ui| {
            if !can_edit {
                ui.label(RichText::new(aurora_i18n::tr!("grids-logout-to-edit")).color(p.muted));
            }
            ui.columns(2, |cols| {
                cols[0].strong(aurora_i18n::tr!("grids-list"));
                egui::ScrollArea::vertical()
                    .id_salt("grid_list")
                    .max_height(470.0)
                    .show(&mut cols[0], |ui| {
                        ui.label("Second Life");
                        ui.label("Aditi");
                        for grid in &settings.grids {
                            if ui
                                .add_enabled(
                                    self.result.is_none(),
                                    egui::Button::selectable(self.previous.as_deref() == Some(&grid.service), &grid.name),
                                )
                                .clicked()
                            {
                                self.draft = grid.clone();
                                self.previous = Some(grid.service.clone());
                                self.error = None;
                            }
                        }
                    });
                if cols[0]
                    .add_enabled(can_edit && self.result.is_none(), egui::Button::new(aurora_i18n::tr!("common-add")))
                    .clicked()
                {
                    self.draft = Grid::default();
                    self.previous = None;
                    self.error = None;
                }
                let ui = &mut cols[1];
                egui::ScrollArea::vertical()
                    .id_salt("grid_fields")
                    .max_height(470.0)
                    .show(ui, |ui| {
                        ui.add_enabled_ui(can_edit && self.result.is_none(), |ui| {
                            edit(ui, aurora_i18n::tr!("grids-service"), &mut self.draft.service);
                            if ui.button(aurora_i18n::tr!("grids-fetch")).clicked() {
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
                            edit(ui, aurora_i18n::tr!("common-name"), &mut self.draft.name);
                            edit(ui, aurora_i18n::tr!("grids-nickname"), &mut self.draft.nickname);
                            edit(ui, aurora_i18n::tr!("grids-login"), &mut self.draft.login);
                            edit(ui, aurora_i18n::tr!("grids-gatekeeper"), &mut self.draft.gatekeeper);
                            edit(ui, aurora_i18n::tr!("grids-welcome"), &mut self.draft.welcome);
                            edit(ui, aurora_i18n::tr!("grids-register"), &mut self.draft.register);
                            edit(ui, aurora_i18n::tr!("grids-password"), &mut self.draft.password);
                            edit(ui, aurora_i18n::tr!("grids-help"), &mut self.draft.help);
                            edit(ui, aurora_i18n::tr!("grids-about"), &mut self.draft.about);
                            edit(ui, aurora_i18n::tr!("grids-search"), &mut self.draft.search);
                            edit(ui, aurora_i18n::tr!("grids-profile"), &mut self.draft.profile);
                        });
                    });
            });
            if self.result.is_some() {
                ui.label(aurora_i18n::tr!("grids-fetching"));
            }
            if let Some(error) = self.error {
                ui.label(RichText::new(error.text()).color(p.danger));
            }
            ui.add_enabled_ui(can_edit && self.result.is_none(), |ui| {
                ui.horizontal_wrapped(|ui| {
                    if ui.button(aurora_i18n::tr!("common-save")).clicked() {
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
                        .add_enabled(self.previous.is_some(), egui::Button::new(aurora_i18n::tr!("common-remove")))
                        .clicked()
                    {
                        self.remove = self.previous.clone().map(|service| (service, self.draft.name.clone()));
                    }
                });
            });
            ui.horizontal_wrapped(|ui| {
                for (label, url) in [
                    (aurora_i18n::tr!("grids-welcome"), &self.draft.welcome),
                    (aurora_i18n::tr!("grids-register"), &self.draft.register),
                    (aurora_i18n::tr!("grids-password"), &self.draft.password),
                    (aurora_i18n::tr!("grids-help"), &self.draft.help),
                    (aurora_i18n::tr!("grids-about"), &self.draft.about),
                ] {
                    if !url.is_empty() && ui.button(label).clicked() {
                        context::request(ctx, context::CtxAction::OpenUrl(url.clone()));
                    }
                }
            });
        });
        self.open = open;
        if let Some((service, name)) = self.remove.clone() {
            let mut answer = None;
            let modal = widgets::Modal::new(egui::Id::new("remove_grid")).show(ctx, |ui| {
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
    }
}

fn edit(ui: &mut egui::Ui, label: &str, value: &mut String) {
    ui.label(label);
    ui.add(egui::TextEdit::singleline(value).desired_width(f32::INFINITY).char_limit(2048));
}
