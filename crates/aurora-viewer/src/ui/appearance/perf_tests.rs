use super::*;
use std::{hint::black_box, time::Instant};

#[test]
#[ignore = "manual large-inventory frame benchmark"]
fn appearance_large_inventory_frame_benchmark() {
    for (count, folders, per_folder) in [(0, 0, 0), (3_000, 0, 0), (100, 20_000, 0), (100, 20_000, 10)] {
        let mut world = tests::wardrobe(count);
        let inv = &mut world.inventory;
        let root_id = Uuid::from_u128(1_963_001);
        inv.root = root_id;
        for folder in inv.folders.values_mut().filter(|f| f.info.parent.is_nil()) {
            folder.info.parent = root_id;
        }
        for item in inv.items.values_mut().filter(|it| it.parent.is_nil()) {
            item.parent = root_id;
        }
        let mut children: Vec<_> = inv
            .folders
            .values()
            .filter(|f| f.info.parent == root_id)
            .map(|f| f.info.id)
            .collect();
        for i in 0..folders {
            let id = Uuid::from_u128(10_000_000 + i as u128);
            children.push(id);
            let items: Vec<_> = (0..per_folder).map(|j| Uuid::from_u128(20_000_000 + i as u128 * 10 + j)).collect();
            for item in &items {
                inv.items.insert(
                    *item,
                    InvItem {
                        id: *item,
                        parent: id,
                        name: format!("Object {item}"),
                        asset_type: 6,
                        inv_type: 6,
                        ..Default::default()
                    },
                );
            }
            inv.folders.insert(
                id,
                crate::world::inventory::Folder {
                    info: aurora_net::inventory::InvFolder {
                        id,
                        parent: root_id,
                        name: format!("Delivery {i}"),
                        type_default: -1,
                        ..Default::default()
                    },
                    children: Vec::new(),
                    items,
                    state: crate::world::inventory::FetchState::Fetched,
                    library: false,
                },
            );
        }
        inv.folders.insert(
            root_id,
            crate::world::inventory::Folder {
                info: aurora_net::inventory::InvFolder {
                    id: root_id,
                    name: "Inventory".into(),
                    type_default: 8,
                    ..Default::default()
                },
                children,
                items: Vec::new(),
                state: crate::world::inventory::FetchState::Fetched,
                library: false,
            },
        );
        inv.generation += 1;
        let ctx = egui::Context::default();
        crate::theme::Theme::default().apply(&ctx, 1.0);
        let _icons = icons::Icons::load(&ctx, None);
        let mut state = AppearanceUi::default();
        for tab in [0, 1, 2] {
            state.tab = tab;
            for _ in 0..30 {
                tests::draw(&ctx, &mut world, &mut state, None);
            }
            let start = Instant::now();
            for _ in 0..100 {
                black_box(tests::draw(&ctx, &mut world, &mut state, None));
            }
            eprintln!(
                "appearance folders={folders} outfits={count} items={} tab={tab}: {:.3} ms",
                world.inventory.items.len(),
                start.elapsed().as_secs_f64() * 10.0
            );
            let start = Instant::now();
            for _ in 0..100 {
                black_box(state.prepare(&mut world.inventory, world.agent_id, &HashSet::new()));
            }
            eprintln!("prepare outfits={count} tab={tab}: {:.3} ms", start.elapsed().as_secs_f64() * 10.0);
        }
    }
}
