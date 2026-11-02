//! "Region / Estate" in Aurora's look (LLFloaterRegionInfo,
//! newview/llfloaterregioninfo.cpp, floater_region_info.xml, originally
//! LGPL 2.1): the agent's region and its estate at a glance in a header
//! shared by three pages, "Overview" for everyone, "Manage" for the
//! settings grouped by intent and "People" for the estate's lists (hidden
//! from a resident, whom the simulator does not send them). The pieces are
//! About Land's (`ui/sheet.rs`); the data are `world/estate.rs`'s. Who may
//! manage the estate can look at it as a resident would ("View as").

mod advanced;
mod classic;
mod confirm;
mod draft;
mod estate_wide;
mod interventions;
mod manage;
mod overview;
mod people;
mod surface;
mod top_objects;

use crate::theme::Palette;
use crate::ui::sheet::{self, PageTab};
use crate::ui::texture_picker::TexturePicker;
use crate::ui::widgets::{self, Floater};
use crate::world::World;
use crate::world::estate::{self, Role};
pub use advanced::MaterialInfo;
use aurora_net::RegionHandle;
pub use classic::{Tab as ClassicTab, WINDOW as CLASSIC_WINDOW};
pub(crate) use confirm::Ask;
use egui::Vec2;
pub(crate) use estate_wide::{Dialog as EstateWideDialog, XpList};
pub(crate) use interventions::{Dialog as InterventionDialog, Home};
pub use people::PeopleDemo;
pub(crate) use people::{List as PeopleList, PeopleAsk};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

/// Id of the window (its place and size are kept by the layout).
pub const WINDOW: &str = "region_estate";

/// The three pages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Page {
    #[default]
    Overview,
    Manage,
    People,
}

impl Page {
    const ALL: [Page; 3] = [Page::Overview, Page::Manage, Page::People];

    fn label(self) -> &'static str {
        match self {
            Page::Overview => aurora_i18n::tr!("ui-land-overview-tab"),
            Page::Manage => aurora_i18n::tr!("ui-land-overview-manage-tab"),
            Page::People => aurora_i18n::tr!("region-people-tab"),
        }
    }

    fn icon(self) -> &'static str {
        match self {
            Page::Overview => "eye-filled",
            Page::Manage => "adjustments-horizontal-filled",
            Page::People => "users",
        }
    }
}

/// Default size: About Land's, the mockup's 1000 × 720.
const SIZE: Vec2 = Vec2::new(1000.0, 720.0);
/// The compact layout (no picture, one column) goes down to this.
const MIN_SIZE: Vec2 = Vec2::new(480.0, 480.0);
/// Below this width the window is laid out compact.
const COMPACT_W: f32 = 620.0;

/// The name of a role, as "View as" and the header say it.
pub(crate) fn role_name(role: Role) -> &'static str {
    match role {
        Role::Owner => aurora_i18n::tr!("region-role-owner"),
        Role::Manager => aurora_i18n::tr!("region-role-manager"),
        Role::Resident => aurora_i18n::tr!("region-role-resident"),
    }
}

/// What the pages read about the region, its estate and the agent.
#[derive(Clone)]
pub(crate) struct View {
    pub handle: RegionHandle,
    /// The agent's real role, and the one the window shows ("View as").
    pub role: Role,
    pub shown: Role,
    pub name: String,
    pub sim_access: u8,
    /// Region flags: RegionInfo's once answered, else the handshake's.
    pub flags: u64,
    /// The estate owner: `estateupdateinfo`'s, else the covenant's, else
    /// the region's (RegionHandshake SimOwner).
    pub owner: Uuid,
    pub product: String,
    pub same_region: bool,
}

impl View {
    /// What a resident does not see: the estate's access, its managers and
    /// the interventions.
    pub fn manages(&self) -> bool {
        self.shown != Role::Resident
    }

    pub fn region_flag(&self, flag: u32) -> bool {
        self.flags & u64::from(flag) != 0
    }
}

/// What the window asks the app to open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegionAction {
    AboutLand,
    WorldMap,
    /// "Edit the day cycle…": the day editor on the region's day.
    CustomizeEnvironment,
    /// A spawn point was added: the selection is let go (deselectAll).
    Deselect,
}

#[derive(Default)]
pub struct RegionUi {
    page: Page,
    /// The owner (or a manager) looks at the region as a resident would.
    as_resident: bool,
    /// The covenant shown in full.
    covenant_open: bool,
    was_open: bool,
    /// Residents whose profile picture the header waits for.
    pub wanted_pics: HashSet<Uuid>,
    /// Textures the terrain's swatches and the picker wait for.
    pub wanted_images: HashSet<Uuid>,
    /// The terrain's ids asked as materials, and what the app's material
    /// store says of them (PBR terrain).
    pub wanted_materials: HashSet<Uuid>,
    pub material_info: HashMap<Uuid, MaterialInfo>,
    /// The root objects selected in the world, in the window's region,
    /// when they are all prims (the telehub's Connect and Add).
    pub selection: Vec<u32>,
    /// An action button was pressed (taken by the app).
    pub action: Option<RegionAction>,
    /// "Manage": its sections' changes and navigation, for `manage_region`.
    manage: manage::ManageState,
    manage_region: Option<RegionHandle>,
    /// "People": its lists' searches, selections, pickers and dialogs.
    people: people::PeopleState,
    /// "Interventions": the resident picker and the dialog on screen.
    interventions: interventions::InterventionsState,
    /// The Top scripts / Top colliders window.
    top: top_objects::TopState,
    /// The rules' and terrain's Advanced folds.
    advanced: advanced::AdvancedState,
    /// The classic window's tab, and AURORA_DEMO_REGION's `classic`.
    classic_tab: classic::Tab,
    force_classic: bool,
    /// Environment, Experiences and Covenant: their pickers and dialogs.
    estate_wide: estate_wide::EstateWideState,
    /// The question or notice on screen.
    ask: Option<Ask>,
    /// The terrain textures' picker.
    texture: TexturePicker,
    surface: surface::SurfaceUi,
    /// Preferences read for this frame (ChangeObjectBonusFactor's
    /// "ignore", RegionCheckTextureHeights) and written back by the dialogs.
    confirm_object_bonus: bool,
    check_texture_heights: bool,
}

/// What AURORA_DEMO_REGION opens on Manage (`demo_region.rs`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ManageDemo {
    /// The section scrolled to: access, rules or terrain.
    pub section: Option<u8>,
    /// Changes waiting for "Apply" in that section.
    pub pending: bool,
    /// The question shown.
    pub ask: Option<Ask>,
    /// The section's Advanced fold opened (rules: the telehub; terrain:
    /// transforms and RAW file).
    pub advanced: bool,
    /// The terrain turned to PBR materials.
    pub pbr: bool,
    /// Open a terrain material picker with the demo's tiled material.
    pub pick_material: bool,
    /// An existing PBR region, with an optional live edit/cancel/apply scenario.
    pub pbr_ground: bool,
    pub surface_demo: u8,
}

/// The menus' and About Land's request to open the window.
pub fn request_open(ctx: &egui::Context) {
    ctx.data_mut(|d| d.insert_temp(egui::Id::unique("aurora_open_region_estate"), true));
}

pub fn take_open_request(ctx: &egui::Context) -> bool {
    ctx.data_mut(|d| d.remove_temp::<bool>(egui::Id::unique("aurora_open_region_estate")))
        .unwrap_or(false)
}

impl RegionUi {
    /// Prepare the selected PBR preview and validate the server and draft slots
    /// through the same streamer (LLPanelRegionTerrainInfo::validateMaterials).
    pub(crate) fn prepare_materials(&mut self, ctx: &egui::Context, scene: &mut crate::scene::Scene) {
        use crate::scene::meshes::MaterialStatus;

        self.texture.prepare_material_preview(ctx, scene);
        if self.advanced.shown_kind == Some(advanced::TerrainKind::Materials)
            && let Some(draft) = self.manage.terrain.editing()
        {
            // Keep loading after OK even if the user leaves the terrain page.
            self.wanted_materials.extend(draft.textures.into_iter().filter(|id| !id.is_nil()));
        }
        for id in &self.wanted_materials {
            let info = match scene.materials.resolve(id) {
                MaterialStatus::Pending => MaterialInfo::Pending,
                MaterialStatus::Missing => MaterialInfo::Missing,
                MaterialStatus::Loaded(m) => MaterialInfo::Loaded {
                    double_sided: m.double_sided,
                    blend: m.alpha_mode == aurora_assets::AlphaMode::Blend,
                },
            };
            self.material_info.insert(*id, info);
        }
        self.surface.prepare(ctx, scene, &mut self.wanted_images);
    }

    /// AURORA_DEMO_REGION: the page and "View as" of a capture, and on
    /// Manage its section, changes and question.
    pub fn demo(&mut self, page: Page, as_resident: bool, manage: ManageDemo) {
        self.page = page;
        self.as_resident = as_resident;
        self.manage.demo = Some(manage);
    }

    /// AURORA_DEMO_REGION on Manage › Interventions: the dialog shown.
    pub fn demo_intervention(&mut self, d: InterventionDialog) {
        self.interventions.demo = Some(d);
    }

    /// AURORA_DEMO_REGION on Manage › Scripts and physics: the Top window
    /// opened on that report.
    pub fn demo_top(&mut self, report: u32) {
        self.top.demo = Some(report);
    }

    /// AURORA_DEMO_REGION on Manage › Environment, Experiences or
    /// Covenant: the picker or question shown.
    pub fn demo_estate_wide(&mut self, d: EstateWideDialog) {
        self.estate_wide.demo = Some(d);
    }

    /// AURORA_DEMO_REGION's `classic`: the Legacy Viewer's tabs, on that one.
    pub fn demo_classic(&mut self, tab: ClassicTab) {
        self.force_classic = true;
        self.classic_tab = tab;
    }

    /// AURORA_DEMO_REGION on People: a row ticked, a dialog or the picker.
    pub fn demo_people(&mut self, people: PeopleDemo) {
        self.people.demo = Some(people);
    }

    /// Show the floater; `open` is the panel flag, `map` the mini-map's
    /// image of the region, `avatars` the profile pictures, `images` the
    /// textures. True when `settings` changed.
    #[allow(clippy::too_many_arguments)]
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        p: &Palette,
        world: &mut World,
        avatars: &HashMap<Uuid, egui::TextureHandle>,
        images: &HashMap<Uuid, egui::TextureHandle>,
        map: Option<&egui::TextureHandle>,
        settings: &mut crate::settings::Settings,
        open: &mut bool,
    ) -> bool {
        let handle = world.main_region.filter(|_| *open);
        let Some(handle) = handle else {
            surface::clear_preview(self, world);
            if self.was_open {
                self.was_open = false;
                self.ask = None;
                self.texture.open = false;
                self.people.reset();
                self.interventions.reset();
                self.top.reset();
                self.estate_wide.reset();
                self.advanced.reset();
                world.estate.close();
            }
            return false;
        };
        // a single window: reopened, or after a teleport, it shows the
        // region the agent is in (onOpen, onRegionChanged)
        if !self.was_open || world.estate.open != Some(handle) {
            self.was_open = true;
            world.refresh_region_window(handle, true);
        }
        // what was being changed belongs to the region it was changed for
        if self.manage_region != Some(handle) {
            surface::clear_preview(self, world);
            self.texture.open = false;
            self.surface = Default::default();
            let demo = self.manage.demo.take();
            self.manage = manage::ManageState {
                demo,
                ..Default::default()
            };
            self.people.reset();
            self.interventions.reset();
            self.top.reset();
            self.estate_wide.reset();
            self.advanced.reset();
            self.wanted_materials.clear();
            self.manage_region = Some(handle);
            self.ask = None;
        }
        self.confirm_object_bonus = settings.confirm_object_bonus;
        self.check_texture_heights = settings.region_check_texture_heights;
        let view = self.view(world, handle);
        if view.role == Role::Resident {
            self.as_resident = false;
        }
        let screen = ctx.content_rect();
        // Preferences › Aurora: the Legacy Viewer's eight tabs, no "View as"
        let classic_look = settings.classic_region_estate || self.force_classic;
        if classic_look {
            self.as_resident = false;
            let mut floater = Floater::new(
                classic::WINDOW,
                aurora_i18n::tr!("region-title"),
                egui::pos2(screen.center().x - classic::SIZE.x / 2.0, 80.0),
                classic::SIZE,
            )
            .help(aurora_i18n::tr!("region-help"));
            floater.min_size = classic::MIN_SIZE;
            floater.show(ctx, p, open, |ui| {
                let pics = overview::Pictures { avatars, map };
                classic::body(ui, p, self, &view, world, &pics, avatars, images);
            });
        }
        let mut floater = Floater::new(
            WINDOW,
            aurora_i18n::tr!("region-title"),
            egui::pos2(screen.center().x - SIZE.x / 2.0, 80.0),
            SIZE,
        )
        .help(aurora_i18n::tr!("region-help"));
        floater.min_size = MIN_SIZE;
        if !classic_look {
            floater.show(ctx, p, open, |ui| {
                let compact = ui.available_width() < COMPACT_W;
                // a resident is not sent the lists: no People page
                let pages: Vec<Page> = Page::ALL
                    .into_iter()
                    .filter(|pg| *pg != Page::People || view.role != Role::Resident)
                    .collect();
                let tabs: Vec<PageTab<'_, Page>> = pages
                    .iter()
                    .map(|pg| PageTab {
                        page: *pg,
                        icon: pg.icon(),
                        label: pg.label(),
                        disabled: None,
                    })
                    .collect();
                let role = (view.role != Role::Resident && !compact).then(|| sheet::ViewAs {
                    mine: role_name(view.role),
                    other: role_name(Role::Resident),
                    tip: aurora_i18n::tr!("region-view-as-tip"),
                    // the mockup lights the choice in the accent
                    accent: true,
                });
                // a resident's window has no People page
                if !pages.contains(&self.page) {
                    self.page = Page::Overview;
                }
                sheet::page_tabs(ui, p, &tabs, &mut self.page, role, &mut self.as_resident);
                ui.add_space(8.0);
                let view = View {
                    shown: if self.as_resident { Role::Resident } else { view.role },
                    ..view.clone()
                };
                let pics = overview::Pictures { avatars, map };
                match self.page {
                    Page::Manage => {
                        // compact, the header keeps only the region's name
                        overview::header(ui, p, self, &view, world, &pics, compact, compact);
                        // the mockup's 12 px under the header, item spacing included
                        ui.add_space((12.0 - ui.spacing().item_spacing.y).max(0.0));
                        manage::demo(self, &view, world);
                        manage::show(ui, p, self, &view, world, images, compact);
                    }
                    Page::People => {
                        egui::ScrollArea::vertical()
                            .id_salt("region_estate_people")
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                ui.set_width(ui.available_width() - 6.0);
                                // compact, the header keeps only the region's name
                                overview::header(ui, p, self, &view, world, &pics, compact, compact);
                                ui.add_space((12.0 - ui.spacing().item_spacing.y).max(0.0));
                                people::show(ui, p, self, &view, world, avatars, images, compact);
                                ui.add_space(4.0);
                            });
                    }
                    _ => {
                        egui::ScrollArea::vertical()
                            .id_salt("region_estate_overview")
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                ui.set_width(ui.available_width() - 6.0);
                                overview::header(ui, p, self, &view, world, &pics, compact, false);
                                overview::show(ui, p, self, &view, world, compact);
                                ui.add_space(4.0);
                            });
                    }
                }
            });
        }
        let view = View {
            shown: if self.as_resident { Role::Resident } else { view.role },
            ..view
        };
        if let Some(asset) = self.texture.show(ctx, p, world, images) {
            manage::picked_texture(self, &view, world, asset);
        } else if !self.texture.open {
            if self.texture.cancelled() {
                surface::cancel_picker(self, &view, world);
            } else {
                self.manage.picking = None;
                self.surface.pick_before = None;
            }
        } else if let Some(asset) = self.texture.take_preview() {
            surface::picker_choice(self, &view, world, asset, false);
        }
        surface::demo_step(self, &view, world);
        surface::update_preview(self, &view, world);
        self.wanted_images.extend(self.texture.wanted_images.drain());
        people::dialogs(ctx, p, self, &view, world);
        top_objects::show(ctx, p, self, &view, world);
        interventions::dialogs(ctx, p, self, &view, world);
        estate_wide::dialogs(ctx, p, self, &view, world);
        let changed = confirm::show(ctx, p, self, &view, world);
        if changed {
            settings.confirm_object_bonus = self.confirm_object_bonus;
            settings.region_check_texture_heights = self.check_texture_heights;
        }
        changed
    }

    fn view(&self, world: &World, handle: RegionHandle) -> View {
        let info = world.regions.get(&handle).and_then(|r| r.info.clone());
        let details = world.estate.region.as_ref();
        let covenant_owner = world
            .land
            .covenant
            .as_ref()
            .filter(|c| c.handle == handle && !c.estate_owner.is_nil())
            .map(|c| c.estate_owner);
        let owner = world
            .estate
            .estate
            .as_ref()
            .map(|e| e.owner)
            .filter(|o| !o.is_nil())
            .or(covenant_owner)
            .or_else(|| info.as_ref().map(|i| i.owner))
            .unwrap_or_default();
        let manager = info.as_ref().is_some_and(|i| i.is_estate_manager);
        let role = estate::role(world.agent_id, owner, manager);
        View {
            handle,
            role,
            shown: role,
            name: details
                .map(|d| d.name.clone())
                .or_else(|| info.as_ref().map(|i| i.name.clone()))
                .unwrap_or_default(),
            sim_access: details.map_or_else(|| info.as_ref().map_or(0, |i| i.sim_access), |d| d.sim_access),
            flags: details.map_or_else(|| info.as_ref().map_or(0, |i| u64::from(i.region_flags)), |d| d.flags),
            owner,
            product: details
                .map(|d| d.product_name.clone())
                .filter(|p| !p.is_empty())
                .or_else(|| info.as_ref().map(|i| i.product_name.clone()))
                .unwrap_or_default(),
            same_region: true,
        }
    }
}

/// A flat button of the window, greyed when `enabled` is false.
fn action(ui: &mut egui::Ui, p: &Palette, icon: &str, label: &str, kind: widgets::ActionKind, enabled: bool) -> egui::Response {
    widgets::action_button_if(ui, p, icon, label, kind, Vec2::new(0.0, overview::ACTION_H), enabled)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_role_is_named() {
        assert_eq!(role_name(Role::Owner), aurora_i18n::tr!("region-role-owner"));
        assert_eq!(role_name(Role::Resident), aurora_i18n::tr!("region-role-resident"));
    }
}
