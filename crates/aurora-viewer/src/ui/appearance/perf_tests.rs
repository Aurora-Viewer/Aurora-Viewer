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

/// The gallery tab with 200 outfits, the Legacy tiles against the showcase
/// (wide: rail, grid and preview), the window at the showcase's size; with
/// the pointer still and over a card (its hover actions and menu rules).
#[test]
#[ignore = "manual showcase frame benchmark"]
fn appearance_showcase_frame_benchmark() {
    for showcase in [false, true] {
        for hover in [false, true] {
            let mut world = tests::wardrobe(200);
            let ctx = egui::Context::default();
            crate::theme::Theme::default().apply(&ctx, 1.0);
            let _icons = icons::Icons::load(&ctx, None);
            crate::ui::widgets::size_floater(&ctx, "appearance", Vec2::new(1080.0, 720.0));
            ctx.data_mut(|d| d.insert_persisted(egui::Id::unique("appearance_showcase_widened"), true));
            let mut state = AppearanceUi {
                showcase,
                ..Default::default()
            };
            let input = || egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1600.0, 900.0))),
                events: if hover {
                    vec![egui::Event::PointerMoved(egui::pos2(300.0, 300.0))]
                } else {
                    Vec::new()
                },
                ..Default::default()
            };
            for _ in 0..30 {
                tests::draw_output(&ctx, &mut world, &mut state, input());
            }
            let mut times = Vec::with_capacity(300);
            for _ in 0..300 {
                let start = Instant::now();
                black_box(tests::draw_output(&ctx, &mut world, &mut state, input()));
                times.push(start.elapsed().as_secs_f64() * 1000.0);
            }
            times.sort_by(f64::total_cmp);
            let mean = times.iter().sum::<f64>() / times.len() as f64;
            eprintln!(
                "gallery showcase={showcase} hover={hover} outfits=200: mean {mean:.3} ms, median {:.3} ms, p95 {:.3} ms",
                times[times.len() / 2],
                times[times.len() * 95 / 100]
            );
        }
    }
}
