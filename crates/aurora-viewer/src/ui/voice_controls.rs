//! Voice controls (the toolbar's Voice command): who is in the voice channel
//! we are in, and who speaks. Port of FSFloaterVoiceControls
//! (indra/newview/fsfloatervoicecontrols.cpp, floater_fs_voice_controls.xml,
//! originally LGPL 2.1) with FSParticipantList (fsparticipantlist.cpp) and
//! menu_participant_list.xml.
//!
//! The title follows the current voice channel (updateTitle: nearby, group
//! call, conference, call with a resident). Our own row is on top with its
//! speaking indicator and the mute a moderator put on us; the participants
//! are below, each joined, invited or left. Someone who left is removed after
//! `VoiceParticipantLeftRemoveDelay` (10 s). The order is the one of
//! `SpeakerParticipantDefaultOrder` (alphabetical by default, or the most
//! recent speakers first), chosen in the right-click menu of a participant
//! like the Legacy Viewer's, with the avatar menu of the resident under it.
//! "Leave call" is not shown on the nearby channel. With RLVa's @shownearby
//! the list is replaced by the restriction's text (updateListVisibility).
//!
//! The per-participant volume slider and mute button the Legacy code looks
//! up are not in its default skin: the volume stays in the avatar menus.

use super::context::{self, AvatarList};
use super::menu;
use super::widgets::{self, ActionKind, Floater};
use crate::theme::Palette;
use crate::world::World;
use crate::world::calls::{CallKind, CallState};
use egui::{RichText, Sense, Vec2};
use std::collections::HashMap;
use std::time::{Duration, Instant};
use uuid::Uuid;

/// The window's id (Floater.Toggle "fs_voice_controls").
pub const WINDOW: &str = "voice_controls";

/// VoiceParticipantLeftRemoveDelay (settings.xml): how long someone who left
/// stays in the list.
pub const LEFT_REMOVE_DELAY: Duration = Duration::from_secs(10);

const ROW_H: f32 = 28.0;
const PIC: f32 = 22.0;
const INDICATOR_W: f32 = 22.0;

/// SpeakerParticipantDefaultOrder (0 = alphabetical, 1 = recent speakers).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum SortOrder {
    #[default]
    Name,
    Recent,
}

impl SortOrder {
    fn from_setting(value: u8) -> SortOrder {
        if value == 1 { SortOrder::Recent } else { SortOrder::Name }
    }

    fn setting(self) -> u8 {
        match self {
            SortOrder::Name => 0,
            SortOrder::Recent => 1,
        }
    }

    fn id() -> egui::Id {
        egui::Id::unique(("voice_controls", "sort"))
    }

    fn load(ctx: &egui::Context) -> SortOrder {
        SortOrder::from_setting(ctx.data_mut(|d| d.get_persisted::<u8>(Self::id())).unwrap_or(0))
    }

    fn save(self, ctx: &egui::Context) {
        ctx.data_mut(|d| d.insert_persisted(Self::id(), self.setting()));
    }
}

/// The channel the list shows (FSFloaterVoiceControls::mVoiceType).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Channel {
    /// The spatial channel (VC_LOCAL_CHAT).
    Nearby,
    /// A call with a resident, by their name (VC_PEER_TO_PEER).
    P2p(String),
    /// A group's call, by the group's name (VC_GROUP_CHAT).
    Group(String),
    /// A conference (VC_AD_HOC_CHAT).
    AdHoc,
}

impl Channel {
    /// updateTitle: title_nearby, title_peer_2_peer, title_group, title_adhoc.
    pub fn title(&self) -> String {
        match self {
            Channel::Nearby => aurora_i18n::tr!("voice-controls-title-nearby").to_owned(),
            Channel::P2p(name) => aurora_i18n::tr!("voice-controls-title-p2p", name = name.as_str()),
            Channel::Group(group) => aurora_i18n::tr!("voice-controls-title-group", group = group.as_str()),
            Channel::AdHoc => aurora_i18n::tr!("voice-controls-title-adhoc").to_owned(),
        }
    }

    /// "Leave Call" is hidden for nearby chat (leave_call_btn_panel).
    pub fn can_leave(&self) -> bool {
        *self != Channel::Nearby
    }

    /// updateListVisibility: RLVa's @shownearby replaces the nearby list
    /// with its text; calls are not concerned.
    pub fn list_hidden(&self, shownearby: bool) -> bool {
        shownearby && *self == Channel::Nearby
    }
}

/// What the voice session says of one participant this frame.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Level {
    /// 0..1.
    pub level: f32,
    pub speaking: bool,
    /// Muted by a moderator.
    pub muted: bool,
}

/// ESpeakerState: where a participant stands in the call.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    /// Asked to join, not in the channel yet (STATE_INVITED).
    Invited,
    Joined,
    /// Was in the channel, no longer (STATE_LEFT): removed after a delay.
    Left,
}

#[derive(Clone, Debug)]
pub struct Entry {
    pub id: Uuid,
    pub state: State,
    pub level: Level,
    left_at: Option<Instant>,
    /// LLSpeaker::mLastSpokeTime.
    last_spoke: Option<Instant>,
}

/// Which channel a roster belongs to: the states of one are forgotten when
/// another becomes the current one (reset).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Nearby,
    Call(Uuid),
}

/// The participants of the channel with their states
/// (FSFloaterVoiceControls::updateParticipantsVoiceState and the speaker
/// manager's last spoke times).
#[derive(Default)]
pub struct Roster {
    key: Option<Key>,
    entries: Vec<Entry>,
    /// Who left this channel and was removed: an invitation or a session
    /// that still lists them does not bring them back as invited.
    gone: std::collections::HashSet<Uuid>,
}

impl Roster {
    /// One frame: `in_voice` are the participants the voice session reports
    /// (us excluded), `invited` the residents asked who may not have joined.
    pub fn update(&mut self, now: Instant, key: Key, in_voice: &[(Uuid, Level)], invited: &[Uuid], delay: Duration) {
        if self.key != Some(key) {
            self.key = Some(key);
            self.entries.clear();
            self.gone.clear();
        }
        for (id, level) in in_voice {
            let at = match self.entries.iter().position(|e| e.id == *id) {
                Some(at) => at,
                None => {
                    self.entries.push(Entry {
                        id: *id,
                        state: State::Joined,
                        level: *level,
                        left_at: None,
                        last_spoke: None,
                    });
                    self.entries.len() - 1
                }
            };
            let e = &mut self.entries[at];
            e.state = State::Joined;
            e.left_at = None;
            self.gone.remove(id);
            e.level = *level;
            if level.speaking {
                e.last_spoke = Some(now);
            }
        }
        for e in &mut self.entries {
            if in_voice.iter().any(|(id, _)| *id == e.id) {
                continue;
            }
            // updateNotInVoiceParticipantState: who was in voice has left
            if e.state == State::Joined {
                e.state = State::Left;
                e.left_at = Some(now);
            }
            e.level = Level::default();
        }
        // an invited resident who is not invited any more, and not in the
        // channel, is no participant
        self.entries.retain(|e| e.state != State::Invited || invited.contains(&e.id));
        for id in invited {
            if !self.gone.contains(id) && !self.entries.iter().any(|e| e.id == *id) {
                self.entries.push(Entry {
                    id: *id,
                    state: State::Invited,
                    level: Level::default(),
                    left_at: None,
                    last_spoke: None,
                });
            }
        }
        // removeVoiceLeftParticipant, `delay` after they left
        let gone = &mut self.gone;
        self.entries.retain(|e| {
            let expired = e.state == State::Left && e.left_at.is_some_and(|at| now.saturating_duration_since(at) >= delay);
            if expired {
                gone.insert(e.id);
            }
            !expired
        });
    }

    /// The rows in the order chosen. FSParticipantList::sort: by name, or by
    /// the last time they spoke (LLAvatarItemRecentSpeakerComparator), those
    /// who never spoke after the others and by name.
    pub fn sorted(&self, order: SortOrder, name: impl Fn(&Uuid) -> String) -> Vec<&Entry> {
        let mut rows: Vec<(&Entry, String)> = self.entries.iter().map(|e| (e, name(&e.id).to_lowercase())).collect();
        rows.sort_by(|(a, an), (b, bn)| {
            let by_name = || an.cmp(bn).then(a.id.cmp(&b.id));
            match order {
                SortOrder::Name => by_name(),
                SortOrder::Recent => match (a.last_spoke, b.last_spoke) {
                    (Some(x), Some(y)) => y.cmp(&x).then_with(by_name),
                    (Some(_), None) => std::cmp::Ordering::Less,
                    (None, Some(_)) => std::cmp::Ordering::Greater,
                    (None, None) => by_name(),
                },
            }
        });
        rows.into_iter().map(|(e, _)| e).collect()
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.entries.len()
    }
}

/// The channel as the world has it this frame.
struct Gathered {
    channel: Channel,
    key: Key,
    /// A call that is not connected yet has no list.
    connected: bool,
    /// Calling a resident who has not answered.
    calling: bool,
    in_voice: Vec<(Uuid, Level)>,
    invited: Vec<Uuid>,
    own: Level,
}

/// Who is in the session of a group or conference and not (yet) in its voice:
/// the invitations pending and the session's other participants, which
/// FSParticipantList lists from the speaker manager of the IM session (the
/// ones not in voice show as invited). The roster leaves out those it knows
/// left voice.
fn waiting(world: &World, conversation: Uuid, me: Uuid) -> Vec<Uuid> {
    let mut ids: Vec<Uuid> = world.calls.invited.get(&conversation).cloned().unwrap_or_default();
    if let Some(info) = world.groups.sessions.get(&conversation) {
        ids.extend(info.participants.iter().copied());
    }
    ids.retain(|id| *id != me);
    ids.sort();
    ids.dedup();
    ids
}

/// Our indicator: the local level and speaking state (`tag_levels` supplies
/// them without a server entry), with the moderator mute of the member.
fn own_level(local: Option<&(f32, bool)>, member: Option<Level>) -> Level {
    let member = member.unwrap_or_default();
    match local {
        Some(&(level, speaking)) => Level { level, speaking, ..member },
        None => member,
    }
}

/// The current voice channel (LLVoiceChannel::getCurrentVoiceChannel): the
/// call in progress, else the nearby channel whose participants are `levels`
/// (id → (level, speaking), as the name tags have them).
fn gather(world: &World, levels: &HashMap<Uuid, (f32, bool)>) -> Gathered {
    let me = world.agent_id;
    if let Some(call) = world.calls.active.as_ref() {
        let channel = match call.kind {
            CallKind::P2p { other } => Channel::P2p(world.social.name_of(&other)),
            CallKind::Group => Channel::Group(call.name.clone()),
            CallKind::AdHoc => Channel::AdHoc,
        };
        let level = |m: &crate::world::calls::CallMember| Level {
            level: m.level,
            speaking: m.speaking,
            muted: m.muted,
        };
        let invited = match call.kind {
            CallKind::P2p { other } => vec![other],
            _ => waiting(world, call.conversation(), me),
        };
        return Gathered {
            channel,
            key: Key::Call(call.session),
            connected: call.state == CallState::Connected,
            calling: call.p2p() && !call.incoming && call.state == CallState::Ringing,
            in_voice: call.members.iter().filter(|m| m.id != me).map(|m| (m.id, level(m))).collect(),
            invited,
            // the local microphone is authoritative even when the server
            // sends no entry of ours; the moderator mute is the server's
            own: own_level(levels.get(&me), call.members.iter().find(|m| m.id == me).map(level)),
        };
    }
    let mut in_voice: Vec<(Uuid, Level)> = levels
        .iter()
        .filter(|(id, _)| **id != me)
        .map(|(id, &(level, speaking))| {
            // shown to everyone; only the moderation actions are gated
            let muted = world
                .calls
                .nearby_moderated
                .as_ref()
                .and_then(|m| m.get(id))
                .copied()
                .unwrap_or(false);
            (*id, Level { level, speaking, muted })
        })
        .collect();
    in_voice.sort_by_key(|(id, _)| *id);
    let own = levels.get(&me).map_or(Level::default(), |&(level, speaking)| Level {
        level,
        speaking,
        muted: world.calls.own_moderated.unwrap_or(false),
    });
    Gathered {
        channel: Channel::Nearby,
        key: Key::Nearby,
        connected: true,
        calling: false,
        in_voice,
        invited: Vec::new(),
        own,
    }
}

/// The id of a participant's row.
fn row_id(resident: Uuid) -> egui::Id {
    egui::Id::unique(("voice_controls_row", resident))
}

/// What the app gives the window each frame.
pub struct Inputs<'a> {
    pub world: &'a World,
    /// Who is in the nearby voice channel: (level, speaking).
    pub levels: &'a HashMap<Uuid, (f32, bool)>,
    pub pics: &'a HashMap<Uuid, egui::TextureHandle>,
}

/// What the window asks of the app.
pub enum Action {
    /// "Leave call" (gIMMgr->endCall).
    LeaveCall,
}

#[derive(Default)]
pub struct VoiceControlsUi {
    roster: Roster,
    selected: Option<Uuid>,
    /// Profile pictures wanted.
    pub wanted_pics: std::collections::HashSet<Uuid>,
}

/// The icon and color of a participant's voice indicator
/// (LLOutputMonitorCtrl: level bars, the muted speaker for a moderator mute).
fn indicator(p: &Palette, state: State, level: Level) -> (&'static str, egui::Color32) {
    if level.muted {
        return ("microphone-off", p.danger);
    }
    match state {
        State::Joined if level.speaking => (
            "microphone-filled",
            p.success.gamma_multiply(0.55 + 0.45 * level.level.clamp(0.0, 1.0)),
        ),
        State::Joined => ("microphone-filled", p.muted_dim),
        State::Invited | State::Left => ("microphone-off", p.muted_dim),
    }
}

impl VoiceControlsUi {
    /// Draw the window while `open`.
    pub fn show(&mut self, ctx: &egui::Context, p: &Palette, f: &Inputs, open: &mut bool) -> Option<Action> {
        if !*open {
            self.roster = Roster::default();
            self.selected = None;
            return None;
        }
        // the speaking indicators and the departures follow the time
        ctx.request_repaint_after(Duration::from_millis(100));
        let g = gather(f.world, f.levels);
        let hidden = g.channel.list_hidden(f.world.rlva.has("shownearby"));
        if g.connected {
            self.roster
                .update(Instant::now(), g.key, &g.in_voice, &g.invited, LEFT_REMOVE_DELAY);
        }
        let mut sort = SortOrder::load(ctx);
        let mut action = None;
        let mut floater = Floater::new(WINDOW, g.channel.title(), egui::pos2(380.0, 140.0), Vec2::new(300.0, 260.0));
        floater.min_size = Vec2::new(230.0, 140.0);
        floater.show(ctx, p, open, |ui| {
            self.own_panel(ui, p, f, &g, &mut action);
            widgets::divider(ui, p);
            if hidden {
                ui.label(
                    RichText::new(aurora_i18n::tr!("voice-controls-blocked-nearby"))
                        .size(12.5)
                        .color(p.muted),
                );
                return;
            }
            if !g.connected {
                let text = if g.calling {
                    aurora_i18n::tr!("ui-call-calling")
                } else {
                    aurora_i18n::tr!("ui-call-connecting")
                };
                ui.label(RichText::new(text).size(12.5).color(p.muted));
                return;
            }
            let rows: Vec<Entry> = self
                .roster
                .sorted(sort, |id| f.world.social.name_of(id))
                .into_iter()
                .cloned()
                .collect();
            if rows.is_empty() {
                if g.channel == Channel::Nearby {
                    ui.label(
                        RichText::new(aurora_i18n::tr!("voice-controls-no-one-near"))
                            .size(12.5)
                            .color(p.muted),
                    );
                }
                return;
            }
            let several = rows.len() > 1;
            let ids: Vec<Uuid> = rows.iter().map(|e| e.id).collect();
            if self.selected.is_some_and(|s| !ids.contains(&s)) {
                self.selected = None;
            }
            let mut clicked = None;
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 1.0;
                for e in &rows {
                    let resp = self.row(ui, p, f, e);
                    if resp.clicked() || resp.secondary_clicked() {
                        clicked = Some(e.id);
                    }
                    menu::context_menu(&resp, p, |ui| {
                        // menu_participant_list.xml: the sort, when there is
                        // something to sort
                        if several {
                            if menu::check(
                                ui,
                                p,
                                "sort-ascending-letters",
                                aurora_i18n::tr!("voice-controls-sort-name"),
                                sort == SortOrder::Name,
                                true,
                            ) {
                                sort = SortOrder::Name;
                                ui.close();
                            }
                            if menu::check(
                                ui,
                                p,
                                "clock",
                                aurora_i18n::tr!("voice-controls-sort-recent"),
                                sort == SortOrder::Recent,
                                true,
                            ) {
                                sort = SortOrder::Recent;
                                ui.close();
                            }
                            menu::separator(ui, p);
                        }
                        context::avatar_list_menu(ui, p, f.world, e.id, AvatarList::Nearby);
                    });
                }
            });
            if clicked.is_some() {
                self.selected = clicked;
            }
        });
        if sort != SortOrder::load(ctx) {
            sort.save(ctx);
        }
        action
    }

    /// my_panel: our picture, name and speaking indicator, and "Leave call".
    fn own_panel(&mut self, ui: &mut egui::Ui, p: &Palette, f: &Inputs, g: &Gathered, action: &mut Option<Action>) {
        let me = f.world.agent_id;
        let name = f.world.social.display_name_of(&me);
        self.wanted_pics.insert(me);
        f.world.social.avatar_names.want(&me);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            let (rect, _) = ui.allocate_exact_size(Vec2::splat(PIC), Sense::hover());
            widgets::avatar_disc(ui, p, rect, f.pics.get(&me), &name, false);
            let leave = g.channel.can_leave();
            let room = (ui.available_width() - INDICATOR_W - if leave { 120.0 } else { 0.0 }).max(40.0);
            ui.scope(|ui| {
                ui.set_max_width(room);
                ui.add(egui::Label::new(RichText::new(&name).size(12.5).strong().color(p.ink)).truncate());
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if leave
                    && widgets::action_button(
                        ui,
                        p,
                        "phone-off",
                        aurora_i18n::tr!("voice-controls-leave"),
                        ActionKind::Danger,
                        Vec2::new(0.0, 24.0),
                    )
                    .on_hover_text(aurora_i18n::tr!("voice-controls-leave-tip"))
                    .clicked()
                {
                    *action = Some(Action::LeaveCall);
                }
                let (rect, resp) = ui.allocate_exact_size(Vec2::splat(INDICATOR_W), Sense::hover());
                let (icon, color) = indicator(p, State::Joined, g.own);
                widgets::paint_glyph(ui.painter(), icon, rect.center(), 16.0, color);
                if g.own.muted {
                    resp.on_hover_text(aurora_i18n::tr!("voice-controls-muted-by-moderator"));
                }
            });
        });
    }

    /// One participant: picture (ringed green while speaking), name, state
    /// and voice indicator.
    fn row(&mut self, ui: &mut egui::Ui, p: &Palette, f: &Inputs, e: &Entry) -> egui::Response {
        let id = e.id;
        self.wanted_pics.insert(id);
        f.world.social.avatar_names.want(&id);
        let name = f.world.social.name_of(&id);
        let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), ROW_H), Sense::hover());
        // an id of the resident, not of the row's place: the rows reorder, and
        // the menu opened on one stays theirs
        let resp = ui.interact(rect, row_id(id), Sense::click());
        if !ui.is_rect_visible(rect) {
            return resp;
        }
        let radius = ui.visuals().window_corner_radius;
        if self.selected == Some(id) {
            ui.painter().rect_filled(rect, radius, p.violet.gamma_multiply(0.30));
        } else if resp.hovered() {
            ui.painter().rect_filled(rect, radius, p.raised.gamma_multiply(0.6));
        }
        let dim = e.state != State::Joined;
        let disc = egui::Rect::from_center_size(egui::pos2(rect.left() + 6.0 + PIC * 0.5, rect.center().y), Vec2::splat(PIC));
        widgets::avatar_disc(ui, p, disc, f.pics.get(&id), &name, dim);
        if e.state == State::Joined && e.level.speaking {
            ui.painter()
                .circle_stroke(disc.center(), PIC * 0.5 + 1.5, egui::Stroke::new(2.0, p.success));
        }
        // the voice indicator at the right, the state before it
        let (icon, color) = indicator(p, e.state, e.level);
        let at = egui::pos2(rect.right() - 6.0 - INDICATOR_W * 0.5, rect.center().y);
        widgets::paint_glyph(ui.painter(), icon, at, 16.0, color);
        let mut right = at.x - INDICATOR_W * 0.5;
        let font = egui::FontId::proportional(11.5);
        let state_text = match e.state {
            State::Joined => None,
            State::Invited => Some(aurora_i18n::tr!("voice-controls-invited")),
            State::Left => Some(aurora_i18n::tr!("voice-controls-left")),
        };
        if let Some(text) = state_text {
            let galley = ui.painter().layout_no_wrap(text.to_owned(), font.clone(), p.muted_dim);
            right -= galley.size().x;
            let top = egui::pos2(right, rect.center().y - galley.size().y * 0.5);
            ui.painter().galley(top, galley, p.muted_dim);
            right -= 6.0;
        }
        let left = disc.right() + 8.0;
        let text_color = if dim { p.muted } else { p.ink };
        widgets::paint_elided(
            ui,
            &name,
            egui::pos2(left, rect.center().y),
            (right - left).max(0.0),
            egui::FontId::proportional(12.5),
            text_color,
        );
        if e.level.muted {
            return resp.on_hover_text(aurora_i18n::tr!("voice-controls-muted-by-moderator"));
        }
        resp
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    fn voice(speaking: bool) -> Level {
        Level {
            level: if speaking { 0.5 } else { 0.0 },
            speaking,
            muted: false,
        }
    }

    fn names(ids: &[&Entry]) -> Vec<Uuid> {
        ids.iter().map(|e| e.id).collect()
    }

    fn name_of(n: &Uuid) -> String {
        match n.as_u128() {
            1 => "Zed".into(),
            2 => "amy".into(),
            3 => "Bob".into(),
            _ => "?".into(),
        }
    }

    #[test]
    fn the_title_follows_the_channel() {
        // the catalogue's text, whatever the language of the PC
        assert_eq!(Channel::Nearby.title(), aurora_i18n::tr!("voice-controls-title-nearby"));
        assert_eq!(Channel::AdHoc.title(), aurora_i18n::tr!("voice-controls-title-adhoc"));
        assert_eq!(
            Channel::P2p("Nova".into()).title(),
            aurora_i18n::tr!("voice-controls-title-p2p", name = "Nova")
        );
        assert!(Channel::P2p("Nova".into()).title().contains("Nova"));
        assert_eq!(
            Channel::Group("Dancers".into()).title(),
            aurora_i18n::tr!("voice-controls-title-group", group = "Dancers")
        );
        assert!(Channel::Group("Dancers".into()).title().contains("Dancers"));
        assert_ne!(Channel::Nearby.title(), Channel::AdHoc.title());
    }

    /// The window keeps its size at every scale and text size, with the
    /// nearby participants listed and the last row inside it.
    #[test]
    fn the_window_keeps_its_size() {
        let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        world.agent_id = id(9);
        world.social.names.insert(id(9), "Me Resident".into());
        let mut levels = HashMap::new();
        levels.insert(id(9), (0.0, false));
        for n in 1..=3 {
            world.social.names.insert(id(n), name_of(&id(n)));
            levels.insert(id(n), (0.0, false));
        }
        let pics = HashMap::new();
        // sorted by name: amy, Bob, Zed
        crate::ui::widgets::steady_tests::assert_window_steady(
            WINDOW,
            &[Vec2::new(300.0, 260.0), Vec2::new(420.0, 360.0)],
            "Zed",
            VoiceControlsUi::default,
            |ui, ctx, p| {
                let inputs = Inputs {
                    world: &world,
                    levels: &levels,
                    pics: &pics,
                };
                ui.show(ctx, p, &inputs, &mut true);
            },
        );
    }

    #[test]
    fn a_row_keeps_its_id_when_the_rows_reorder() {
        assert_eq!(row_id(id(1)), row_id(id(1)));
        assert_ne!(row_id(id(1)), row_id(id(2)));
    }

    #[test]
    fn our_indicator_follows_the_microphone_and_keeps_the_moderator_mute() {
        let member = Level {
            level: 0.0,
            speaking: false,
            muted: true,
        };
        let own = own_level(Some(&(0.6, true)), Some(member));
        assert!(own.speaking && own.muted && (own.level - 0.6).abs() < 1e-6);
        let own = own_level(Some(&(0.6, true)), None);
        assert!(own.speaking && !own.muted);
        assert_eq!(own_level(None, Some(member)), member);
    }

    #[test]
    fn leave_call_is_for_calls_only() {
        assert!(!Channel::Nearby.can_leave());
        assert!(Channel::AdHoc.can_leave() && Channel::P2p("N".into()).can_leave() && Channel::Group("G".into()).can_leave());
    }

    #[test]
    fn rlva_hides_the_nearby_list_only() {
        assert!(Channel::Nearby.list_hidden(true));
        assert!(!Channel::Nearby.list_hidden(false));
        assert!(!Channel::AdHoc.list_hidden(true));
        assert!(!Channel::Group("G".into()).list_hidden(true));
    }

    #[test]
    fn the_sort_order_is_the_legacy_setting() {
        assert_eq!(SortOrder::from_setting(0), SortOrder::Name);
        assert_eq!(SortOrder::from_setting(1), SortOrder::Recent);
        assert_eq!(SortOrder::default(), SortOrder::Name);
        assert_eq!(SortOrder::Recent.setting(), 1);
    }

    #[test]
    fn who_leaves_is_removed_after_the_delay() {
        let t0 = Instant::now();
        let at = |s| t0 + Duration::from_secs(s);
        let mut r = Roster::default();
        r.update(
            t0,
            Key::Nearby,
            &[(id(1), voice(false)), (id(2), voice(true))],
            &[],
            LEFT_REMOVE_DELAY,
        );
        assert_eq!(r.len(), 2);
        // 2 leaves: still listed, as left
        r.update(at(1), Key::Nearby, &[(id(1), voice(false))], &[], LEFT_REMOVE_DELAY);
        let rows = r.sorted(SortOrder::Name, name_of);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows.iter().find(|e| e.id == id(2)).map(|e| e.state), Some(State::Left));
        r.update(at(10), Key::Nearby, &[(id(1), voice(false))], &[], LEFT_REMOVE_DELAY);
        assert_eq!(r.len(), 2, "9 s after they left");
        r.update(at(11), Key::Nearby, &[(id(1), voice(false))], &[], LEFT_REMOVE_DELAY);
        assert_eq!(r.len(), 1, "10 s after they left");
    }

    #[test]
    fn someone_who_comes_back_is_joined_again() {
        let t0 = Instant::now();
        let mut r = Roster::default();
        r.update(t0, Key::Nearby, &[(id(1), voice(false))], &[], LEFT_REMOVE_DELAY);
        r.update(t0 + Duration::from_secs(1), Key::Nearby, &[], &[], LEFT_REMOVE_DELAY);
        r.update(
            t0 + Duration::from_secs(3),
            Key::Nearby,
            &[(id(1), voice(false))],
            &[],
            LEFT_REMOVE_DELAY,
        );
        let rows = r.sorted(SortOrder::Name, name_of);
        assert_eq!(rows[0].state, State::Joined);
        r.update(
            t0 + Duration::from_secs(30),
            Key::Nearby,
            &[(id(1), voice(false))],
            &[],
            LEFT_REMOVE_DELAY,
        );
        assert_eq!(r.len(), 1, "no timer is left on someone who came back");
    }

    #[test]
    fn the_invited_wait_until_they_join_or_are_not_invited_any_more() {
        let t0 = Instant::now();
        let call = Key::Call(id(9));
        let mut r = Roster::default();
        r.update(t0, call, &[], &[id(3)], LEFT_REMOVE_DELAY);
        assert_eq!(r.sorted(SortOrder::Name, name_of)[0].state, State::Invited);
        // an invitation does not expire by itself
        r.update(t0 + Duration::from_secs(600), call, &[], &[id(3)], LEFT_REMOVE_DELAY);
        assert_eq!(r.len(), 1);
        r.update(
            t0 + Duration::from_secs(601),
            call,
            &[(id(3), voice(false))],
            &[id(3)],
            LEFT_REMOVE_DELAY,
        );
        assert_eq!(r.sorted(SortOrder::Name, name_of)[0].state, State::Joined);
        // withdrawn
        let mut r = Roster::default();
        r.update(t0, call, &[], &[id(3)], LEFT_REMOVE_DELAY);
        r.update(t0, call, &[], &[], LEFT_REMOVE_DELAY);
        assert_eq!(r.len(), 0);
        // an invitation that outlives their visit does not bring them back
        let mut r = Roster::default();
        r.update(t0, call, &[(id(3), voice(false))], &[id(3)], LEFT_REMOVE_DELAY);
        r.update(t0 + Duration::from_secs(1), call, &[], &[id(3)], LEFT_REMOVE_DELAY);
        r.update(t0 + Duration::from_secs(12), call, &[], &[id(3)], LEFT_REMOVE_DELAY);
        assert_eq!(r.len(), 0, "removed");
        r.update(t0 + Duration::from_secs(13), call, &[], &[id(3)], LEFT_REMOVE_DELAY);
        assert_eq!(r.len(), 0, "and not invited again");
        r.update(
            t0 + Duration::from_secs(14),
            call,
            &[(id(3), voice(false))],
            &[id(3)],
            LEFT_REMOVE_DELAY,
        );
        assert_eq!(r.len(), 1, "unless they join again");
        // someone who left is not invited back into the list by the invitation
        let mut r = Roster::default();
        r.update(t0, call, &[(id(3), voice(false))], &[id(3)], LEFT_REMOVE_DELAY);
        r.update(t0, call, &[], &[id(3)], LEFT_REMOVE_DELAY);
        assert_eq!(r.sorted(SortOrder::Name, name_of)[0].state, State::Left);
    }

    #[test]
    fn another_channel_starts_a_fresh_list() {
        let t0 = Instant::now();
        let mut r = Roster::default();
        r.update(t0, Key::Nearby, &[(id(1), voice(false))], &[], LEFT_REMOVE_DELAY);
        r.update(t0, Key::Call(id(9)), &[(id(2), voice(false))], &[], LEFT_REMOVE_DELAY);
        assert_eq!(names(&r.sorted(SortOrder::Name, name_of)), [id(2)]);
    }

    #[test]
    fn names_sort_regardless_of_case() {
        let mut r = Roster::default();
        r.update(
            Instant::now(),
            Key::Nearby,
            &[(id(1), voice(false)), (id(2), voice(false)), (id(3), voice(false))],
            &[],
            LEFT_REMOVE_DELAY,
        );
        // amy, Bob, Zed
        assert_eq!(names(&r.sorted(SortOrder::Name, name_of)), [id(2), id(3), id(1)]);
    }

    #[test]
    fn the_most_recent_speaker_comes_first() {
        let t0 = Instant::now();
        let at = |s| t0 + Duration::from_secs(s);
        let mut r = Roster::default();
        let all = |a: bool, b: bool| [(id(1), voice(a)), (id(2), voice(b)), (id(3), voice(false))];
        r.update(at(0), Key::Nearby, &all(true, false), &[], LEFT_REMOVE_DELAY);
        r.update(at(1), Key::Nearby, &all(false, true), &[], LEFT_REMOVE_DELAY);
        r.update(at(2), Key::Nearby, &all(false, false), &[], LEFT_REMOVE_DELAY);
        // amy spoke last, then Zed; Bob never spoke
        assert_eq!(names(&r.sorted(SortOrder::Recent, name_of)), [id(2), id(1), id(3)]);
        // and by name when nobody spoke
        let mut quiet = Roster::default();
        quiet.update(at(0), Key::Nearby, &all(false, false), &[], LEFT_REMOVE_DELAY);
        assert_eq!(names(&quiet.sorted(SortOrder::Recent, name_of)), [id(2), id(3), id(1)]);
    }
}
