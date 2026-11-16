//! Copy and Paste of the Texture tab through the shared texture copy rule.

use super::*;
use crate::build::BuildSettings;
use crate::ui::context::flags::{OBJECT_COPY, OBJECT_MODIFY, OBJECT_MOVE, OBJECT_TRANSFER};
use crate::world::inventory::actions as rules;
use aurora_net::NetCommand;
use aurora_net::build::{BuildCmd, ObjectProps};
use aurora_net::inventory::InvItem;
use std::sync::Arc;

const ASSET: Uuid = Uuid::from_u128(0x9a57_0001);
const OTHER: Uuid = Uuid::from_u128(0x9a57_0002);
const NORMAL: Uuid = Uuid::from_u128(0x9a57_0003);

struct Fixture {
    world: World,
    tool: BuildTool,
    scene: crate::scene::Scene,
    media: crate::media::MediaManager,
    idx: usize,
}

/// The demo's object 118 selected with the Edit tool: owned and created by
/// the agent, modifiable, but neither copy nor transfer, so its textures
/// are not free by themselves.
fn fixture() -> Fixture {
    let mut world = World::new(Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    for event in crate::demo::events() {
        world.apply(event);
    }
    world.inventory.items.clear();
    let idx = world.objects.index_of_uuid(&crate::demo::action_id(118)).expect("demo object");
    world.objects.get_mut(idx).expect("demo object").update_flags |= OBJECT_MOVE | OBJECT_MODIFY;
    let id = world.objects.get(idx).expect("demo object").full_id;
    let mut tool = BuildTool::default();
    let settings = BuildSettings::default();
    tool.open_build(Tool::Edit);
    tool.click_select(&mut world, &settings, Some(idx), false);
    let agent = world.agent_id;
    tool.props.insert(
        id,
        ObjectProps {
            object_id: id,
            full: true,
            owner_id: agent,
            creator_id: agent,
            ..Default::default()
        },
    );
    let scene = crate::scene::Scene::new(std::env::temp_dir(), world.avatar_lib.clone());
    Fixture {
        world,
        tool,
        scene,
        media: Default::default(),
        idx,
    }
}

impl Fixture {
    fn give(&mut self, n: u128, asset: Uuid, mask: u32) -> Uuid {
        let id = Uuid::from_u128(n);
        let agent = self.world.agent_id;
        self.world.inventory.items.insert(
            id,
            InvItem {
                id,
                asset_id: asset,
                owner: agent,
                base_mask: rules::FULL,
                owner_mask: mask,
                ..Default::default()
            },
        );
        id
    }

    fn set_texture(&mut self, texture: Uuid) {
        let o = self.world.objects.get_mut(self.idx).expect("object");
        let mut te = o.te.as_deref().cloned().expect("faces");
        for f in te.faces.iter_mut() {
            f.texture = texture;
        }
        o.te = Some(Arc::new(te));
    }

    fn texture(&self) -> Uuid {
        self.world.objects.get(self.idx).expect("object").te.as_ref().expect("faces").faces[0].texture
    }

    fn copy(&mut self) {
        let sel = selected(&self.tool, &self.world);
        assert!(copy_allowed(&self.tool, &self.world, &sel));
        let env = Env {
            images: &Default::default(),
            media: &mut self.media,
            scene: &mut self.scene,
            zoom_fraction: 0.0,
            property_lines: &mut false,
            demo: false,
        };
        let clip = copy_faces(&mut self.tool, &env, &self.world, &sel);
        self.tool.ui.face_clipboard = Some(clip);
    }

    /// Paste onto the same faces, after their texture was changed to OTHER.
    fn paste(&mut self) {
        self.set_texture(OTHER);
        self.tool.out.clear();
        let sel = selected(&self.tool, &self.world);
        let env = Env {
            images: &Default::default(),
            media: &mut self.media,
            scene: &mut self.scene,
            zoom_fraction: 0.0,
            property_lines: &mut false,
            demo: false,
        };
        paste_faces(&mut self.tool, &mut self.world, &env, &sel);
    }

    fn sent(&self, f: impl Fn(&BuildCmd) -> bool) -> bool {
        self.tool.out.iter().any(|c| matches!(c, NetCommand::Build(b) if f(b)))
    }
}

#[test]
fn a_texture_the_agent_does_not_hold_is_not_pasted() {
    let mut fx = fixture();
    fx.set_texture(ASSET);
    fx.copy();
    fx.paste();
    assert_eq!(fx.texture(), OTHER, "paste refused, the target keeps its texture");
    assert!(!fx.sent(|b| matches!(b, BuildCmd::SetTextures { .. })));
}

#[test]
fn a_full_permission_texture_is_pasted_as_before() {
    let mut fx = fixture();
    fx.set_texture(ASSET);
    fx.give(1, ASSET, rules::FULL);
    fx.copy();
    fx.paste();
    assert_eq!(fx.texture(), ASSET);
    assert!(
        !fx.sent(|b| matches!(b, BuildCmd::UpdateTaskInventory { .. })),
        "nothing goes into the contents"
    );
}

#[test]
fn a_texture_on_an_object_its_owner_created_with_every_right_is_free() {
    let mut fx = fixture();
    fx.world.objects.get_mut(fx.idx).expect("object").update_flags |= OBJECT_COPY | OBJECT_TRANSFER;
    fx.set_texture(ASSET);
    fx.copy();
    fx.paste();
    assert_eq!(fx.texture(), ASSET);
}

#[test]
fn a_no_transfer_texture_goes_into_the_target_contents_first() {
    let mut fx = fixture();
    fx.set_texture(ASSET);
    let item = fx.give(1, ASSET, rules::COPY | rules::MODIFY);
    fx.copy();
    // the contents are not listed yet: asked for, nothing pasted
    fx.paste();
    assert_eq!(fx.texture(), OTHER);
    assert!(fx.sent(|b| matches!(b, BuildCmd::RequestTaskInventory { .. })));
    let object = fx.world.objects.get(fx.idx).expect("object").full_id;
    fx.tool.on_task_inventory(&fx.world, object, Some(1), Vec::new());
    fx.paste();
    assert_eq!(fx.texture(), ASSET);
    assert!(fx.sent(|b| matches!(b, BuildCmd::UpdateTaskInventory { item: task, .. } if task.item_id == item)));
    // the item left the inventory since the copy: refused
    fx.world.inventory.items.remove(&item);
    fx.paste();
    assert_eq!(fx.texture(), OTHER);
}

#[test]
fn a_normal_map_the_agent_cannot_copy_leaves_the_target_its_own() {
    let mut fx = fixture();
    fx.give(1, OTHER, rules::FULL);
    let object = fx.world.objects.get(fx.idx).expect("object").full_id;
    let te = fx.world.objects.get(fx.idx).expect("object").te.as_ref().expect("faces").faces[0];
    let material = LegacyMaterial {
        normal_map: NORMAL,
        ..Default::default()
    };
    fx.tool.ui.legacy_pending.insert((object, 0), (te.material_id, Some(material)));
    fx.copy();
    let clip = fx.tool.ui.face_clipboard.clone().expect("clip");
    let face0 = &clip[0];
    assert!(face0.normal_kept && !face0.specular_kept);
    assert_eq!(face0.legacy.as_ref().map(|m| m.normal_map), Some(Uuid::nil()));
    fx.give(2, NORMAL, rules::COPY);
    fx.copy();
    let clip = fx.tool.ui.face_clipboard.clone().expect("clip");
    assert!(!clip[0].normal_kept, "a copyable normal map is copied");
    assert_eq!(clip[0].legacy.as_ref().map(|m| m.normal_map), Some(NORMAL));
}

#[test]
fn a_material_is_copied_only_from_an_object_with_every_right() {
    let mut fx = fixture();
    fx.world.objects.get_mut(fx.idx).expect("object").extra.render_materials = vec![(u8::MAX, ASSET)];
    let sel = selected(&fx.tool, &fx.world);
    assert!(!copy_allowed(&fx.tool, &fx.world, &sel));
    fx.world.objects.get_mut(fx.idx).expect("object").update_flags |= OBJECT_COPY | OBJECT_TRANSFER;
    assert!(copy_allowed(&fx.tool, &fx.world, &sel));
}
