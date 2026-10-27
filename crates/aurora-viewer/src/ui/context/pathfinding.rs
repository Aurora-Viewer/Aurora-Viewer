//! Pathfinding.Linksets.Select / Characters.Select inspection. The selected
//! linkset is revealed in a real RegionObjects / CharacterProperties reply.
//! Navmesh editing and terrain mutations remain separate from this reader.
use crate::{theme::Palette, ui::widgets, world::World};
use aurora_net::{
    NetCommand, RegionHandle,
    pathfinding::{Detail, Error, Kind, Row},
};
use uuid::Uuid;

#[derive(Default)]
pub struct PathfindingUi {
    open: bool,
    region: RegionHandle,
    kind: Option<Kind>,
    pending: Option<Uuid>,
    rows: Vec<Row>,
    selected: Option<Uuid>,
    selected_row: Option<usize>,
    query: String,
    filtered: Vec<usize>,
    error: Option<Error>,
    reveal: bool,
}

fn usage(category: u8, phantom: bool) -> &'static str {
    aurora_i18n::text(match (category, phantom) {
        (0, false) => "scene-nav-walkable",
        (0, true) => "scene-nav-material",
        (1, false) => "scene-nav-obstacle",
        (1, true) => "scene-nav-exclusion",
        (_, false) => "scene-nav-dynamic",
        (_, true) => "scene-nav-phantom",
    })
}

impl PathfindingUi {
    pub(crate) fn open(&mut self, region: RegionHandle, kind: Kind, selected: Uuid) -> NetCommand {
        *self = Self {
            open: true,
            region,
            kind: Some(kind),
            selected: (!selected.is_nil()).then_some(selected),
            ..Default::default()
        };
        self.refresh()
    }

    fn refresh(&mut self) -> NetCommand {
        let request = Uuid::new_v4();
        self.pending = Some(request);
        self.rows.clear();
        self.selected_row = None;
        self.filtered.clear();
        self.error = None;
        NetCommand::FetchPathfinding {
            request,
            region: self.region,
            kind: self.kind.unwrap_or(Kind::Objects),
        }
    }

    pub(crate) fn reply(&mut self, request: Uuid, region: RegionHandle, kind: Kind, result: Result<Vec<Row>, Error>) {
        if self.pending != Some(request) || self.region != region || self.kind != Some(kind) || !self.open {
            return;
        }
        self.pending = None;
        match result {
            Ok(rows) => {
                self.selected_row = rows.iter().position(|row| Some(row.id) == self.selected);
                self.rows = rows;
                self.filter();
                self.reveal = true;
            }
            Err(error) => self.error = Some(error),
        }
    }

    fn filter(&mut self) {
        use rayon::prelude::*;
        let query = self.query.trim().to_lowercase();
        self.filtered = self
            .rows
            .par_iter()
            .enumerate()
            .filter_map(|(i, row)| {
                (query.is_empty() || row.name.to_lowercase().contains(&query) || row.description.to_lowercase().contains(&query))
                    .then_some(i)
            })
            .collect();
    }

    pub(crate) fn show(&mut self, ctx: &egui::Context, p: &Palette, world: &mut World) -> Option<NetCommand> {
        if !self.open {
            return None;
        }
        let mut open = true;
        let mut refresh = false;
        let characters = self.kind == Some(Kind::Characters);
        let floater = widgets::Floater::new(
            "scene_pathfinding",
            aurora_i18n::text(if characters { "scene-nav-characters" } else { "scene-nav-objects" }),
            egui::pos2(220.0, 120.0),
            egui::vec2(720.0, 570.0),
        );
        floater.show(ctx, p, &mut open, |ui| {
            ui.label(world.regions.get(&self.region).map_or_else(String::new, |r| r.name.clone()));
            ui.horizontal(|ui| {
                if ui
                    .add(egui::TextEdit::singleline(&mut self.query).hint_text(aurora_i18n::tr!("common-search")))
                    .changed()
                {
                    self.filter();
                }
                refresh = ui
                    .add_enabled(self.pending.is_none(), egui::Button::new(aurora_i18n::tr!("area-search-refresh")))
                    .clicked();
                ui.label(aurora_i18n::integer(self.filtered.len() as i64));
            });
            if self.pending.is_some() {
                ui.label(aurora_i18n::tr!("common-loading"));
            }
            if let Some(error) = self.error {
                ui.label(aurora_i18n::text(match error {
                    Error::Unsupported => "scene-nav-unsupported",
                    Error::Failed => "scene-nav-failed",
                    Error::InvalidReply => "scene-nav-invalid",
                }));
            }
            if self.pending.is_none() && self.error.is_none() && self.selected.is_some() && self.selected_row.is_none() {
                ui.label(aurora_i18n::tr!("scene-nav-selected-missing"));
            }
            let mut scroll = egui::ScrollArea::vertical().max_height((ui.available_height() - 235.0).max(80.0));
            if self.reveal {
                if let Some(index) = self.filtered.iter().position(|&i| Some(self.rows[i].id) == self.selected) {
                    scroll = scroll.vertical_scroll_offset(index as f32 * 28.0);
                }
                self.reveal = false;
            }
            scroll.show_rows(ui, 24.0, self.filtered.len(), |ui, range| {
                for i in range {
                    let row = &self.rows[self.filtered[i]];
                    ui.push_id(row.id, |ui| {
                        let response = ui.add_sized(
                            egui::vec2(ui.available_width(), 24.0),
                            egui::Button::selectable(self.selected == Some(row.id), &row.name),
                        );
                        if response.clicked() {
                            self.selected = Some(row.id);
                            self.selected_row = Some(self.filtered[i]);
                        }
                        response.on_hover_text(&row.description);
                    });
                }
            });
            if let Some(row) = self.selected_row.and_then(|index| self.rows.get(index)) {
                widgets::divider(ui, p);
                ui.label(egui::RichText::new(&row.name).strong());
                ui.add(egui::Label::new(&row.description).truncate());
                if row.group_owned {
                    ui.label(
                        world
                            .groups
                            .group(&row.owner)
                            .map_or_else(|| aurora_i18n::tr!("common-group").into(), |group| group.name.clone()),
                    );
                } else {
                    super::object_profile::person(ui, p, world, row.owner);
                }
                ui.label(aurora_i18n::tr!(
                    "scene-nav-position",
                    x = format!("{:.1}", row.position[0]),
                    y = format!("{:.1}", row.position[1]),
                    z = format!("{:.1}", row.position[2])
                ));
                match &row.detail {
                    Detail::Object {
                        category,
                        phantom,
                        land_impact,
                        walkability,
                        modifiable,
                        scripted,
                    } => {
                        ui.label(aurora_i18n::tr!(
                            "scene-nav-object-detail",
                            usage = usage(*category, *phantom),
                            impact = *land_impact,
                            a = walkability[0],
                            b = walkability[1],
                            c = walkability[2],
                            d = walkability[3]
                        ));
                        ui.label(aurora_i18n::tr!(
                            "scene-nav-flags",
                            modify = aurora_i18n::text(if *modifiable { "common-yes" } else { "common-no" }),
                            scripts = aurora_i18n::text(if *scripted { "common-yes" } else { "common-no" })
                        ));
                    }
                    Detail::Character {
                        cpu_us,
                        length,
                        radius,
                        horizontal,
                    } => {
                        ui.label(aurora_i18n::tr!(
                            "scene-nav-character-detail",
                            cpu = format!("{cpu_us:.0}"),
                            length = format!("{length:.2}"),
                            radius = format!("{radius:.2}")
                        ));
                        ui.label(aurora_i18n::text(if *horizontal {
                            "scene-nav-horizontal"
                        } else {
                            "scene-nav-vertical"
                        }));
                    }
                }
                ui.horizontal(|ui| {
                    let visible = world.objects.index_of_uuid(&row.id).is_some();
                    if ui
                        .add_enabled(visible, egui::Button::new(aurora_i18n::tr!("ui-context-object-profile")))
                        .clicked()
                    {
                        super::request(ctx, super::CtxAction::ObjectProfile(row.id));
                    }
                    if ui
                        .add_enabled(visible, egui::Button::new(aurora_i18n::tr!("ui-context-inspect-2")))
                        .clicked()
                    {
                        super::request(ctx, super::CtxAction::Inspect(row.id));
                    }
                    if let Some((main_x, main_y)) = world.main_origin() {
                        let (x, y) = aurora_net::handle_to_origin(self.region);
                        let point = glam::Vec3::new(
                            (row.position[0] + f64::from(x) - f64::from(main_x)) as f32,
                            (row.position[1] + f64::from(y) - f64::from(main_y)) as f32,
                            row.position[2] as f32,
                        );
                        if ui
                            .add_enabled(point.is_finite(), egui::Button::new(aurora_i18n::tr!("common-zoom-2")))
                            .clicked()
                        {
                            super::request(ctx, super::CtxAction::Zoom(point, None));
                        }
                    }
                    if ui.button(aurora_i18n::tr!("build-ui-general-copy-uuid")).clicked() {
                        ctx.copy_text(row.id.to_string());
                    }
                });
            }
        });
        if !open {
            self.open = false;
            self.pending = None;
        }
        refresh.then(|| self.refresh())
    }

    pub(crate) fn demo(&mut self, world: &World, command: NetCommand) {
        let NetCommand::FetchPathfinding { request, region, kind } = command else {
            return;
        };
        let mut data = aurora_llsd::Llsd::new_map();
        let selected = self
            .selected
            .and_then(|id| world.objects.index_of_uuid(&id))
            .and_then(|index| world.objects.get(index).map(|object| (index, object)));
        for (_, object) in selected
            .into_iter()
            .chain(world.objects.iter().filter(|(_, object)| Some(object.full_id) != self.selected))
            .filter(|(_, o)| !o.is_avatar())
            .take(24)
        {
            data.insert(object.full_id.to_string(), aurora_llsd::llsd_map! {
                "name" => if object.text.is_empty() { aurora_i18n::tr!("common-object") } else { &object.text },
                "description" => "", "owner" => object.owner_id, "position" => aurora_llsd::Llsd::Array(object.position.to_array().map(|n| aurora_llsd::Llsd::Real(f64::from(n))).to_vec()),
                "landimpact" => 2, "navmesh_category" => 0, "phantom" => false, "modifiable" => true, "is_scripted" => true,
                "A" => 100, "B" => 75, "C" => 50, "D" => 25, "cpu_time" => 80.0, "length" => 1.5, "radius" => 0.5, "horizontal" => false
            });
        }
        self.reply(request, region, kind, aurora_net::pathfinding::parse(&data, kind));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn replies_are_correlated_with_request_region_and_kind_and_ignore_old_windows() {
        let mut state = PathfindingUi::default();
        let NetCommand::FetchPathfinding { request, .. } = state.open(1, Kind::Objects, Uuid::from_u128(1)) else {
            panic!("request");
        };
        state.reply(request, 2, Kind::Objects, Ok(Vec::new()));
        state.reply(request, 1, Kind::Characters, Ok(Vec::new()));
        state.reply(Uuid::new_v4(), 1, Kind::Objects, Ok(Vec::new()));
        assert_eq!(state.pending, Some(request));
        state.reply(request, 1, Kind::Objects, Err(Error::Unsupported));
        assert_eq!(state.error, Some(Error::Unsupported));
        let NetCommand::FetchPathfinding { request, .. } = state.refresh() else {
            panic!("refresh");
        };
        state.open = false;
        state.reply(request, 1, Kind::Objects, Ok(Vec::new()));
        assert_eq!(state.pending, Some(request));
    }
}
