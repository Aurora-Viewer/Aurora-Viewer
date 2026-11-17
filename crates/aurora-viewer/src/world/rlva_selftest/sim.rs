//! The offline simulator of the self-test: what a region answers to the
//! test's own commands when there is no grid (demo mode, unit tests).
//! In demo mode the application's own demo answers come after it (`whole`
//! false): only what they lack is answered here. In the unit tests it stands
//! alone (`whole` true), with the same building blocks as the demo
//! (`build::demo_sim`, `demo::outfit_attachment`, `demo::sit_on_prim`).
use super::super::appearance;
use super::{FOLDER, World};
use aurora_net::build::BuildCmd;
use aurora_net::outfits::{OutfitLink, OutfitMutation};
use aurora_net::{ChatMessage, ChatSourceType, ChatType, NetCommand, NetEvent};
use glam::Vec3;
use std::collections::HashSet;
use uuid::Uuid;

#[derive(Default)]
pub struct Sim {
    build: crate::build::demo_sim::DemoSim,
    /// The prim the avatar sits on: its position and size.
    seat: Option<(Vec3, Vec3)>,
    /// The objects a relay script was saved in.
    relays: HashSet<Uuid>,
}

/// One of the test's own inventory objects.
fn own(world: &World, item: &Uuid) -> bool {
    world.inventory.items.get(item).is_some_and(|i| i.name.starts_with(FOLDER))
}

/// A simulator links what arrives worn into the Current Outfit folder
/// (through the viewer on a grid, LLAttachmentsMgr::
/// linkRecentlyArrivedAttachments), and unlinks what is taken off: without
/// it the next outfit update would take the test objects off as strays.
fn relink(world: &mut World, items: &[Uuid], worn: bool) {
    let Some(cof) = appearance::cof(&world.inventory) else {
        return;
    };
    let mut links = appearance::folder_links(&world.inventory, cof);
    let before = links.len();
    links.retain(|l| worn || !items.contains(&l.target));
    for item in items {
        if worn
            && !links.iter().any(|l| l.target == *item)
            && let Some(it) = world.inventory.items.get(item)
        {
            links.push(OutfitLink {
                target: *item,
                name: it.name.clone(),
                desc: String::new(),
                folder: false,
                inv_type: it.inv_type,
            });
        }
    }
    if links.len() != before {
        let agent = world.agent_id;
        let _ = appearance::demo_mutate(
            &mut world.inventory,
            agent,
            &OutfitMutation {
                folder: cof,
                create: None,
                links,
                cof: None,
            },
        );
    }
}

impl Sim {
    /// The events `command` is answered with; None when it is not this
    /// simulator's to answer.
    pub fn reply(&mut self, world: &mut World, command: &NetCommand, whole: bool) -> Option<Vec<NetEvent>> {
        let handle = world.main_region.unwrap_or_default();
        match command {
            // the copies of the Library object, under their new names
            NetCommand::FinishInventoryCopies { request, reply } if reply.copies.iter().all(|c| c.name.starts_with(FOLDER)) => {
                let mut items = Vec::new();
                for copy in &reply.copies {
                    let Some(mut item) = world.inventory.items.get(&copy.item).cloned() else {
                        return Some(vec![NetEvent::InventoryEdited {
                            request: *request,
                            result: Err("the Library object is gone".to_owned()),
                        }]);
                    };
                    item.id = Uuid::new_v4();
                    item.parent = copy.parent;
                    item.name.clone_from(&copy.name);
                    item.owner = world.agent_id;
                    items.push(item);
                }
                let mut completed = reply.clone();
                completed.copies.clear();
                completed.contents.clear();
                completed.folders.clear();
                Some(vec![
                    NetEvent::InventoryUpdated {
                        bulk: false,
                        folders: Vec::new(),
                        items,
                    },
                    NetEvent::InventoryEdited {
                        request: *request,
                        result: Ok(completed),
                    },
                ])
            }
            NetCommand::RezAttachments(items) if items.iter().all(|i| own(world, &i.item_id)) => {
                let ids: Vec<Uuid> = items.iter().map(|i| i.item_id).collect();
                relink(world, &ids, true);
                whole.then(|| {
                    vec![NetEvent::ObjectUpdates {
                        handle,
                        objects: items.iter().map(crate::demo::outfit_attachment).collect(),
                    }]
                })
            }
            NetCommand::DetachAttachments(items) if items.iter().all(|i| own(world, i)) => {
                relink(world, items, false);
                whole.then(|| {
                    vec![NetEvent::ObjectsKilled {
                        handle,
                        local_ids: items
                            .iter()
                            .filter_map(|id| world.worn_attachment(id))
                            .filter_map(|i| world.objects.get(i))
                            .map(|o| o.key.local_id)
                            .collect(),
                    }]
                })
            }
            _ if !whole => None,
            NetCommand::EditInventory { request, change } => {
                let agent = world.agent_id;
                let result = super::super::inventory::actions::demo_mutate(&mut world.inventory, agent, change.clone());
                Some(vec![NetEvent::InventoryEdited { request: *request, result }])
            }
            NetCommand::UpdateOutfit { request, change } => {
                let agent = world.agent_id;
                let result = appearance::demo_mutate(&mut world.inventory, agent, change);
                Some(vec![NetEvent::OutfitUpdated { request: *request, result }])
            }
            NetCommand::RezAttachments(items) => Some(vec![NetEvent::ObjectUpdates {
                handle,
                objects: items.iter().map(crate::demo::outfit_attachment).collect(),
            }]),
            NetCommand::DetachAttachments(items) => Some(vec![NetEvent::ObjectsKilled {
                handle,
                local_ids: items
                    .iter()
                    .filter_map(|id| world.worn_attachment(id))
                    .filter_map(|i| world.objects.get(i))
                    .map(|o| o.key.local_id)
                    .collect(),
            }]),
            NetCommand::RequestTaskInventory { handle, local_id, object } => Some(self.build.reply(
                world,
                &BuildCmd::RequestTaskInventory {
                    handle: *handle,
                    local_id: *local_id,
                    object_id: *object,
                },
            )),
            NetCommand::SaveScript { request, target, .. } => {
                self.relays.insert(target.object);
                Some(vec![NetEvent::ScriptSaved {
                    request: *request,
                    target: *target,
                    result: Ok(aurora_net::scripts::ScriptSaved {
                        asset: Uuid::new_v4(),
                        compiled: true,
                        checked: true,
                        errors: Vec::new(),
                    }),
                }])
            }
            // the relay script: what its owner says on its channel, it says
            NetCommand::ScriptDialogReply { channel, label, .. } => Some(
                self.relays
                    .iter()
                    .filter(|object| super::super::bridge::private_channel(**object) == *channel)
                    .filter(|object| world.objects.index_of_uuid(object).is_some())
                    .map(|object| {
                        NetEvent::Chat(ChatMessage {
                            from_name: FOLDER.to_owned(),
                            source_id: *object,
                            owner_id: world.agent_id,
                            source_type: ChatSourceType::Object,
                            chat_type: ChatType::OwnerSay,
                            region: world.main_region,
                            position: world.agent.position,
                            message: label.clone(),
                        })
                    })
                    .collect(),
            ),
            NetCommand::RequestSit { target, .. } => {
                let seat = world.objects.index_of_uuid(target).and_then(|i| world.objects.get(i))?;
                let (local_id, position, size) = (seat.key.local_id, seat.position, seat.scale);
                self.seat = Some((position, size));
                Some(crate::demo::sit_on_prim(local_id, *target, size))
            }
            NetCommand::OneShotControl(flags) if flags & aurora_net::control::STAND_UP != 0 => Some(
                self.seat
                    .take()
                    .map_or_else(Vec::new, |(position, size)| crate::demo::stand_from_prim(position, size)),
            ),
            NetCommand::Build(build) => Some(self.build.reply(world, build)),
            // nothing else reaches a network from a unit test
            _ => Some(Vec::new()),
        }
    }
}
