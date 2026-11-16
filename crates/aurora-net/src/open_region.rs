//! The region settings of Aurora-Sim based grids: the event queue's
//! OpenRegionInfo message, read over LLWorld::refreshLimits' OpenSim
//! defaults (OpenRegionInfoUpdate::post, indra/newview/llpanelopenregionsettings.cpp,
//! originally LGPL 2.1, Copyright (C) 2011 Patrick Sapinski, Matthew
//! Beardmore), and the body the Region / Estate window's "Region settings"
//! tab POSTs to the DispatchOpenRegionSettings capability
//! (LLPanelRegionOpenSettingsInfo::onClickOrs, indra/newview/llfloaterregioninfo.cpp).

use aurora_llsd::Llsd;

/// The capability the tab needs: without it the tab is not shown
/// (LLFloaterRegionInfo::postBuild).
pub const CAP: &str = "DispatchOpenRegionSettings";

/// What the tab shows and sends.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OpenRegionSettings {
    /// Default draw distance, in metres (-1: none given).
    pub draw_distance: f32,
    pub force_draw_distance: bool,
    pub max_drag_distance: f32,
    pub max_prim_scale: f32,
    pub min_prim_scale: f32,
    pub max_phys_prim_scale: f32,
    pub max_hollow_size: f32,
    pub min_hole_size: f32,
    pub max_link_count: i32,
    pub max_link_count_phys: i32,
    /// -1: no limit.
    pub max_inventory_items_transfer: i32,
    /// -1: none given.
    pub terrain_detail_scale: f32,
    /// 2 shows the name tags, 1 briefly, 0 hides them.
    pub show_tags: i32,
    /// MaxGroups; None until the region names one (the tab then shows the
    /// account's limit, gMaxAgentGroups).
    pub max_groups: Option<i32>,
    pub render_water: bool,
    pub allow_minimap: bool,
    pub allow_physical_prims: bool,
    pub enforce_max_build: bool,
    pub allow_parcel_windlight: bool,
    /// ToggleTeenMode: not on the tab, sent back as it came.
    pub enable_teen_mode: bool,
}

impl Default for OpenRegionSettings {
    /// LLWorld::refreshLimits on OpenSim (OS_DEFAULT_MAX_PRIM_SCALE,
    /// OS_MIN_PRIM_SCALE, OS_OBJECT_MAX_HOLLOW_SIZE, OS_OBJECT_MIN_HOLE_SIZE).
    fn default() -> Self {
        OpenRegionSettings {
            draw_distance: -1.0,
            force_draw_distance: false,
            max_drag_distance: 10_000.0,
            max_prim_scale: 256.0,
            min_prim_scale: 0.001,
            max_phys_prim_scale: 256.0,
            max_hollow_size: 99.0,
            min_hole_size: 0.01,
            max_link_count: 10_000,
            max_link_count_phys: 10_000,
            max_inventory_items_transfer: 42,
            terrain_detail_scale: -1.0,
            show_tags: 2,
            max_groups: None,
            render_water: true,
            allow_minimap: true,
            allow_physical_prims: true,
            enforce_max_build: false,
            allow_parcel_windlight: true,
            enable_teen_mode: false,
        }
    }
}

impl OpenRegionSettings {
    /// OpenRegionInfoUpdate::post: each key the message has replaces the
    /// default; a flag is on when its integer is 1.
    pub fn from_llsd(body: &Llsd) -> Self {
        let mut s = OpenRegionSettings::default();
        let has = |key: &str| !body[key].is_undef();
        let flag = |key: &str| body[key].as_i32() == 1;
        // OpenRegionSettingsEnableDrawDistance is on by default
        if has("DrawDistance") {
            s.draw_distance = body["DrawDistance"].as_f32();
        }
        if has("ForceDrawDistance") {
            s.force_draw_distance = flag("ForceDrawDistance");
        }
        if has("TerrainDetailScale") {
            s.terrain_detail_scale = body["TerrainDetailScale"].as_f32();
        }
        if has("MaxDragDistance") {
            s.max_drag_distance = body["MaxDragDistance"].as_f32();
        }
        if has("MinHoleSize") {
            s.min_hole_size = body["MinHoleSize"].as_f32();
        }
        if has("MaxHollowSize") {
            s.max_hollow_size = body["MaxHollowSize"].as_f32();
        }
        if has("MaxInventoryItemsTransfer") {
            s.max_inventory_items_transfer = body["MaxInventoryItemsTransfer"].as_i32();
        }
        if has("MaxLinkCount") {
            s.max_link_count = body["MaxLinkCount"].as_i32();
        }
        if has("MaxLinkCountPhys") {
            s.max_link_count_phys = body["MaxLinkCountPhys"].as_i32();
        }
        if has("MaxPrimScale") {
            s.max_prim_scale = body["MaxPrimScale"].as_f32();
        }
        if has("MaxPhysPrimScale") {
            s.max_phys_prim_scale = body["MaxPhysPrimScale"].as_f32();
        }
        if has("MinPrimScale") {
            s.min_prim_scale = body["MinPrimScale"].as_f32();
        }
        if has("RenderWater") {
            s.render_water = flag("RenderWater");
        }
        if has("ToggleTeenMode") {
            s.enable_teen_mode = flag("ToggleTeenMode");
        }
        if has("ShowTags") {
            s.show_tags = body["ShowTags"].as_i32();
        }
        if has("EnforceMaxBuild") {
            s.enforce_max_build = flag("EnforceMaxBuild");
        }
        if has("MaxGroups") {
            s.max_groups = Some(body["MaxGroups"].as_i32());
        }
        if has("AllowMinimap") {
            s.allow_minimap = flag("AllowMinimap");
        }
        if has("AllowPhysicalPrims") {
            s.allow_physical_prims = flag("AllowPhysicalPrims");
        }
        if has("AllowParcelWindLight") {
            s.allow_parcel_windlight = flag("AllowParcelWindLight");
        }
        s
    }

    /// onClickOrs: every value, with the LLSD type the Legacy Viewer gives
    /// it (the link counts, tags and groups as reals).
    pub fn body(&self) -> Llsd {
        aurora_llsd::llsd_map! {
            "draw_distance" => self.draw_distance as i32,
            "force_draw_distance" => self.force_draw_distance,
            "allow_minimap" => self.allow_minimap,
            "allow_physical_prims" => self.allow_physical_prims,
            "max_drag_distance" => f64::from(self.max_drag_distance),
            "min_hole_size" => f64::from(self.min_hole_size),
            "max_hollow_size" => f64::from(self.max_hollow_size),
            "max_inventory_items_transfer" => self.max_inventory_items_transfer,
            "max_link_count" => f64::from(self.max_link_count),
            "max_link_count_phys" => f64::from(self.max_link_count_phys),
            "max_phys_prim_scale" => f64::from(self.max_phys_prim_scale),
            "max_prim_scale" => f64::from(self.max_prim_scale),
            "min_prim_scale" => f64::from(self.min_prim_scale),
            "render_water" => self.render_water,
            "terrain_detail_scale" => f64::from(self.terrain_detail_scale),
            "show_tags" => f64::from(self.show_tags),
            "max_groups" => f64::from(self.max_groups.unwrap_or(-1)),
            "allow_parcel_windlight" => self.allow_parcel_windlight,
            "enable_teen_mode" => self.enable_teen_mode,
            "enforce_max_build" => self.enforce_max_build,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_message_replaces_only_the_keys_it_has() {
        let body = aurora_llsd::llsd_map! {
            "DrawDistance" => 256.0,
            "ForceDrawDistance" => 1,
            "MaxLinkCount" => 255,
            "AllowMinimap" => 0,
            "ShowTags" => 1,
            "MaxGroups" => 60,
            "RenderWater" => true,
        };
        let s = OpenRegionSettings::from_llsd(&body);
        assert_eq!(s.draw_distance, 256.0);
        assert!(s.force_draw_distance);
        assert_eq!(s.max_link_count, 255);
        assert!(!s.allow_minimap);
        assert_eq!(s.show_tags, 1);
        assert_eq!(s.max_groups, Some(60));
        assert!(s.render_water, "a boolean true reads as 1");
        // the rest keeps refreshLimits' OpenSim values
        let d = OpenRegionSettings::default();
        assert_eq!(s.max_link_count_phys, d.max_link_count_phys);
        assert_eq!(s.max_prim_scale, 256.0);
        assert_eq!(s.min_hole_size, 0.01);
        assert_eq!(s.max_inventory_items_transfer, 42);
        assert!(s.allow_physical_prims && s.allow_parcel_windlight);
        assert_eq!(OpenRegionSettings::from_llsd(&Llsd::Undef), d);
    }

    #[test]
    fn the_body_follows_on_click_ors() {
        let s = OpenRegionSettings {
            draw_distance: 128.7,
            max_link_count: 512,
            max_groups: None,
            ..OpenRegionSettings::default()
        };
        let b = s.body();
        let map = b.as_map().expect("a map");
        assert_eq!(map.len(), 20);
        assert!(matches!(b["draw_distance"], Llsd::Integer(128)));
        assert!(matches!(b["force_draw_distance"], Llsd::Boolean(false)));
        assert!(matches!(b["max_inventory_items_transfer"], Llsd::Integer(42)));
        assert!(matches!(b["max_link_count"], Llsd::Real(r) if r == 512.0));
        assert!(matches!(b["show_tags"], Llsd::Real(r) if r == 2.0));
        assert!(matches!(b["max_groups"], Llsd::Real(r) if r == -1.0));
        assert!(matches!(b["enable_teen_mode"], Llsd::Boolean(false)));
        assert!(matches!(b["min_prim_scale"], Llsd::Real(_)));
    }
}
