//! Whole-block gestures share the single-control commit transaction, including
//! its saved gap identities and no-op rules (LLToolBarView::handleDragTool).

use super::{Bar, BarStyle, Drag, Edit, Style, ToolbarSettings, ToolbarUi, drawers, edit_mode, structure};
use crate::{
    theme::Palette,
    ui::{
        menu,
        skin::{Layout, ToolButton},
        widgets,
    },
};
use egui::{Id, Rect, Vec2};

pub(crate) fn saved_style(ctx: &egui::Context, command: &str) -> BarStyle {
    ToolbarUi::layout_configuration(ctx)
        .unwrap_or_else(|| super::appearance::settings(ctx))
        .block_styles
        .get(command)
        .copied()
        .unwrap_or_default()
}

pub(crate) fn style(ctx: &egui::Context, command: &str, base: Style) -> Style {
    let saved = saved_style(ctx, command);
    Style {
        mode: saved.mode.unwrap_or(base.mode),
        align: saved.align.unwrap_or(base.align),
        sizing: saved.sizing.unwrap_or(base.sizing),
    }
}

/// Keep one default per visible root block through split, merge and drag edits.
/// Destination members choose the default when a moved control joins a block;
/// an isolated whole block carries its own defaults across edges.
pub(super) fn normalize_styles(settings: &mut ToolbarSettings, layout: &Layout, moving: &[String]) {
    if settings.block_styles.is_empty() {
        return;
    }
    let previous = std::mem::take(&mut settings.block_styles);
    let mut next = std::collections::BTreeMap::new();
    for bar in Bar::ALL {
        for group in settings.bar(bar, layout).split(|button| structure::is_separator(&button.command)) {
            let real = |button: &&ToolButton| !structure::is_divider(&button.command);
            let anchor = group
                .iter()
                .filter(real)
                .find(|button| !moving.contains(&button.command))
                .or_else(|| group.iter().find(|button| !structure::is_divider(&button.command)));
            let Some(anchor) = anchor else { continue };
            let style = previous.get(&anchor.command).copied().unwrap_or_default();
            if style != BarStyle::default() {
                for member in group {
                    next.insert(member.command.clone(), style);
                }
            }
        }
    }
    settings.block_styles = next;
}

#[allow(clippy::too_many_arguments)]
pub(super) fn style_menu(
    ui: &mut egui::Ui,
    p: &Palette,
    toolbar: &mut ToolbarUi,
    settings: &ToolbarSettings,
    bar: Bar,
    command: &str,
    base: Style,
) {
    let saved = settings.block_styles.get(command).copied().unwrap_or_default();
    let mut next = BarStyle::default();
    for (mode, icon, label) in super::mode_rows() {
        if menu::check(ui, p, icon, label, saved.mode.unwrap_or(base.mode) == mode, !settings.locked) {
            next.mode = Some(mode);
        }
    }
    menu::separator(ui, p);
    for (align, icon, label) in super::align_rows(bar) {
        if menu::check(ui, p, icon, label, saved.align.unwrap_or(base.align) == align, !settings.locked) {
            next.align = Some(align);
        }
    }
    menu::separator(ui, p);
    for (sizing, icon, label) in super::sizing_rows() {
        if menu::check(ui, p, icon, label, saved.sizing.unwrap_or(base.sizing) == sizing, !settings.locked) {
            next.sizing = Some(sizing);
        }
    }
    if next != BarStyle::default() {
        toolbar.pending = Some(Edit::BlockStyle(bar, command.into(), next));
    }
}

#[derive(Clone, Copy)]
pub(super) enum Destination<'a> {
    Bar(Bar, usize, bool),
    Drawer(&'a str, usize),
}

pub(super) fn move_to(
    settings: &mut ToolbarSettings,
    layout: &Layout,
    members: &[ToolButton],
    destination: Destination<'_>,
    provisional: bool,
) -> bool {
    if settings.locked || members.is_empty() {
        return false;
    }
    let Some(first) = members.iter().find(|button| !structure::is_divider(&button.command)) else {
        return false;
    };
    let Some((source, range)) = Bar::ALL.into_iter().find_map(|bar| {
        let list = settings.bar(bar, layout);
        edit_mode::group(list, &first.command)
            .filter(|range| list[range.clone()] == *members)
            .map(|range| (bar, range))
    }) else {
        return false;
    };
    if matches!(destination, Destination::Drawer(..)) && members.iter().any(|button| drawers::is_drawer(&button.command)) {
        return false;
    }
    let mut next = settings.clone();
    let original = next.bar(source, layout).to_vec();
    let mut collapsed = original.clone();
    collapsed.splice(range, [first.clone()]);
    next.set_bar(source, collapsed);
    let edit = match destination {
        Destination::Bar(bar, rank, separate) => {
            let separate = separate
                && next
                    .bar(bar, layout)
                    .iter()
                    .any(|button| button.command != first.command && !structure::is_separator(&button.command));
            let removed_before = if source == bar {
                original
                    .iter()
                    .take(rank)
                    .filter(|button| button.command != first.command && members.iter().any(|member| member.command == button.command))
                    .count()
            } else {
                0
            };
            if separate {
                Edit::InsertGroup(bar, first.clone(), rank.saturating_sub(removed_before))
            } else {
                Edit::Insert(bar, first.clone(), rank.saturating_sub(removed_before))
            }
        }
        Destination::Drawer(id, rank) => Edit::DrawerInsert(id.into(), first.clone(), rank),
    };
    let mut transaction = ToolbarUi {
        pending: Some(edit),
        ..Default::default()
    };
    transaction.commit_impl(layout, &mut next, provisional);
    match destination {
        Destination::Bar(bar, ..) => {
            let mut list = next.bar(bar, layout).to_vec();
            let Some(at) = list.iter().position(|button| button.command == first.command) else {
                return false;
            };
            list.splice(at..at + 1, members.iter().cloned());
            next.set_bar(bar, list);
        }
        Destination::Drawer(id, _) => {
            let Some(drawer) = next.drawers.get_mut(id) else { return false };
            let Some(at) = drawer.buttons.iter().position(|button| button.command == first.command) else {
                return false;
            };
            drawer.buttons.splice(at..at + 1, members.iter().cloned());
        }
    }
    // A move never adds/removes the integrated navigation control. Its temporary
    // representative must not change the separately saved visibility preference.
    next.address_visible = settings.address_visible;
    normalize_styles(
        &mut next,
        layout,
        &members.iter().map(|button| button.command.clone()).collect::<Vec<_>>(),
    );
    if next == *settings {
        return false;
    }
    *settings = next;
    true
}

#[derive(Clone)]
struct Target {
    bar: Bar,
    style: Style,
    rect: Rect,
    members: Vec<ToolButton>,
}

fn targets_id() -> Id {
    Id::unique("toolbar_block_handles")
}

pub(super) fn prepare(ctx: &egui::Context) {
    ctx.data_mut(|data| data.insert_temp(targets_id(), Vec::<Target>::new()));
}

pub(crate) fn collect(ui: &egui::Ui, bar: Bar, style: Style, buttons: &[ToolButton], rects: &[Rect]) {
    if !ToolbarUi::editing(ui.ctx()) || ToolbarUi::dragging(ui.ctx()) {
        return;
    }
    let settings = super::appearance::settings(ui.ctx());
    let mut start = 0;
    let mut targets = Vec::new();
    for end in 0..=buttons.len() {
        if end != buttons.len() && !structure::is_separator(&buttons[end].command) {
            continue;
        }
        let members = &buttons[start..end];
        if let Some(first) = members.iter().find(|button| !structure::is_divider(&button.command))
            && settings.drawer_of(&first.command).is_none()
            && let Some(bounds) = rects[start..end]
                .iter()
                .copied()
                .filter(|rect| rect.is_positive())
                .reduce(Rect::union)
        {
            let rect = bounds.expand(4.0).intersect(ui.clip_rect());
            if rect.is_positive() {
                targets.push(Target {
                    bar,
                    style,
                    rect,
                    members: members.to_vec(),
                });
            }
        }
        start = end + 1;
    }
    ui.ctx()
        .data_mut(|data| data.get_temp_mut_or_default::<Vec<Target>>(targets_id()).extend(targets));
}

const CONTROL_SIZE: f32 = 22.0;

fn control_bounds(bar: Bar, block: Rect, screen: Rect) -> Rect {
    let size = if bar.vertical() {
        Vec2::new(CONTROL_SIZE, CONTROL_SIZE * 2.0)
    } else {
        Vec2::new(CONTROL_SIZE * 2.0, CONTROL_SIZE)
    }
    .min(screen.size());
    let start = match bar {
        Bar::Top => egui::pos2(block.center().x - size.x / 2.0, block.bottom() + 2.0),
        Bar::Bottom => egui::pos2(block.center().x - size.x / 2.0, block.top() - size.y - 2.0),
        Bar::Left => egui::pos2(block.right() + 2.0, block.center().y - size.y / 2.0),
        Bar::Right => egui::pos2(block.left() - size.x - 2.0, block.center().y - size.y / 2.0),
    };
    Rect::from_min_size(
        egui::pos2(
            start.x.clamp(screen.left(), (screen.right() - size.x).max(screen.left())),
            start.y.clamp(screen.top(), (screen.bottom() - size.y).max(screen.top())),
        ),
        size,
    )
}

/// The settings gear acts on the complete canonical block, including temporarily
/// hidden native controls. Every mutation uses the same guarded commit path.
#[allow(clippy::too_many_arguments)]
fn settings_menu(
    ui: &mut egui::Ui,
    p: &Palette,
    toolbar: &mut ToolbarUi,
    settings: &ToolbarSettings,
    layout: &Layout,
    bar: Bar,
    command: &str,
    base: Style,
) {
    let buttons = settings.bar(bar, layout);
    let Some(range) = edit_mode::group(buttons, command) else { return };
    let members = &buttons[range];
    let Some(first) = members.first() else { return };
    let unlocked = !settings.locked;
    menu::info(ui, p, "settings-filled", aurora_i18n::tr!("toolbar-block-settings"));
    style_menu(ui, p, toolbar, settings, bar, command, base);
    menu::separator(ui, p);
    let splits: Vec<_> = members
        .iter()
        .enumerate()
        .filter(|(index, button)| {
            !structure::is_divider(&button.command) && members[..*index].iter().any(|previous| !structure::is_divider(&previous.command))
        })
        .map(|(_, button)| button)
        .collect();
    menu::submenu(
        ui,
        p,
        "separator-vertical",
        aurora_i18n::tr!("toolbar-block-split"),
        unlocked && !splits.is_empty(),
        |ui| {
            for button in splits {
                if menu::item(
                    ui,
                    p,
                    "separator-vertical",
                    &aurora_i18n::tr!("toolbar-block-split-before", button = super::label(button)),
                ) {
                    toolbar.pending = Some(Edit::Split(bar, button.command.clone(), true));
                }
            }
        },
    );
    for before in [true, false] {
        if menu::item_if(
            ui,
            p,
            "arrows-join",
            aurora_i18n::text(if before {
                "toolbar-edit-merge-before"
            } else {
                "toolbar-edit-merge-after"
            }),
            unlocked && edit_mode::merge_range(buttons, command, before).is_some(),
        ) {
            toolbar.pending = Some(Edit::Merge(bar, command.into(), before));
        }
    }
    if members.len() == 1 && drawers::is_drawer(&first.command) {
        if menu::item_if(ui, p, "pencil-filled", aurora_i18n::tr!("toolbar-drawer-settings"), unlocked) {
            toolbar.drawer_editor = Some(first.command.clone());
        }
        if menu::item_if(ui, p, "folder-open-filled", aurora_i18n::tr!("toolbar-drawer-unfold"), unlocked) {
            toolbar.pending = Some(Edit::Unfold(bar, first.command.clone()));
        }
    } else if menu::item_if(
        ui,
        p,
        "folder-filled",
        aurora_i18n::tr!("toolbar-drawer-fold"),
        unlocked && edit_mode::fold_allowed(buttons, command),
    ) {
        toolbar.pending = Some(Edit::Fold(bar, command.into()));
    }
    if menu::item_if(ui, p, "separator-vertical", aurora_i18n::tr!("toolbar-add-separator"), unlocked) {
        toolbar.pending = Some(Edit::DividerAfter(bar, members.last().map(|button| button.command.clone())));
    }
    menu::separator(ui, p);
    if menu::item_if(ui, p, "trash-filled", aurora_i18n::tr!("toolbar-block-remove"), unlocked) {
        toolbar.pending = Some(Edit::RemoveBlock(members.iter().map(|button| button.command.clone()).collect()));
    }
    if menu::check(ui, p, "lock-filled", aurora_i18n::tr!("toolbar-locked"), settings.locked, true) {
        toolbar.pending = Some(Edit::Lock);
    }
    super::native_context::start_group(ui, p);
    toolbar.interface_edit_menu(ui, p, Some(command));
}

pub(super) fn show(ctx: &egui::Context, p: &Palette, toolbar: &mut ToolbarUi, settings: &ToolbarSettings, layout: &Layout) {
    if !ToolbarUi::editing(ctx) || ToolbarUi::dragging(ctx) {
        return;
    }
    let targets = ctx.data(|data| data.get_temp::<Vec<Target>>(targets_id())).unwrap_or_default();
    for target in targets {
        let Some(first) = target.members.iter().find(|button| !structure::is_divider(&button.command)) else {
            continue;
        };
        let rect = control_bounds(target.bar, target.rect, ctx.content_rect());
        egui::Area::new(Id::unique(("toolbar_block_grip_area", target.bar, &first.command)))
            .order(egui::Order::Foreground)
            .fixed_pos(rect.min)
            .constrain(false)
            .fade_in(false)
            .show(ctx, |ui| {
                let (rect, _) = ui.allocate_exact_size(rect.size(), egui::Sense::hover());
                ui.painter().add(widgets::overlay_frame(ctx, p).paint(rect));
                let (grip, gear) = if target.bar.vertical() {
                    (
                        Rect::from_min_max(rect.min, rect.right_bottom() - Vec2::new(0.0, CONTROL_SIZE)),
                        Rect::from_min_max(rect.left_bottom() - Vec2::new(0.0, CONTROL_SIZE), rect.max),
                    )
                } else {
                    (
                        Rect::from_min_max(rect.min, rect.right_bottom() - Vec2::new(CONTROL_SIZE, 0.0)),
                        Rect::from_min_max(rect.right_top() - Vec2::new(CONTROL_SIZE, 0.0), rect.max),
                    )
                };
                let response = ui.interact(
                    grip,
                    Id::unique(("toolbar_block_grip", target.bar, &first.command)),
                    egui::Sense::drag(),
                );
                widgets::paint_glyph(
                    ui.painter(),
                    if target.bar.vertical() {
                        "grip-vertical"
                    } else {
                        "grip-horizontal"
                    },
                    grip.center(),
                    14.0,
                    if settings.locked { p.muted_dim } else { p.ink },
                );
                let mut child = ui.new_child(
                    egui::UiBuilder::new()
                        .id_salt(("block_settings", target.bar, &first.command))
                        .max_rect(gear),
                );
                let settings_button = widgets::action_button(&mut child, p, "settings-filled", "", widgets::ActionKind::Flat, gear.size())
                    .on_hover_text(aurora_i18n::tr!("toolbar-block-settings"));
                ctx.data_mut(|data| {
                    data.insert_temp(
                        Id::unique(("toolbar_block_settings", target.bar, &first.command)),
                        settings_button.rect,
                    )
                });
                let place = match target.bar {
                    Bar::Top => egui::RectAlign::BOTTOM_START,
                    Bar::Bottom => egui::RectAlign::TOP_START,
                    Bar::Left => egui::RectAlign::RIGHT,
                    Bar::Right => egui::RectAlign::LEFT,
                };
                let opened = menu::dropdown_placed(
                    &settings_button,
                    p,
                    egui::PopupCloseBehavior::CloseOnClickOutside,
                    Some((place, 6.0)),
                    |ui| {
                        settings_menu(ui, p, toolbar, settings, layout, target.bar, &first.command, target.style);
                    },
                );
                if response.hovered() || settings_button.hovered() || opened {
                    ctx.layer_painter(ui.layer_id()).rect_stroke(
                        target.rect,
                        ui.visuals().widgets.inactive.corner_radius,
                        egui::Stroke::new(1.0, p.violet_light),
                        egui::StrokeKind::Outside,
                    );
                }
                if !settings.locked && response.drag_started_by(egui::PointerButton::Primary) {
                    response.dnd_set_drag_payload(Drag {
                        button: first.clone(),
                        from: Some(target.bar),
                        members: target.members.clone(),
                    });
                }
                response
                    .on_hover_cursor(if settings.locked {
                        egui::CursorIcon::NotAllowed
                    } else {
                        egui::CursorIcon::Grab
                    })
                    .on_hover_text(aurora_i18n::tr!("toolbar-move-block"));
            });
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::button;
    use super::*;

    #[test]
    fn scoped_defaults_follow_moves_splits_and_drawers_without_changing_other_blocks() {
        let layout = Layout::default();
        let mut settings = ToolbarSettings {
            composed: true,
            buttons: Some(Vec::new()),
            top: vec![
                button("inventory"),
                button("people"),
                structure::separator("scope"),
                button("preferences"),
                button("worldmap"),
            ],
            ..Default::default()
        };
        let mut toolbar = ToolbarUi::default();
        let first = BarStyle {
            mode: Some(super::super::ButtonMode::TextOnly),
            sizing: Some(super::super::Sizing::Auto),
            align: Some(super::super::Align::Start),
        };
        let second = BarStyle {
            mode: Some(super::super::ButtonMode::IconsWithText),
            sizing: Some(super::super::Sizing::Fill),
            align: Some(super::super::Align::End),
        };
        for (anchor, style) in [("inventory", first), ("preferences", second)] {
            toolbar.pending = Some(Edit::BlockStyle(Bar::Top, anchor.into(), style));
            assert!(toolbar.commit(&layout, &mut settings));
        }
        assert_eq!(settings.top_style, BarStyle::default());
        assert_eq!(settings.block_styles["inventory"], first);
        assert_eq!(settings.block_styles["people"], first);
        assert_eq!(settings.block_styles["preferences"], second);
        toolbar.pending = Some(Edit::MoveBlock(Bar::Right, vec![button("inventory"), button("people")], 0, true));
        assert!(toolbar.commit(&layout, &mut settings));
        assert_eq!(settings.block_styles["people"], first);
        let before_noop = settings.clone();
        toolbar.pending = Some(Edit::MoveBlock(Bar::Right, vec![button("inventory"), button("people")], 0, true));
        assert!(!toolbar.commit(&layout, &mut settings));
        assert_eq!(settings, before_noop);
        toolbar.pending = Some(Edit::Split(Bar::Right, "people".into(), true));
        assert!(toolbar.commit(&layout, &mut settings));
        assert_eq!(settings.block_styles["people"], first);
        toolbar.pending = Some(Edit::Merge(Bar::Right, "people".into(), true));
        assert!(toolbar.commit(&layout, &mut settings));
        let rank = settings
            .top
            .iter()
            .position(|button| button.command == "preferences")
            .expect("destination");
        toolbar.pending = Some(Edit::Insert(Bar::Top, button("inventory"), rank));
        assert!(toolbar.commit(&layout, &mut settings));
        assert_eq!(
            settings.block_styles["inventory"], second,
            "the joined destination supplies its defaults"
        );
        assert_eq!(settings.block_styles["people"], first, "the remaining source retains its defaults");
        toolbar.pending = Some(Edit::Fold(Bar::Top, "inventory".into()));
        assert!(toolbar.commit(&layout, &mut settings));
        let id = settings
            .top
            .iter()
            .find(|button| drawers::is_drawer(&button.command))
            .expect("folded block")
            .command
            .clone();
        assert_eq!(settings.drawers[&id].mode, super::super::ButtonMode::IconsWithText);
        settings.drawers.get_mut(&id).expect("drawer").mode = super::super::ButtonMode::TextOnly;
        toolbar.pending = Some(Edit::Unfold(Bar::Top, id));
        assert!(toolbar.commit(&layout, &mut settings));
        assert_eq!(settings.block_styles["inventory"].mode, Some(super::super::ButtonMode::TextOnly));
        assert_eq!(settings.block_styles["inventory"].sizing, second.sizing);
        assert_eq!(settings.block_styles["inventory"].align, second.align);
        assert_eq!(settings.block_styles["people"], first);
        let mut loaded: ToolbarSettings = serde_json::from_str(&serde_json::to_string(&settings).expect("save")).expect("load");
        loaded.sanitize();
        assert_eq!(loaded, settings);
        let before = settings.clone();
        settings.locked = true;
        toolbar.pending = Some(Edit::BlockStyle(Bar::Top, "inventory".into(), first));
        assert!(!toolbar.commit(&layout, &mut settings));
        settings.locked = false;
        assert_eq!(settings, before);
        let old: ToolbarSettings = serde_json::from_str("{}").expect("old settings");
        assert!(old.block_styles.is_empty());
        let mut inherited = ToolbarSettings::default();
        inherited.block_styles.insert("custom-skin-command".into(), first);
        inherited.sanitize();
        assert_eq!(
            inherited.block_styles["custom-skin-command"], first,
            "loading must not prune styles before the real skin is known"
        );
    }

    #[test]
    fn whole_blocks_keep_members_and_gap_preferences_across_edges_and_repeated_noops() {
        let layout = Layout::default();
        for source in Bar::ALL {
            for destination in Bar::ALL {
                let members = vec![button("inventory"), structure::divider("inside"), button("people")];
                let mut settings = ToolbarSettings {
                    composed: true,
                    top: Vec::new(),
                    buttons: Some(Vec::new()),
                    ..Default::default()
                };
                settings.set_bar(
                    source,
                    members
                        .iter()
                        .cloned()
                        .chain([structure::separator("boundary"), button("preferences")])
                        .collect(),
                );
                settings
                    .separators
                    .insert("separator:boundary".into(), structure::SeparatorStyle { size: 75.0, stretch: true });
                let visibility = settings.address_visible;
                if source != destination {
                    assert!(move_to(
                        &mut settings,
                        &layout,
                        &members,
                        Destination::Bar(destination, 0, true),
                        false
                    ));
                    assert_eq!(settings.bar(destination, &layout)[..members.len()], members);
                    assert_eq!(settings.location("preferences", &layout), Some(source));
                }
                for _ in 0..4 {
                    let before = settings.clone();
                    assert!(!move_to(
                        &mut settings,
                        &layout,
                        &members,
                        Destination::Bar(destination, 0, true),
                        false
                    ));
                    assert_eq!(settings, before);
                }
                assert_eq!(settings.address_visible, visibility);
                assert_eq!(settings.separators["separator:boundary"].size, 75.0);
                assert!(settings.separators["separator:boundary"].stretch);
                assert_eq!(
                    serde_json::from_str::<ToolbarSettings>(&serde_json::to_string(&settings).expect("encode")).expect("decode"),
                    settings
                );
                let before = settings.clone();
                settings.locked = true;
                assert!(!move_to(&mut settings, &layout, &members, Destination::Bar(source, 0, true), false));
                settings.locked = false;
                assert_eq!(settings, before);
            }
        }
    }

    #[test]
    fn blocks_can_enter_drawers_without_nesting_or_accepting_stale_members() {
        let layout = Layout::default();
        let members = vec![button("inventory"), structure::divider("inside"), button("people")];
        let mut settings = ToolbarSettings {
            composed: true,
            top: members.clone(),
            buttons: Some(Vec::new()),
            ..Default::default()
        };
        let id = drawers::create(&mut settings, Bar::Right, &layout);
        assert!(!move_to(&mut settings, &layout, &members[..1], Destination::Drawer(&id, 0), false));
        assert!(move_to(&mut settings, &layout, &members, Destination::Drawer(&id, 0), false));
        assert_eq!(settings.drawers[&id].buttons, members);
        let nested = vec![button(&id)];
        let second = drawers::create(&mut settings, Bar::Left, &layout);
        let before = settings.clone();
        assert!(!move_to(&mut settings, &layout, &nested, Destination::Drawer(&second, 0), false));
        assert_eq!(settings, before);
    }
}
