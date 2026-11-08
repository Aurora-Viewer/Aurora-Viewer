//! "Group titles": every title of every group in one list, and the title
//! worn by region. Port of FSFloaterGroupTitles (indra/newview/
//! fsfloatergrouptitles.cpp, floater_fs_group_titles.xml, originally LGPL
//! 2.1): a filter, the list with the default mark, the title, the group and
//! its regions, "<No group title>" first; the active title in bold and
//! selected on opening; a double click or Activate wears the selected one;
//! This region / Type region / Clear region (a menu of its regions when it
//! has several, with Clear all regions), Info, Refresh, "Use default title
//! in regions without assignment" and Set as default.
//!
//! Aurora addition: under the titles, the list of the regions given a title
//! (the Legacy Viewer only shows them in the titles' column), where a
//! region's title is changed or removed, and a rule whose group was left is
//! shown as ignored. The rules themselves are `world/region_titles.rs`.

use super::widgets::{self, ActionKind, Floater};
use crate::theme::Palette;
use crate::world::region_titles::{LookupResult, RegionTitles, Target};
use aurora_net::GroupMembership;
use egui::{FontId, Rect, RichText, Sense, Vec2};
use uuid::Uuid;

/// The window's id (Floater.Toggle "fs_group_titles").
pub const WINDOW: &str = "group_titles";

const ROW_H: f32 = 22.0;
const BUTTON_H: f32 = 26.0;

/// What the window shows.
pub struct View<'a> {
    pub groups: &'a [GroupMembership],
    /// AgentDataUpdate's active group (nil: none).
    pub active: Uuid,
    pub titles: &'a RegionTitles,
    /// The agent's region; None outside one or while RLVa hides it.
    pub region: Option<&'a str>,
}

/// What the window asks the app for.
#[derive(Debug, Clone, PartialEq)]
pub enum TitlesAction {
    /// activateGroupTitle: GroupTitleUpdate (not for "no group"), then
    /// ActivateGroup when the group is not the active one.
    Activate {
        group: Uuid,
        role: Uuid,
    },
    Info(Uuid),
    Refresh,
    /// setAssignmentForCurrentRegion.
    AssignHere(Target),
    /// The FSSetTitleRegion dialog's name, to look up on the map.
    LookUp(String, Target),
    /// A region given another title (Aurora's rule list).
    Reassign(String, Target),
    Remove(String),
    RemoveTitle(Uuid, Uuid),
    UseDefault(bool),
    SetDefault(Target),
}

/// One row of the titles' list.
#[derive(Debug, Clone)]
struct Row {
    target: Target,
    active: bool,
}

#[derive(Default)]
pub struct GroupTitlesUi {
    filter: String,
    selected: Option<(Uuid, Uuid)>,
    /// The region name being typed (Type region), while its field shows.
    typing: Option<String>,
    /// The field takes the keyboard on the next frame (it just opened).
    focus: bool,
    /// The active row was selected once since the window opened.
    picked: bool,
}

/// The titles of `view`: "<No group title>" first, then by title and group
/// (title_sort_column, `0` for no group).
fn rows(view: &View) -> Vec<Row> {
    let mut out = vec![Row {
        target: Target {
            title: aurora_i18n::tr!("group-titles-no-title").to_owned(),
            group_name: aurora_i18n::tr!("common-none").to_owned(),
            ..Default::default()
        },
        active: view.active.is_nil(),
    }];
    let mut rest: Vec<Row> = view
        .groups
        .iter()
        .flat_map(|g| {
            view.titles.titles.get(&g.id).into_iter().flatten().map(move |t| Row {
                target: Target {
                    group: g.id,
                    role: t.role,
                    title: t.title.clone(),
                    group_name: g.name.clone(),
                },
                active: t.selected && g.id == view.active,
            })
        })
        .collect();
    rest.sort_by_cached_key(|r| {
        (
            crate::natural_order::key(&r.target.title),
            crate::natural_order::key(&r.target.group_name),
        )
    });
    out.extend(rest);
    out
}

/// The filter matches the title, the group or one of its regions.
fn matches(filter: &str, row: &Row, regions: &str) -> bool {
    let f = filter.trim().to_lowercase();
    f.is_empty()
        || row.target.title.to_lowercase().contains(&f)
        || row.target.group_name.to_lowercase().contains(&f)
        || regions.to_lowercase().contains(&f)
}

/// The default mark, the title, the group, the regions.
fn cells(row: Rect) -> [Rect; 4] {
    let mark = 24.0f32.min(row.width());
    let unit = (row.width() - mark).max(0.0) / 3.2;
    let mut x = row.left();
    let mut cell = |w: f32| {
        let r = Rect::from_min_max(egui::pos2(x, row.top()), egui::pos2(x + w, row.bottom()));
        x += w;
        r
    };
    [cell(mark), cell(unit * 1.2), cell(unit * 1.1), cell(unit * 0.9)]
}

fn icon(ui: &egui::Ui, name: &str, at: egui::Pos2, size: f32, color: egui::Color32) {
    if let Some(t) = crate::ui::icons::global(name) {
        let uv = Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
        ui.painter().image(t.id(), Rect::from_center_size(at, Vec2::splat(size)), uv, color);
    }
}

impl GroupTitlesUi {
    /// The window closed: the active row is selected again next time.
    pub fn closed(&mut self) {
        self.picked = false;
        self.typing = None;
    }

    fn header(ui: &mut egui::Ui, p: &Palette) {
        let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), ROW_H), Sense::hover());
        ui.painter().rect_filled(rect, ui.visuals().window_corner_radius, p.field);
        let [mark, title, group, regions] = cells(rect);
        icon(ui, "check", mark.center(), 12.0, p.muted);
        ui.interact(mark, ui.scope_id().with("default_mark_tip"), Sense::hover())
            .on_hover_text(aurora_i18n::tr!("group-titles-default-tip"));
        let font = FontId::proportional(12.0);
        for (cell, text) in [
            (title, aurora_i18n::tr!("group-titles-col-title")),
            (group, aurora_i18n::tr!("common-group")),
            (regions, aurora_i18n::tr!("group-titles-col-regions")),
        ] {
            widgets::paint_elided(
                ui,
                text,
                egui::pos2(cell.left() + 8.0, cell.center().y),
                cell.width() - 12.0,
                font.clone(),
                p.muted,
            );
        }
    }

    fn title_rows(&mut self, ui: &mut egui::Ui, p: &Palette, view: &View, shown: &[(Row, String)], actions: &mut Vec<TitlesAction>) {
        let radius = ui.visuals().window_corner_radius;
        let saved = &view.titles.saved;
        for (row, regions) in shown {
            let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), ROW_H), Sense::click());
            let key = (row.target.group, row.target.role);
            if resp.clicked() || resp.secondary_clicked() {
                self.selected = Some(key);
            }
            if resp.double_clicked() {
                actions.push(TitlesAction::Activate { group: key.0, role: key.1 });
            }
            if !ui.is_rect_visible(rect) {
                continue;
            }
            if self.selected == Some(key) {
                ui.painter().rect_filled(rect, radius, p.violet.gamma_multiply(0.30));
            } else if resp.hovered() {
                ui.painter().rect_filled(rect, radius, p.raised.gamma_multiply(0.6));
            }
            let [mark, title, group, region_cell] = cells(rect);
            if saved.use_default && saved.default.same(key.0, key.1) {
                icon(ui, "check", mark.center(), 14.0, p.ink);
            }
            let font = if row.active {
                FontId::new(12.5, crate::ui::fonts::semibold(ui.ctx()))
            } else {
                FontId::proportional(12.5)
            };
            for (cell, text, color) in [
                (title, row.target.title.as_str(), p.ink),
                (group, row.target.group_name.as_str(), p.muted),
                (region_cell, regions.as_str(), p.muted),
            ] {
                widgets::paint_elided(
                    ui,
                    text,
                    egui::pos2(cell.left() + 8.0, cell.center().y),
                    cell.width() - 12.0,
                    font.clone(),
                    color,
                );
            }
            if !regions.is_empty() {
                resp.on_hover_text(regions.as_str());
            }
        }
    }

    /// Aurora's list of the regions given a title.
    fn rule_rows(ui: &mut egui::Ui, p: &Palette, view: &View, all: &[Row], actions: &mut Vec<TitlesAction>) {
        ui.add_space(10.0);
        widgets::section_header(ui, p, "map-pin-filled", aurora_i18n::tr!("group-titles-rules"));
        let saved = &view.titles.saved;
        if saved.rules.is_empty() {
            ui.add(
                egui::Label::new(
                    RichText::new(aurora_i18n::tr!("group-titles-rules-empty"))
                        .size(12.0)
                        .color(p.muted),
                )
                .wrap(),
            );
            return;
        }
        let radius = ui.visuals().window_corner_radius;
        for rule in &saved.rules {
            let t = &rule.target;
            let left = !t.group.is_nil() && !view.groups.iter().any(|g| g.id == t.group);
            let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), BUTTON_H), Sense::hover());
            if resp.hovered() {
                ui.painter().rect_filled(rect, radius, p.raised.gamma_multiply(0.6));
            }
            let buttons = 2.0 * 24.0 + 10.0;
            let text = rect.with_max_x((rect.right() - buttons).max(rect.left()));
            let region_w = text.width() * 0.38;
            let at = egui::pos2(text.left() + 8.0, text.center().y);
            widgets::paint_elided(
                ui,
                &rule.region,
                at,
                region_w - 12.0,
                FontId::new(12.5, crate::ui::fonts::semibold(ui.ctx())),
                p.ink,
            );
            let wears = if t.group.is_nil() {
                aurora_i18n::tr!("group-titles-no-title").to_owned()
            } else {
                aurora_i18n::tr!("group-titles-rule-wears", title = t.title.as_str(), group = t.group_name.as_str())
            };
            let mut x = text.left() + region_w;
            if left {
                icon(ui, "alert-triangle-filled", egui::pos2(x + 7.0, text.center().y), 13.0, p.muted);
                x += 18.0;
            }
            let shown = if left {
                aurora_i18n::tr!("group-titles-rule-group-left", wears = wears.as_str())
            } else {
                wears
            };
            widgets::paint_elided(
                ui,
                &shown,
                egui::pos2(x, text.center().y),
                text.right() - x - 6.0,
                FontId::proportional(12.5),
                p.muted,
            );
            if left {
                resp.clone().on_hover_text(aurora_i18n::tr!("group-titles-rule-group-left-tip"));
            }
            let mut strip = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(Rect::from_min_max(egui::pos2(rect.right() - buttons, rect.top()), rect.max))
                    .layout(egui::Layout::right_to_left(egui::Align::Center)),
            );
            strip.add_space(4.0);
            if widgets::icon_button(&mut strip, p, "trash-filled", aurora_i18n::tr!("common-remove"), true) {
                actions.push(TitlesAction::Remove(rule.region.clone()));
            }
            let change = widgets::action_button(&mut strip, p, "pencil-filled", "", ActionKind::Flat, Vec2::new(24.0, 20.0))
                .on_hover_text(aurora_i18n::tr!("group-titles-change"));
            super::menu::dropdown(&change, p, egui::PopupCloseBehavior::CloseOnClick, |ui| {
                for row in all {
                    let current = t.same(row.target.group, row.target.role);
                    let label = if row.target.group.is_nil() {
                        row.target.title.clone()
                    } else {
                        aurora_i18n::tr!(
                            "group-titles-rule-wears",
                            title = row.target.title.as_str(),
                            group = row.target.group_name.as_str()
                        )
                    };
                    if super::menu::check(ui, p, "tag-filled", &label, current, true) && !current {
                        actions.push(TitlesAction::Reassign(rule.region.clone(), row.target.clone()));
                    }
                }
            });
        }
    }

    fn footer(&mut self, ui: &mut egui::Ui, p: &Palette, view: &View, all: &[Row], actions: &mut Vec<TitlesAction>) {
        let selected = self
            .selected
            .and_then(|(g, r)| all.iter().find(|row| row.target.same(g, r)))
            .map(|row| row.target.clone());
        let saved = &view.titles.saved;
        let size = Vec2::new(0.0, BUTTON_H);
        let mut cancel = false;
        let focus = std::mem::take(&mut self.focus);
        if let Some(text) = &mut self.typing {
            ui.horizontal(|ui| {
                ui.set_height(BUTTON_H);
                let field = ui.add(
                    egui::TextEdit::singleline(text)
                        .hint_text(aurora_i18n::tr!("group-titles-region-hint"))
                        .char_limit(crate::world::region_titles::MAX_REGION_NAME)
                        .desired_width((ui.available_width() - 200.0).clamp(80.0, 320.0)),
                );
                if focus {
                    field.request_focus();
                }
                let enter = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                let ready = selected.is_some() && !text.trim().is_empty() && view.titles.lookup.is_none();
                let add = widgets::action_button_if(ui, p, "check", aurora_i18n::tr!("common-add"), ActionKind::Flat, size, ready);
                if (add.clicked() || enter && ready)
                    && let Some(target) = selected.clone()
                {
                    actions.push(TitlesAction::LookUp(text.clone(), target));
                }
                if widgets::action_button(ui, p, "x", aurora_i18n::tr!("common-cancel"), ActionKind::Flat, size).clicked() {
                    cancel = true;
                }
            });
        }
        if cancel || matches!(actions.last(), Some(TitlesAction::LookUp(..))) {
            self.typing = None;
        }
        let status = match (&view.titles.lookup, &view.titles.lookup_result) {
            (Some(l), _) => Some((aurora_i18n::tr!("group-titles-looking-up", region = l.name.as_str()), p.muted)),
            (None, Some(LookupResult::NotFound(name))) => {
                Some((aurora_i18n::tr!("group-titles-not-found", region = name.as_str()), p.danger))
            }
            (None, Some(LookupResult::Added(name))) => Some((aurora_i18n::tr!("group-titles-added", region = name.as_str()), p.muted)),
            (None, None) => None,
        };
        if let Some((text, color)) = status {
            ui.add(egui::Label::new(RichText::new(text).size(12.0).color(color)).wrap());
        }
        ui.horizontal_wrapped(|ui| {
            let has = selected.is_some();
            let here = widgets::action_button_if(
                ui,
                p,
                "map-pin-filled",
                aurora_i18n::tr!("group-titles-this-region"),
                ActionKind::Flat,
                size,
                has && view.region.is_some(),
            )
            .on_hover_text(aurora_i18n::tr!("group-titles-this-region-tip"));
            if here.clicked()
                && let Some(target) = selected.clone()
            {
                actions.push(TitlesAction::AssignHere(target));
            }
            let typed = widgets::action_button_if(
                ui,
                p,
                "pencil-filled",
                aurora_i18n::tr!("group-titles-type-region"),
                ActionKind::Flat,
                size,
                has,
            )
            .on_hover_text(aurora_i18n::tr!("group-titles-type-region-tip"));
            if typed.clicked() {
                self.typing = Some(String::new());
                self.focus = true;
            }
            let regions: Vec<String> = selected
                .as_ref()
                .map(|t| saved.regions_of(t.group, t.role).into_iter().map(str::to_owned).collect())
                .unwrap_or_default();
            let clear = widgets::action_button_if(
                ui,
                p,
                "trash-filled",
                aurora_i18n::tr!("group-titles-clear-region"),
                ActionKind::Flat,
                size,
                !regions.is_empty(),
            )
            .on_hover_text(aurora_i18n::tr!("group-titles-clear-region-tip"));
            if regions.len() == 1 && clear.clicked() {
                actions.push(TitlesAction::Remove(regions[0].clone()));
            }
            if regions.len() > 1 {
                super::menu::dropdown(&clear, p, egui::PopupCloseBehavior::CloseOnClick, |ui| {
                    for r in &regions {
                        if super::menu::item(ui, p, "map-pin-filled", r) {
                            actions.push(TitlesAction::Remove(r.clone()));
                        }
                    }
                    super::menu::separator(ui, p);
                    if super::menu::item(ui, p, "trash-filled", aurora_i18n::tr!("group-titles-clear-all"))
                        && let Some(t) = &selected
                    {
                        actions.push(TitlesAction::RemoveTitle(t.group, t.role));
                    }
                });
            }
            let real = selected.as_ref().is_some_and(|t| !t.group.is_nil());
            let info = widgets::action_button_if(
                ui,
                p,
                "info-circle-filled",
                aurora_i18n::tr!("group-titles-info"),
                ActionKind::Flat,
                size,
                real,
            )
            .on_hover_text(aurora_i18n::tr!("group-titles-info-tip"));
            if info.clicked()
                && let Some(t) = &selected
            {
                actions.push(TitlesAction::Info(t.group));
            }
            let activate = widgets::action_button_if(
                ui,
                p,
                "circle-check-filled",
                aurora_i18n::tr!("group-titles-activate"),
                ActionKind::Flat,
                size,
                has,
            )
            .on_hover_text(aurora_i18n::tr!("group-titles-activate-tip"));
            if activate.clicked()
                && let Some(t) = &selected
            {
                actions.push(TitlesAction::Activate {
                    group: t.group,
                    role: t.role,
                });
            }
            let refresh = widgets::action_button(ui, p, "refresh", "", ActionKind::Flat, Vec2::new(BUTTON_H, BUTTON_H))
                .on_hover_text(aurora_i18n::tr!("group-titles-refresh-tip"));
            if refresh.clicked() {
                actions.push(TitlesAction::Refresh);
            }
        });
        ui.horizontal_wrapped(|ui| {
            let mut on = saved.use_default;
            if widgets::switch_labeled(ui, p, &mut on, aurora_i18n::tr!("group-titles-use-default"))
                .on_hover_text(aurora_i18n::tr!("group-titles-use-default-tip"))
                .changed()
            {
                actions.push(TitlesAction::UseDefault(on));
            }
            let set = widgets::action_button_if(
                ui,
                p,
                "star-filled",
                aurora_i18n::tr!("group-titles-set-default"),
                ActionKind::Flat,
                size,
                selected.is_some(),
            )
            .on_hover_text(aurora_i18n::tr!("group-titles-set-default-tip"));
            if set.clicked()
                && let Some(t) = selected.clone()
            {
                actions.push(TitlesAction::SetDefault(t));
            }
        });
    }

    /// Draw the window while `open`.
    pub fn show(&mut self, ctx: &egui::Context, p: &Palette, view: &View, open: &mut bool) -> Vec<TitlesAction> {
        let mut actions = Vec::new();
        if !*open {
            return actions;
        }
        let all = rows(view);
        if !self.picked
            && let Some(active) = all.iter().find(|r| r.active)
        {
            self.selected = Some((active.target.group, active.target.role));
            self.picked = true;
        }
        let mut floater = Floater::new(
            WINDOW,
            aurora_i18n::tr!("toolbar-command-group-titles"),
            egui::pos2(300.0, 160.0),
            Vec2::new(640.0, 540.0),
        );
        floater.min_size = Vec2::new(520.0, 320.0);
        floater.show(ctx, p, open, |ui| {
            let w = ui.available_width();
            ui.allocate_ui_with_layout(Vec2::new(w, 30.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                widgets::search_box(ui, p, &mut self.filter, aurora_i18n::tr!("group-titles-filter"), w);
            });
            ui.add_space(6.0);
            let shown: Vec<(Row, String)> = all
                .iter()
                .map(|r| (r.clone(), view.titles.saved.regions_of(r.target.group, r.target.role).join(", ")))
                .filter(|(r, regions)| matches(&self.filter, r, regions))
                .collect();
            let (mut list, footer) = widgets::above_footer(ui);
            Self::header(&mut list, p);
            egui::ScrollArea::vertical()
                .id_salt("group_titles_rows")
                .auto_shrink([false, false])
                .show(&mut list, |ui| {
                    ui.spacing_mut().item_spacing.y = 1.0;
                    self.title_rows(ui, p, view, &shown, &mut actions);
                    let missing = view.groups.iter().any(|g| !view.titles.titles.contains_key(&g.id));
                    if missing {
                        ui.add_space(4.0);
                        ui.label(RichText::new(aurora_i18n::tr!("group-titles-loading")).size(12.0).color(p.muted));
                    }
                    Self::rule_rows(ui, p, view, &all, &mut actions);
                });
            footer.show(ui, |ui| self.footer(ui, p, view, &all, &mut actions));
        });
        if !*open {
            self.closed();
        }
        actions
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aurora_net::groups::GroupTitle;

    const G1: Uuid = Uuid::from_u128(1);
    const G2: Uuid = Uuid::from_u128(2);
    const LEFT: Uuid = Uuid::from_u128(3);
    const R1: Uuid = Uuid::from_u128(11);
    const R2: Uuid = Uuid::from_u128(12);

    fn group(id: Uuid, name: &str) -> GroupMembership {
        GroupMembership {
            id,
            name: name.into(),
            insignia: Uuid::nil(),
            powers: 0,
            accept_notices: true,
            list_in_profile: true,
            contribution: 0,
        }
    }

    fn titles() -> RegionTitles {
        let mut t = RegionTitles::default();
        t.request_titles([G1, G2]);
        for cmd in t.take_commands() {
            let aurora_net::NetCommand::GroupTitlesRequest { group, request } = cmd else {
                continue;
            };
            let list = if group == G1 {
                vec![
                    GroupTitle {
                        title: "Builder".into(),
                        role: R1,
                        selected: true,
                    },
                    GroupTitle {
                        title: "Member".into(),
                        role: Uuid::nil(),
                        selected: false,
                    },
                ]
            } else {
                vec![GroupTitle {
                    title: "Guide".into(),
                    role: R2,
                    selected: true,
                }]
            };
            t.on_titles(group, request, &list, "");
        }
        let target = |group, role, title: &str, name: &str| Target {
            group,
            role,
            title: title.into(),
            group_name: name.into(),
        };
        for i in 0..12 {
            t.saved.assign(&format!("Region {i}"), target(G2, R2, "Guide", "Wolves"));
        }
        t.saved.assign("Gone", target(LEFT, R1, "Old title", "A group left"));
        t
    }

    #[test]
    fn no_group_comes_first_then_titles_sorted() {
        let groups = [group(G1, "Builders"), group(G2, "Wolves")];
        let t = titles();
        let view = View {
            groups: &groups,
            active: G1,
            titles: &t,
            region: Some("Home"),
        };
        let r = rows(&view);
        let names: Vec<&str> = r.iter().map(|r| r.target.title.as_str()).collect();
        assert_eq!(names[1..], ["Builder", "Guide", "Member"]);
        assert!(r[0].target.group.is_nil() && !r[0].active);
        assert!(r.iter().find(|r| r.target.title == "Builder").is_some_and(|r| r.active));
        assert!(
            r.iter().find(|r| r.target.title == "Guide").is_some_and(|r| !r.active),
            "selected in a group not active"
        );
        let guide = r.iter().find(|r| r.target.title == "Guide").expect("guide");
        assert!(matches("region 1", guide, "Region 1, Region 2"));
        assert!(matches("wol", guide, ""));
        assert!(!matches("builder", guide, ""));
    }

    /// The window keeps its size with a long list over its footer, the
    /// region field open.
    #[test]
    fn the_window_keeps_its_size() {
        let groups = [group(G1, "Builders"), group(G2, "Wolves")];
        let t = titles();
        widgets::steady_tests::assert_window_steady(
            WINDOW,
            &[Vec2::new(640.0, 460.0), Vec2::new(520.0, 320.0)],
            aurora_i18n::tr!("group-titles-use-default"),
            || GroupTitlesUi {
                typing: Some("Some".into()),
                ..Default::default()
            },
            |ui_state, ctx, p| {
                let view = View {
                    groups: &groups,
                    active: G1,
                    titles: &t,
                    region: Some("Home"),
                };
                ui_state.show(ctx, p, &view, &mut true);
            },
        );
    }
}
