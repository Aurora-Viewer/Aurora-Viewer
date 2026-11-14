//! The Avatar Studio animation timeline window. Top to bottom: a pinned
//! toolbar (transport, frame readout, posing tools, keying, edit, view and
//! file), a frame ruler lined up with the bone lanes, the lanes in their own
//! scroll area under one continuous playhead, and a status line.
use super::*;
use crate::ui::menu;

const WINDOW_ID: &str = "avatar-studio-animation-timeline-window-v3";
/// Height of the toolbar's buttons.
const TOOL_HEIGHT: f32 = 24.0;
/// An icon-only toolbar button.
const ICON_BUTTON: egui::Vec2 = egui::vec2(26.0, TOOL_HEIGHT);
/// Room between two groups of the toolbar: spacing alone tells them apart.
const GROUP_GAP: f32 = 10.0;
const RULER_HEIGHT: f32 = 24.0;
const ROW_HEIGHT: f32 = 22.0;
const ROW_GAP: f32 = 2.0;
/// Room kept at both ends of the ruler and of every lane, so that a key on
/// the first or the last visible frame shows whole.
const LANE_PAD: f32 = 7.0;
/// How near the pointer, in points, a key can be grabbed.
const KEY_GRAB: f32 = 8.0;
/// Half the diagonal of a key's diamond.
const KEY_HALF: f32 = 4.5;
/// Labelled ruler ticks stay at least this many points apart.
const LABEL_SPACING: f32 = 60.0;
/// Frames between labelled ruler ticks, from the closest zoom out.
const TICK_STEPS: [u32; 8] = [1, 5, 10, 30, 60, 150, 300, 600];
const RULER_FONT: f32 = 11.0;
pub(super) const MAX_TIMELINE_ZOOM: f32 = 64.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum TimelineShortcut {
    PlayPause,
    Rewind,
    PreviousFrame,
    NextFrame,
    KeyBone,
    Undo,
    Redo,
    CopyPose,
    PastePose,
    DeleteKey,
    SaveProject,
    SelectAllKeys,
    ClearKeySelection,
}

pub(super) fn timeline_key(key: &winit::keyboard::Key) -> Option<egui::Key> {
    use winit::keyboard::{Key, NamedKey};
    match key {
        Key::Named(NamedKey::Space) => Some(egui::Key::Space),
        Key::Named(NamedKey::Home) => Some(egui::Key::Home),
        Key::Named(NamedKey::Delete) => Some(egui::Key::Delete),
        Key::Named(NamedKey::Escape) => Some(egui::Key::Escape),
        Key::Named(NamedKey::ArrowLeft) => Some(egui::Key::ArrowLeft),
        Key::Named(NamedKey::ArrowRight) => Some(egui::Key::ArrowRight),
        Key::Character(character) => match character.to_lowercase().as_str() {
            "z" => Some(egui::Key::Z),
            "y" => Some(egui::Key::Y),
            "c" => Some(egui::Key::C),
            "v" => Some(egui::Key::V),
            "s" => Some(egui::Key::S),
            "k" => Some(egui::Key::K),
            "a" => Some(egui::Key::A),
            _ => None,
        },
        _ => None,
    }
}

pub(super) fn timeline_default_position(content: egui::Rect, size: egui::Vec2) -> egui::Pos2 {
    egui::pos2(
        (content.right() - size.x - 16.0).max(content.left()),
        (content.bottom() - size.y - 16.0).max(content.top()),
    )
}

pub(super) fn timeline_default_size(content: egui::Rect) -> egui::Vec2 {
    let width = (content.width() - 700.0).clamp(700.0, 960.0).min(content.width());
    egui::vec2(width, content.height().min(440.0))
}

pub(super) fn timeline_shortcut(key: egui::Key, modifiers: egui::Modifiers) -> Option<TimelineShortcut> {
    let command = modifiers.ctrl || modifiers.command;
    if modifiers.matches_exact(egui::Modifiers::NONE) {
        let plain = match key {
            egui::Key::Space => Some(TimelineShortcut::PlayPause),
            egui::Key::Delete => Some(TimelineShortcut::DeleteKey),
            egui::Key::Home => Some(TimelineShortcut::Rewind),
            egui::Key::ArrowLeft => Some(TimelineShortcut::PreviousFrame),
            egui::Key::ArrowRight => Some(TimelineShortcut::NextFrame),
            egui::Key::K => Some(TimelineShortcut::KeyBone),
            egui::Key::Escape => Some(TimelineShortcut::ClearKeySelection),
            _ => None,
        };
        if plain.is_some() {
            return plain;
        }
    }
    if command && !modifiers.alt {
        return match (key, modifiers.shift) {
            (egui::Key::Z, false) => Some(TimelineShortcut::Undo),
            (egui::Key::Z, true) | (egui::Key::Y, false) => Some(TimelineShortcut::Redo),
            (egui::Key::C, false) => Some(TimelineShortcut::CopyPose),
            (egui::Key::V, false) => Some(TimelineShortcut::PastePose),
            (egui::Key::S, false) => Some(TimelineShortcut::SaveProject),
            (egui::Key::A, false) => Some(TimelineShortcut::SelectAllKeys),
            _ => None,
        };
    }
    None
}

pub(super) fn timeline_view(start: u32, end: u32, zoom: f32, requested_start: u32) -> (u32, u32) {
    let total = end.saturating_sub(start).max(1);
    let visible = ((total as f32 / zoom.clamp(1.0, MAX_TIMELINE_ZOOM)).round() as u32).clamp(1, total);
    let view_start = requested_start.clamp(start, end.saturating_sub(visible));
    (view_start, view_start + visible)
}

pub(super) fn timeline_start_around(start: u32, end: u32, zoom: f32, anchor: u32) -> u32 {
    let (_, view_end) = timeline_view(start, end, zoom, start);
    let visible = view_end - start;
    timeline_view(start, end, zoom, anchor.saturating_sub(visible / 2)).0
}

pub(super) fn reveal_timeline_frame(start: u32, end: u32, zoom: f32, requested_start: u32, frame: u32) -> u32 {
    let (left, right) = timeline_view(start, end, zoom, requested_start);
    if (left..=right).contains(&frame) {
        left
    } else {
        timeline_start_around(start, end, zoom, frame.clamp(start, end))
    }
}

pub(super) fn follow_timeline_playhead(start: u32, end: u32, zoom: f32, requested_start: u32, frame: u32, playing: bool) -> u32 {
    let (left, right) = timeline_view(start, end, zoom, requested_start);
    if !playing || (frame >= left && frame <= right) {
        return left;
    }
    let visible = right - left;
    timeline_view(start, end, zoom, frame.saturating_sub(visible * 9 / 10)).0
}

/// Zoom the view by `factor` (Ctrl+wheel) and keep the frame under the
/// pointer, at `fraction` of the lane's width, under the pointer. Returns the
/// new zoom and the start of the view.
pub(super) fn zoom_timeline_around(start: u32, end: u32, zoom: f32, requested_start: u32, fraction: f32, factor: f32) -> (f32, u32) {
    let (left, right) = timeline_view(start, end, zoom, requested_start);
    if !factor.is_finite() || factor <= 0.0 || !fraction.is_finite() {
        return (zoom, left);
    }
    let fraction = fraction.clamp(0.0, 1.0);
    let anchor = left as f32 + fraction * (right - left) as f32;
    let zoom = (zoom * factor).clamp(1.0, MAX_TIMELINE_ZOOM);
    let (_, visible_end) = timeline_view(start, end, zoom, start);
    let visible = (visible_end - start) as f32;
    let requested = (anchor - fraction * visible).round().max(0.0) as u32;
    (zoom, timeline_view(start, end, zoom, requested).0)
}

/// Move the view by `frames` (negative: towards the start), within the
/// animation (Shift+wheel or a horizontal wheel).
pub(super) fn pan_timeline(start: u32, end: u32, zoom: f32, requested_start: u32, frames: i64) -> u32 {
    let (left, _) = timeline_view(start, end, zoom, requested_start);
    let moved = (i64::from(left) + frames).clamp(0, i64::from(u32::MAX)) as u32;
    timeline_view(start, end, zoom, moved).0
}

/// Frames between two labelled ticks of the ruler at `points_per_frame`:
/// the first step whose labels stay `LABEL_SPACING` apart, and past the
/// longest one, a multiple of it.
pub(super) fn ruler_tick_step(points_per_frame: f32) -> u32 {
    let longest = TICK_STEPS[TICK_STEPS.len() - 1];
    if !points_per_frame.is_finite() || points_per_frame <= 0.0 {
        return longest;
    }
    TICK_STEPS
        .into_iter()
        .find(|step| *step as f32 * points_per_frame >= LABEL_SPACING)
        .unwrap_or_else(|| {
            let multiple = (LABEL_SPACING / (longest as f32 * points_per_frame)).ceil().max(1.0);
            longest.saturating_mul(multiple as u32)
        })
}

/// Frames between the short unlabelled ticks inside one labelled step.
pub(super) fn ruler_minor_step(major: u32) -> Option<u32> {
    match major {
        0 | 1 => None,
        5 | 10 | 150 | 300 => Some(major / 5),
        _ => Some((major / 6).max(1)),
    }
}

/// The key before (or after, `forward`) `frame`: on the selected bone's
/// track when it has one, else on every authored track.
pub(super) fn neighbour_key(tracks: &[TimelineTrack], selected: Option<usize>, frame: u32, forward: bool) -> Option<u32> {
    let pick = |keys: &[u32]| {
        if forward {
            keys.get(keys.partition_point(|key| *key <= frame)).copied()
        } else {
            keys.partition_point(|key| *key < frame).checked_sub(1).map(|index| keys[index])
        }
    };
    if let Some(track) = selected.and_then(|bone| tracks.iter().find(|track| track.bone == bone)) {
        return pick(&track.keys);
    }
    let keys = tracks.iter().filter(|track| track.authored).filter_map(|track| pick(&track.keys));
    if forward { keys.min() } else { keys.max() }
}

/// Where the frames of the visible window fall across the ruler and the
/// lanes, which share their left edge and width.
#[derive(Clone, Copy, Debug)]
pub(super) struct FrameScale {
    left: f32,
    width: f32,
    start: u32,
    end: u32,
}

impl FrameScale {
    /// The visible frames `start..=end` across `lane`, inside its padding.
    pub(super) fn new(lane: egui::Rect, start: u32, end: u32) -> Self {
        Self {
            left: lane.left() + LANE_PAD,
            width: (lane.width() - 2.0 * LANE_PAD).max(1.0),
            start,
            end,
        }
    }

    fn span(&self) -> f64 {
        f64::from(self.end.saturating_sub(self.start).max(1))
    }

    /// The x of a frame, fractional for the playing playhead.
    pub(super) fn x(&self, frame: f64) -> f32 {
        self.left + self.width * ((frame - f64::from(self.start)) / self.span()) as f32
    }

    /// The visible frame nearest to `x`.
    pub(super) fn frame_at(&self, x: f32) -> u32 {
        let frame = f64::from(self.start) + f64::from((x - self.left) / self.width) * self.span();
        (frame.round().max(0.0) as u32).max(self.start).min(self.end)
    }

    pub(super) fn points_per_frame(&self) -> f32 {
        self.width / self.span() as f32
    }

    fn contains(&self, frame: f64) -> bool {
        frame >= f64::from(self.start) && frame <= f64::from(self.end)
    }
}

/// The visible key of `keys` (sorted) nearest to `x`, within `KEY_GRAB`.
pub(super) fn key_under(keys: &[u32], scale: &FrameScale, x: f32) -> Option<u32> {
    let visible = &keys[keys.partition_point(|key| *key < scale.start)..keys.partition_point(|key| *key <= scale.end)];
    let pivot = visible.partition_point(|key| scale.x(f64::from(*key)) < x);
    [pivot.checked_sub(1), Some(pivot)]
        .into_iter()
        .flatten()
        .filter_map(|index| visible.get(index).copied())
        .map(|key| (key, (scale.x(f64::from(key)) - x).abs()))
        .filter(|(_, distance)| *distance <= KEY_GRAB)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(key, _)| key)
}

/// The visible keys of `keys` (sorted) inside `area`, a box drawn over the
/// lanes: the lane must cross the box, and a key's diamond touch it.
pub(super) fn keys_in_box(keys: &[u32], scale: &FrameScale, lane: egui::Rect, area: egui::Rect) -> Vec<u32> {
    if lane.bottom() < area.top() || lane.top() > area.bottom() {
        return Vec::new();
    }
    let visible = &keys[keys.partition_point(|key| *key < scale.start)..keys.partition_point(|key| *key <= scale.end)];
    visible
        .iter()
        .copied()
        .filter(|key| {
            let x = scale.x(f64::from(*key));
            x + KEY_HALF >= area.left() && x - KEY_HALF <= area.right()
        })
        .collect()
}

/// Pick a key with the pointer, as the lists of files do: a plain click
/// picks it alone, Ctrl (Cmd) toggles it, Shift adds it. A press that starts
/// a drag keeps the selection when the key is already in it, so that the
/// drag moves every picked key, and adds it under Ctrl or Shift.
pub(super) fn pick_key(selection: &mut BTreeSet<(usize, u32)>, key: (usize, u32), modifiers: egui::Modifiers, drag: bool) {
    let command = modifiers.ctrl || modifiers.command;
    if command && !drag {
        if !selection.remove(&key) {
            selection.insert(key);
        }
    } else if command || modifiers.shift || (drag && selection.contains(&key)) {
        selection.insert(key);
    } else {
        selection.clear();
        selection.insert(key);
    }
}

/// A box drawn over the lanes with Shift or Ctrl: from where the drag
/// started to the pointer, and the keys inside it this frame.
pub(super) struct KeyBox {
    origin: egui::Pos2,
    current: egui::Pos2,
    hits: Vec<(usize, u32)>,
}

impl KeyBox {
    fn area(&self) -> egui::Rect {
        egui::Rect::from_two_pos(self.origin, self.current)
    }
}

/// The lanes' geometry shared by every row: the name column, then the
/// visible frames.
#[derive(Clone, Copy)]
struct Lanes {
    name_width: f32,
    start: u32,
    end: u32,
}

/// An icon-only button of the toolbar, greyed out when not `enabled`.
fn tool_button(ui: &mut egui::Ui, palette: &Palette, icon: &str, enabled: bool) -> egui::Response {
    widgets::action_button_if(ui, palette, icon, "", widgets::ActionKind::Flat, ICON_BUTTON, enabled)
}

/// An icon-only switch of the toolbar, lit while `on`.
fn tool_toggle(ui: &mut egui::Ui, palette: &Palette, icon: &str, on: bool) -> egui::Response {
    let kind = if on {
        widgets::ActionKind::Primary
    } else {
        widgets::ActionKind::Flat
    };
    widgets::action_button(ui, palette, icon, "", kind, ICON_BUTTON)
}

/// A `widgets::segmented` laid out as one item of the wrapping toolbar,
/// with `tip` over it. Returns true when an enabled choice changed.
fn tool_segmented(
    ui: &mut egui::Ui,
    palette: &Palette,
    id: &str,
    selected: &mut usize,
    options: &[(&str, bool)],
    enabled: bool,
    tip: &str,
) -> bool {
    let (rect, _) = ui.allocate_exact_size(widgets::segmented_size(ui, options), egui::Sense::hover());
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .id_salt(id)
            .max_rect(rect)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    if !enabled {
        child.disable();
    }
    let changed = widgets::segmented(&mut child, palette, selected, options);
    // registered after the segments so that it is hovered with them
    ui.interact(rect, ui.scope_id().with((id, "tip")), egui::Sense::hover())
        .on_hover_text(tip);
    changed && enabled
}

fn paint_key(painter: &egui::Painter, center: egui::Pos2, half: f32, color: egui::Color32) {
    painter.add(egui::Shape::convex_polygon(
        vec![
            center - egui::vec2(0.0, half),
            center + egui::vec2(half, 0.0),
            center + egui::vec2(0.0, half),
            center - egui::vec2(half, 0.0),
        ],
        color,
        egui::Stroke::NONE,
    ));
}

/// The ruler's ticks and frame numbers, and the faint lines of the labelled
/// ticks across the lanes below it, down to `bottom`.
fn paint_ruler(ui: &egui::Ui, palette: &Palette, ruler: egui::Rect, bottom: f32, scale: &FrameScale) {
    let painter = ui.painter_at(ruler);
    let grid = ui.painter_at(egui::Rect::from_min_max(
        egui::pos2(ruler.left(), ruler.bottom()),
        egui::pos2(ruler.right(), bottom),
    ));
    let points_per_frame = scale.points_per_frame();
    let step = ruler_tick_step(points_per_frame);
    let tick = egui::Stroke::new(1.0, palette.muted_dim);
    if let Some(minor) = ruler_minor_step(step).filter(|minor| *minor as f32 * points_per_frame >= 4.0) {
        let mut frame = scale.start.div_ceil(minor).saturating_mul(minor);
        while frame <= scale.end {
            if frame % step != 0 {
                let x = scale.x(f64::from(frame));
                painter.line_segment([egui::pos2(x, ruler.bottom() - 3.0), egui::pos2(x, ruler.bottom())], tick);
            }
            let Some(next) = frame.checked_add(minor) else { break };
            frame = next;
        }
    }
    let font = egui::FontId::proportional(RULER_FONT);
    let mut frame = scale.start.div_ceil(step).saturating_mul(step);
    while frame <= scale.end {
        let x = scale.x(f64::from(frame));
        painter.line_segment([egui::pos2(x, ruler.bottom() - 7.0), egui::pos2(x, ruler.bottom())], tick);
        grid.vline(x, grid.clip_rect().y_range(), widgets::divider_stroke(palette));
        let galley = painter.layout_no_wrap(frame.to_string(), font.clone(), palette.muted);
        let left = (x - galley.size().x / 2.0).min(ruler.right() - galley.size().x).max(ruler.left());
        painter.galley(egui::pos2(left, ruler.top() + 2.0), galley, palette.muted);
        let Some(next) = frame.checked_add(step) else { break };
        frame = next;
    }
    painter.hline(ruler.x_range(), ruler.bottom() - 0.5, widgets::divider_stroke(palette));
}

/// One playhead line across the ruler and the lanes down to `bottom`, at the
/// continuous playhead (smooth while playing), and its frame's tag on the
/// ruler.
fn paint_playhead(ui: &egui::Ui, palette: &Palette, ruler: egui::Rect, bottom: f32, scale: &FrameScale, cursor: f64, frame: u32) {
    if !scale.contains(cursor) {
        return;
    }
    let x = scale.x(cursor);
    let painter = ui.painter();
    painter.line_segment(
        [egui::pos2(x, ruler.top()), egui::pos2(x, bottom.max(ruler.bottom()))],
        egui::Stroke::new(1.5, palette.teal),
    );
    let galley = painter.layout_no_wrap(frame.to_string(), egui::FontId::proportional(RULER_FONT), palette.navy);
    let size = galley.size() + egui::vec2(8.0, 2.0);
    let left = (x - size.x / 2.0).min(ruler.right() - size.x).max(ruler.left());
    let tag = egui::Rect::from_min_size(egui::pos2(left, ruler.top() + 1.0), size);
    painter.rect_filled(tag, ui.visuals().window_corner_radius, palette.teal);
    painter.galley(tag.center() - galley.size() / 2.0, galley, palette.navy);
}

impl Editor {
    pub(crate) fn map_timeline_key(key: &winit::keyboard::Key) -> Option<egui::Key> {
        timeline_key(key)
    }

    /// Route editor accelerators before the viewer's global keybindings see
    /// them. This keeps timeline commands such as Home and Ctrl+S from also
    /// changing flight or resynchronizing avatar animations.
    ///
    /// True only when a command ran, and only while the timeline is shown:
    /// a key that would do nothing here (nothing to undo, no key to delete,
    /// no bone to key, an empty clipboard, a save while one is under way)
    /// keeps its global action, as a disabled menu accelerator does
    /// (LLMenuItemCallGL::handleAcceleratorKey, indra/llui/llmenugl.cpp).
    pub(crate) fn handle_timeline_shortcut(&mut self, ctx: &egui::Context, key: egui::Key, modifiers: egui::Modifiers) -> bool {
        if !self.clip.active
            || !self.open
            || !self.clip.timeline_open
            || self.clip.document.is_none()
            || self.clip.waiting_source()
            || ctx.egui_wants_keyboard_input()
            || egui::Popup::is_any_open(ctx)
            || ctx.memory(|memory| memory.top_modal_layer().is_some() || memory.focused().is_some())
        {
            return false;
        }
        let Some(shortcut) = timeline_shortcut(key, modifiers) else {
            return false;
        };
        let ran = match shortcut {
            // The transport always acts on the playhead, clamped to the
            // range: an arrow held at the last frame stays the timeline's.
            TimelineShortcut::PlayPause => {
                self.studio_toggle_clip_playback();
                true
            }
            TimelineShortcut::Rewind => {
                if let Some(start) = self.clip.document.as_ref().map(|document| document.start) {
                    self.clip_seek(start);
                }
                true
            }
            TimelineShortcut::PreviousFrame => {
                self.step_timeline_frame(false);
                true
            }
            TimelineShortcut::NextFrame => {
                self.step_timeline_frame(true);
                true
            }
            TimelineShortcut::CopyPose => {
                self.studio_copy_clip_keyframe();
                true
            }
            TimelineShortcut::KeyBone => match self.selected {
                Some(bone) => {
                    self.clip.capture = Some(Capture::Bone(bone));
                    true
                }
                None => false,
            },
            TimelineShortcut::Undo if !self.clip.undo.is_empty() => {
                self.studio_clip_undo();
                true
            }
            TimelineShortcut::Redo if !self.clip.redo.is_empty() => {
                self.studio_clip_redo();
                true
            }
            TimelineShortcut::PastePose if self.has_clip_clipboard() => {
                self.studio_paste_clip_keyframe();
                true
            }
            TimelineShortcut::DeleteKey if self.has_deletable_key_selection() => {
                self.studio_delete_selected_keys();
                true
            }
            TimelineShortcut::DeleteKey if self.selected.is_some_and(|bone| self.clip.has_authored_key(bone, self.clip.frame)) => {
                self.studio_delete_selected_clip_bone_keyframe();
                true
            }
            TimelineShortcut::SelectAllKeys => self.select_all_timeline_keys(),
            TimelineShortcut::ClearKeySelection if !self.clip.selected_keys.is_empty() => {
                self.clip.selected_keys.clear();
                true
            }
            TimelineShortcut::SaveProject if !self.clip.busy => {
                self.queue_studio_file_action(files::Action::SaveProject);
                true
            }
            TimelineShortcut::Undo
            | TimelineShortcut::Redo
            | TimelineShortcut::PastePose
            | TimelineShortcut::DeleteKey
            | TimelineShortcut::ClearKeySelection
            | TimelineShortcut::SaveProject => false,
        };
        if ran {
            // egui holds the same press for its next pass: the build tools
            // must not also delete an object or undo an edit with it
            if self.clip.taken_keys.len() >= 16 {
                self.clip.taken_keys.remove(0);
            }
            self.clip.taken_keys.push((key, modifiers));
        }
        ran
    }

    /// The presses the timeline acted on since the last pass, for the caller
    /// to take out of egui's input before the build tools read it.
    pub(crate) fn take_timeline_keys(&mut self) -> Vec<(egui::Key, egui::Modifiers)> {
        std::mem::take(&mut self.clip.taken_keys)
    }

    /// Step the playhead one frame (Left and Right arrows), within the range.
    fn step_timeline_frame(&mut self, forward: bool) {
        let Some((start, end)) = self.clip.document.as_ref().map(|document| (document.start, document.end)) else {
            return;
        };
        let now = self.clip.frame.max(start).min(end);
        let next = if forward {
            now.saturating_add(1).min(end)
        } else {
            now.saturating_sub(1).max(start)
        };
        if next != self.clip.frame || self.clip.playing {
            self.clip_seek(next);
        }
    }

    /// Commit a key drag once no button is down any more, to the frame the
    /// drag last reached: the pointer may be gone at the release, and the
    /// lane of the key may have scrolled out of view.
    pub(super) fn finish_timeline_key_drag(&mut self, pointer_down: bool) {
        if pointer_down {
            return;
        }
        let Some((bone, from, to)) = self.clip.drag_key.take() else {
            return;
        };
        if from == to {
            return;
        }
        let group = self.clip.selected_keys.len() > 1 && self.clip.selected_keys.contains(&(bone, from));
        self.clip.checkpoint();
        if let Some(document) = self.clip.document.as_mut() {
            if group {
                // every picked key moves by the same number of frames, and
                // the selection follows them
                let applied = Arc::make_mut(document).move_keys(&self.clip.selected_keys, i64::from(to) - i64::from(from));
                self.clip.selected_keys = self
                    .clip
                    .selected_keys
                    .iter()
                    .map(|(bone, frame)| (*bone, (i64::from(*frame) + applied) as u32))
                    .collect();
            } else {
                Arc::make_mut(document).move_track_keyframe(bone, from, to);
                self.clip.selected_keys = BTreeSet::from([(bone, to)]);
            }
            self.clip.changed();
        }
        self.clip_seek(to);
    }

    /// Pick every key of the lanes the timeline shows (Ctrl+A). False, and
    /// the key left to the rest of the viewer, when there is none.
    fn select_all_timeline_keys(&mut self) -> bool {
        let Some(tracks) = self.clip.timeline_tracks() else { return false };
        let selected = self.selected;
        let keys: BTreeSet<(usize, u32)> = tracks
            .iter()
            .filter(|track| track.authored || track.keys.len() > 1 || selected == Some(track.bone))
            .flat_map(|track| track.keys.iter().map(|key| (track.bone, *key)))
            .collect();
        if keys.is_empty() {
            return false;
        }
        self.clip.selected_keys = keys;
        true
    }

    /// Shift or Ctrl pressed over the lanes arms a box; once the press turns
    /// into a drag the box starts from where it was pressed, on a key, between
    /// two rows or under the last one alike, so that a dense lane can still be
    /// boxed. A click without a drag stays a click on the key under it.
    fn arm_key_box(&mut self, ui: &egui::Ui, lanes_area: egui::Rect) {
        let (pressed, modifiers, origin, dragging, latest) = ui.input(|input| {
            (
                input.pointer.primary_pressed(),
                input.modifiers,
                input.pointer.press_origin(),
                input.pointer.is_decidedly_dragging() && input.pointer.primary_down(),
                input.pointer.latest_pos(),
            )
        });
        let over_timeline = origin.is_some_and(|origin| ui.ctx().layer_id_at(origin) == Some(ui.layer_id()));
        if pressed
            && (modifiers.shift || modifiers.ctrl || modifiers.command)
            && over_timeline
            && origin.is_some_and(|origin| lanes_area.contains(origin))
        {
            self.clip.key_box_armed = true;
        }
        if self.clip.key_box_armed
            && dragging
            && self.clip.key_box.is_none()
            && let Some(origin) = origin
        {
            self.clip.drag_key = None;
            self.clip.key_box = Some(KeyBox {
                origin,
                current: latest.unwrap_or(origin),
                hits: Vec::new(),
            });
        }
    }

    /// The shift a drag of picked keys gives them all, and the frame it
    /// started from: `None` unless several keys are picked and the grabbed
    /// key is one of them.
    fn group_drag(&self) -> Option<i64> {
        let (bone, origin, to) = self.clip.drag_key?;
        if self.clip.selected_keys.len() < 2 || !self.clip.selected_keys.contains(&(bone, origin)) {
            return None;
        }
        let max_frame = self.clip.document.as_ref().map_or(MAX_FRAME, |document| document.max_frame());
        document::clamp_shift(
            self.clip.selected_keys.iter().map(|(_, frame)| *frame),
            i64::from(to) - i64::from(origin),
            max_frame,
        )
    }

    /// Change the auto-key step. `checkpoint` is true once per edit (a drag's
    /// start, or a typed value), so that a drag of the value fills one undo
    /// entry instead of one per frame.
    pub(super) fn set_timeline_auto_key_step(&mut self, seconds: f32, checkpoint: bool) {
        let Some(document) = self.clip.document.as_ref() else { return };
        let start = document.start as f32 / document.fps.max(1) as f32;
        let keyed_after_start = document.tracks.iter().any(|track| {
            track.motion.rot_keys.iter().any(|key| key.time > start) || track.motion.pos_keys.iter().any(|key| key.time > start)
        });
        if keyed_after_start {
            self.clip.auto_key_retime_from.get_or_insert(document.auto_key_advance_seconds);
        }
        if checkpoint {
            self.clip.checkpoint();
        }
        if let Some(document) = self.clip.document.as_mut() {
            Arc::make_mut(document).auto_key_advance_seconds = seconds.clamp(0.1, 5.0);
            self.clip.changed();
        }
    }

    fn set_timeline_auto_key(&mut self, on: bool) {
        self.clip.checkpoint();
        if let Some(document) = self.clip.document.as_mut() {
            Arc::make_mut(document).auto_keyframe = on;
            self.clip.changed();
        }
    }

    /// Retime the existing keys from the previous auto-key step to the
    /// current one; nothing changes, the undo history included, when the
    /// keys would collide or pass the length limit.
    fn retime_timeline_keys(&mut self, old_step: f32) {
        let Some(document) = self.clip.document.as_ref() else { return };
        let new_step = document.auto_key_advance_seconds;
        let mut retimed = Document::clone(document);
        if retimed.retime_keys_to_step(old_step, new_step) {
            self.clip.checkpoint();
            self.clip.document = Some(Arc::new(retimed));
            self.clip.auto_key_retime_from = None;
            self.clip.changed();
        } else {
            self.clip.error = Some(aurora_i18n::text("anim-edit-auto-key-retime-limit").to_owned());
        }
    }

    fn set_timeline_track_enabled(&mut self, bone: usize, enabled: bool) {
        self.clip.checkpoint();
        if let Some(document) = self.clip.document.as_mut()
            && let Some(track) = Arc::make_mut(document).tracks.iter_mut().find(|track| track.bone == bone)
        {
            track.enabled = enabled;
            track.authored |= enabled;
        }
        self.clip.changed();
    }

    pub(in crate::ik_edit) fn studio_timeline_overlay(&mut self, ctx: &egui::Context, palette: &Palette, world: &World) {
        if !self.clip.active || !self.open || self.clip.waiting_source() || !self.clip.timeline_open {
            // a key drag ends with the lanes that showed it
            self.clip.drag_key = None;
            self.clip.key_box = None;
            self.clip.key_box_armed = false;
            return;
        }
        let Some((start, end)) = self.clip.document.as_ref().map(|document| (document.start, document.end)) else {
            return;
        };
        self.clip.timeline_start = follow_timeline_playhead(
            start,
            end,
            self.clip.timeline_zoom,
            self.clip.timeline_start,
            self.clip.frame,
            self.clip.playing,
        );
        let frame = Frame::new(world);
        let content = ctx.content_rect();
        let size = timeline_default_size(content);
        let pos = timeline_default_position(content, size);
        let mut open = self.clip.timeline_open;
        let mut floater = widgets::Floater::new(WINDOW_ID, aurora_i18n::tr!("avatar-studio-animation-timeline"), pos, size);
        floater.min_size = egui::vec2(content.width().min(700.0), content.height().min(320.0));
        floater.order = egui::Order::Foreground;
        floater.steady_size = true;
        floater.show(ctx, palette, &mut open, |ui| {
            self.timeline_toolbar(ui, palette, world, frame.as_ref());
            ui.add_space(4.0);
            self.timeline_body(ui, palette, &world.avatar_lib.rig, frame.as_ref());
        });
        self.clip.timeline_open = open;
        if self.clip.playing {
            ctx.request_repaint();
        }
    }

    /// The pinned toolbar: one row of groups that wraps on narrow widths.
    fn timeline_toolbar(&mut self, ui: &mut egui::Ui, palette: &Palette, world: &World, frame: Option<&Frame>) {
        let Some((start, end, fps)) = self
            .clip
            .document
            .as_ref()
            .map(|document| (document.start, document.end, document.fps))
        else {
            return;
        };
        let tracks = self.clip.timeline_tracks().unwrap_or_default();
        let now = self.clip.frame;
        let previous_key = neighbour_key(&tracks, self.selected, now, false).filter(|key| *key >= start);
        let next_key = neighbour_key(&tracks, self.selected, now, true).filter(|key| *key <= end);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(4.0, 6.0);
            self.timeline_transport(ui, palette, start, previous_key, next_key);
            ui.add_space(GROUP_GAP);
            self.timeline_readout(ui, start, end, fps);
            ui.add_space(GROUP_GAP);
            self.timeline_tools(ui, palette, world, frame);
            ui.add_space(GROUP_GAP);
            self.timeline_keying(ui, palette);
            ui.add_space(GROUP_GAP);
            self.timeline_edit(ui, palette);
            ui.add_space(GROUP_GAP);
            self.timeline_view_buttons(ui, palette, start, end);
            ui.add_space(GROUP_GAP);
            self.timeline_file_menu(ui, palette);
        });
    }

    fn timeline_transport(&mut self, ui: &mut egui::Ui, palette: &Palette, start: u32, previous_key: Option<u32>, next_key: Option<u32>) {
        if tool_button(ui, palette, "player-skip-back-filled", true)
            .on_hover_text(aurora_i18n::tr!("anim-edit-shortcut-rewind"))
            .clicked()
        {
            self.clip_seek(start);
        }
        if tool_button(ui, palette, "player-track-prev-filled", previous_key.is_some())
            .on_hover_text(aurora_i18n::tr!("anim-timeline-prev-key"))
            .clicked()
            && let Some(key) = previous_key
        {
            self.clip_seek(key);
        }
        let play = if self.clip.playing {
            "player-pause-filled"
        } else {
            "player-play-filled"
        };
        if widgets::action_button(ui, palette, play, "", widgets::ActionKind::Primary, ICON_BUTTON)
            .on_hover_text(aurora_i18n::tr!("anim-edit-shortcut-play"))
            .clicked()
        {
            self.studio_toggle_clip_playback();
        }
        if tool_button(ui, palette, "player-track-next-filled", next_key.is_some())
            .on_hover_text(aurora_i18n::tr!("anim-timeline-next-key"))
            .clicked()
            && let Some(key) = next_key
        {
            self.clip_seek(key);
        }
        if tool_toggle(ui, palette, "repeat", self.clip.timeline_looping)
            .on_hover_text(aurora_i18n::tr!("anim-edit-timeline-loop-hint"))
            .clicked()
        {
            self.clip.timeline_looping = !self.clip.timeline_looping;
        }
        if self.clip.timeline_looping {
            let range = |last_key: bool| {
                if last_key {
                    aurora_i18n::tr!("anim-edit-loop-last-key")
                } else {
                    aurora_i18n::tr!("anim-edit-loop-full-range")
                }
            };
            egui::ComboBox::from_id_salt("animation-timeline-loop-range")
                .selected_text(range(self.clip.timeline_loop_last_key))
                .width(120.0)
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.clip.timeline_loop_last_key, false, range(false));
                    ui.selectable_value(&mut self.clip.timeline_loop_last_key, true, range(true));
                });
        }
    }

    /// Frame / end and seconds, in a width that fits the longest value so
    /// that the toolbar does not shift while playing.
    fn timeline_readout(&self, ui: &mut egui::Ui, start: u32, end: u32, fps: u32) {
        let seconds = |frame: u32| format!("{:.2}", frame.saturating_sub(start) as f32 / fps.max(1) as f32);
        let now = self.clip.frame;
        let text = aurora_i18n::tr!("anim-timeline-readout", frame = now, end = end, seconds = seconds(now));
        let widest = aurora_i18n::tr!("anim-timeline-readout", frame = end, end = end, seconds = seconds(end));
        let font = egui::TextStyle::Body.resolve(ui.style());
        let width = ui
            .painter()
            .layout_no_wrap(widest, font, egui::Color32::PLACEHOLDER)
            .size()
            .x
            .ceil()
            + 2.0;
        ui.allocate_ui_with_layout(
            egui::vec2(width, TOOL_HEIGHT),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.add(egui::Label::new(text).selectable(false)).on_hover_text(aurora_i18n::tr!(
                    "anim-timeline-readout-hint",
                    frame = now,
                    end = end,
                    fps = fps
                ));
            },
        );
    }

    /// Move | Rotate, IK | FK for the selected bone, Mirror and limb handles.
    fn timeline_tools(&mut self, ui: &mut egui::Ui, palette: &Palette, world: &World, frame: Option<&Frame>) {
        let mut tool = usize::from(self.tool == Tool::Rotate);
        if tool_segmented(
            ui,
            palette,
            "anim-timeline-tool",
            &mut tool,
            &[(aurora_i18n::tr!("ik-edit-move"), false), (aurora_i18n::tr!("ik-edit-turn"), false)],
            true,
            aurora_i18n::tr!("anim-timeline-tool-hint"),
        ) {
            self.tool = if tool == 0 { Tool::Move } else { Tool::Rotate };
            self.drag = None;
            self.gizmo.cancel();
        }
        let rig = &world.avatar_lib.rig;
        // the skeleton root has no chain to solve: it only moves directly
        let chained = self.selected.is_some_and(|bone| rig.parent.get(bone).is_some_and(Option::is_some));
        let mut fk = usize::from(!chained || self.mode == Mode::Direct);
        if tool_segmented(
            ui,
            palette,
            "anim-timeline-chain",
            &mut fk,
            &[
                (aurora_i18n::tr!("anim-timeline-ik"), false),
                (aurora_i18n::tr!("anim-timeline-fk"), false),
            ],
            chained && frame.is_some(),
            aurora_i18n::tr!("anim-timeline-ik-hint"),
        ) && let Some(frame) = frame
        {
            self.set_chain_ik(fk == 0, rig, frame);
        }
        if tool_toggle(ui, palette, "arrows-horizontal", self.symmetry)
            .on_hover_text(aurora_i18n::tr!("anim-timeline-mirror"))
            .clicked()
        {
            self.symmetry = !self.symmetry;
        }
        if tool_toggle(ui, palette, "hand-grab", self.limb_controls)
            .on_hover_text(aurora_i18n::tr!("anim-timeline-limb-handles"))
            .clicked()
        {
            self.limb_controls = !self.limb_controls;
        }
    }

    /// Key the selected bone, auto-key with its step, and the retime of the
    /// existing keys after a step change.
    fn timeline_keying(&mut self, ui: &mut egui::Ui, palette: &Palette) {
        let Some((auto_key, step)) = self
            .clip
            .document
            .as_ref()
            .map(|document| (document.auto_keyframe, document.auto_key_advance_seconds))
        else {
            return;
        };
        if widgets::action_button_if(
            ui,
            palette,
            "keyframe-filled",
            aurora_i18n::tr!("anim-timeline-key"),
            widgets::ActionKind::Flat,
            egui::vec2(0.0, TOOL_HEIGHT),
            self.selected.is_some(),
        )
        .on_hover_text(aurora_i18n::tr!("anim-timeline-key-hint"))
        .clicked()
            && let Some(bone) = self.selected
        {
            self.clip.capture = Some(Capture::Bone(bone));
        }
        if tool_toggle(ui, palette, "player-record-filled", auto_key)
            .on_hover_ui(|ui| {
                ui.strong(aurora_i18n::tr!("anim-edit-auto-keyframe"));
                ui.label(aurora_i18n::tr!("anim-edit-auto-keyframe-hint"));
            })
            .clicked()
        {
            self.set_timeline_auto_key(!auto_key);
        }
        if auto_key {
            let mut seconds = step;
            let response = ui
                .add_sized(
                    egui::vec2(52.0, TOOL_HEIGHT),
                    egui::DragValue::new(&mut seconds)
                        .range(0.1..=5.0)
                        .speed(0.02)
                        .fixed_decimals(1)
                        .suffix(aurora_i18n::tr!("anim-edit-seconds-short")),
                )
                .on_hover_text(aurora_i18n::tr!("anim-edit-auto-key-step-hint"));
            if response.changed() {
                self.set_timeline_auto_key_step(seconds, super::ui::checkpoint_on_edit(&response));
            }
        }
        if let Some(old_step) = self.clip.auto_key_retime_from
            && widgets::action_button(
                ui,
                palette,
                "",
                aurora_i18n::tr!("anim-edit-auto-key-retime"),
                widgets::ActionKind::Flat,
                egui::vec2(0.0, TOOL_HEIGHT),
            )
            .on_hover_text(aurora_i18n::tr!("anim-edit-auto-key-retime-hint"))
            .clicked()
        {
            self.retime_timeline_keys(old_step);
        }
    }

    /// Undo, redo, copy and paste the pose, delete the selected bone's key.
    fn timeline_edit(&mut self, ui: &mut egui::Ui, palette: &Palette) {
        if tool_button(ui, palette, "arrow-back-up", !self.clip.undo.is_empty())
            .on_hover_text(aurora_i18n::tr!("anim-edit-shortcut-undo"))
            .clicked()
        {
            self.studio_clip_undo();
        }
        if tool_button(ui, palette, "arrow-forward-up", !self.clip.redo.is_empty())
            .on_hover_text(aurora_i18n::tr!("anim-edit-shortcut-redo"))
            .clicked()
        {
            self.studio_clip_redo();
        }
        if tool_button(ui, palette, "copy-filled", true)
            .on_hover_text(aurora_i18n::tr!("anim-edit-shortcut-copy"))
            .clicked()
        {
            self.studio_copy_clip_keyframe();
        }
        if tool_button(ui, palette, "clipboard-text-filled", self.has_clip_clipboard())
            .on_hover_text(aurora_i18n::tr!("anim-edit-shortcut-paste"))
            .clicked()
        {
            self.studio_paste_clip_keyframe();
        }
        let picked = self.has_deletable_key_selection();
        let selected_key = picked || self.selected.is_some_and(|bone| self.clip.has_authored_key(bone, self.clip.frame));
        if tool_button(ui, palette, "trash-filled", selected_key)
            .on_hover_text(aurora_i18n::tr!("anim-edit-shortcut-delete"))
            .clicked()
        {
            if picked {
                self.studio_delete_selected_keys();
            } else {
                self.studio_delete_selected_clip_bone_keyframe();
            }
        }
    }

    /// Zoom out, zoom in around the playhead, and fit the whole animation.
    fn timeline_view_buttons(&mut self, ui: &mut egui::Ui, palette: &Palette, start: u32, end: u32) {
        let zoom = self.clip.timeline_zoom;
        let mut zoomed = None;
        if tool_button(ui, palette, "zoom-out-filled", zoom > 1.0)
            .on_hover_text(aurora_i18n::tr!("anim-timeline-zoom-out"))
            .clicked()
        {
            zoomed = Some((zoom / 2.0).max(1.0));
        }
        if tool_button(ui, palette, "zoom-in-filled", zoom < MAX_TIMELINE_ZOOM)
            .on_hover_text(aurora_i18n::tr!("anim-timeline-zoom-in"))
            .clicked()
        {
            zoomed = Some((zoom * 2.0).min(MAX_TIMELINE_ZOOM));
        }
        if let Some(zoom) = zoomed {
            self.clip.timeline_zoom = zoom;
            self.clip.timeline_start = timeline_start_around(start, end, zoom, self.clip.frame);
        }
        if tool_button(ui, palette, "arrow-autofit-width-filled", true)
            .on_hover_text(aurora_i18n::tr!("anim-timeline-fit"))
            .clicked()
        {
            self.clip.timeline_zoom = 1.0;
            self.clip.timeline_start = start;
        }
    }

    /// The file actions and the rarer, wider edits, in one menu.
    fn timeline_file_menu(&mut self, ui: &mut egui::Ui, palette: &Palette) {
        let busy = self.clip.busy;
        let can_open = self.clip.file_action_allowed(files::Action::Load);
        let frame_keyed = self.clip.has_key(None, self.clip.frame);
        let exportable =
            !busy && !self.clip.waiting_source() && self.clip.document.as_ref().is_some_and(|document| document.has_exportable_tracks());
        let button = widgets::action_menu_button(
            ui,
            palette,
            "folder-filled",
            aurora_i18n::tr!("anim-timeline-file"),
            false,
            TOOL_HEIGHT,
        );
        let mut action = None;
        let mut delete_frame_keys = false;
        menu::dropdown(&button, palette, egui::PopupCloseBehavior::CloseOnClick, |ui| {
            // greyed out with the drafts full, like Create: opening would
            // have to drop the current document
            if menu::item_if(ui, palette, "file-import", aurora_i18n::tr!("anim-edit-open-file"), can_open) {
                action = Some(files::Action::Load);
            }
            if menu::item_if(
                ui,
                palette,
                "folder-open-filled",
                aurora_i18n::tr!("anim-edit-open-project"),
                can_open,
            ) {
                action = Some(files::Action::LoadProject);
            }
            if menu::item_tip(
                ui,
                palette,
                "device-floppy-filled",
                aurora_i18n::tr!("anim-edit-save-project"),
                !busy,
                aurora_i18n::tr!("anim-edit-shortcut-save"),
            ) {
                action = Some(files::Action::SaveProject);
            }
            let last_key_only = self.clip.timeline_looping && self.clip.timeline_loop_last_key;
            if menu::item_if(ui, palette, "file-export", aurora_i18n::tr!("anim-edit-save"), exportable) {
                action = Some(files::Action::Save { last_key_only });
            }
            if menu::item_tip(
                ui,
                palette,
                "file-export",
                aurora_i18n::tr!("anim-edit-export-bvh"),
                exportable,
                aurora_i18n::tr!("anim-edit-export-bvh-hint"),
            ) {
                action = Some(files::Action::ExportBvh { last_key_only });
            }
            menu::separator(ui, palette);
            if menu::item_if(
                ui,
                palette,
                "trash-filled",
                aurora_i18n::tr!("anim-timeline-delete-frame-keys"),
                frame_keyed,
            ) {
                delete_frame_keys = true;
            }
        });
        if let Some(action) = action {
            self.queue_studio_file_action(action);
        }
        if delete_frame_keys {
            self.studio_delete_clip_keyframe();
        }
    }

    /// What the status line says: an error, the work under way, or the last
    /// message; `true` with an error.
    fn timeline_status(&self) -> Option<(String, bool)> {
        if let Some(error) = &self.clip.error {
            Some((error.clone(), true))
        } else if self.clip.busy {
            Some((aurora_i18n::tr!("anim-edit-preparing").to_owned(), false))
        } else if let Some(message) = self.clip.message.clone() {
            Some((message, false))
        } else if self.clip.selected_keys.len() > 1 {
            Some((
                aurora_i18n::tr!("anim-timeline-keys-selected", count = self.clip.selected_keys.len()).to_string(),
                false,
            ))
        } else {
            None
        }
    }

    /// The ruler, the lanes under one playhead, and the status line.
    fn timeline_body(&mut self, ui: &mut egui::Ui, palette: &Palette, rig: &Rig, frame: Option<&Frame>) {
        let Some((start, end)) = self.clip.document.as_ref().map(|document| (document.start, document.end)) else {
            return;
        };
        let tracks = self.clip.timeline_tracks().unwrap_or_default();
        // keys an undo, a delete or a range change took away are no longer picked
        self.clip.selected_keys.retain(|(bone, frame)| {
            tracks
                .iter()
                .any(|track| track.bone == *bone && track.keys.binary_search(frame).is_ok())
        });
        if let Some(key_box) = self.clip.key_box.as_mut() {
            if let Some(pointer) = ui.input(|input| input.pointer.latest_pos()) {
                key_box.current = pointer;
            }
            key_box.hits.clear();
        }
        let selected = self.selected;
        let shown: Vec<&TimelineTrack> = tracks
            .iter()
            .filter(|track| track.authored || track.keys.len() > 1 || selected == Some(track.bone))
            .collect();
        let status = self.timeline_status();
        let spacing = ui.spacing().item_spacing.y;
        let status_height = if status.is_some() {
            ui.spacing().interact_size.y + spacing
        } else {
            0.0
        };
        let list_height = (ui.available_height() - RULER_HEIGHT - spacing - status_height).max(ROW_HEIGHT);
        // the rows take the width the scroll area leaves them, known before
        // they are drawn, so that the ruler above has exactly their width
        let rows_height = shown.len() as f32 * (ROW_HEIGHT + ROW_GAP) - ROW_GAP;
        let scroll_bar = rows_height > list_height + 0.5;
        let width = (ui.available_width() - if scroll_bar { widgets::scroll_strip(ui) } else { 0.0 }).max(1.0);
        let name_width = (width * 0.28).clamp(96.0, 168.0);
        let (ruler_row, _) = ui.allocate_exact_size(egui::vec2(width, RULER_HEIGHT), egui::Sense::hover());
        let ruler = egui::Rect::from_min_max(egui::pos2(ruler_row.left() + name_width, ruler_row.top()), ruler_row.max);
        let list_bottom = ruler.bottom() + spacing + list_height;
        self.timeline_wheel(
            ui,
            egui::Rect::from_min_max(ruler.min, egui::pos2(ruler.right(), list_bottom)),
            ruler,
            start,
            end,
        );
        let lanes_area = egui::Rect::from_min_max(egui::pos2(ruler.left(), ruler.bottom()), egui::pos2(ruler.right(), list_bottom));
        self.arm_key_box(ui, lanes_area);
        let (view_start, view_end) = timeline_view(start, end, self.clip.timeline_zoom, self.clip.timeline_start);
        self.clip.timeline_start = view_start;
        let scale = FrameScale::new(ruler, view_start, view_end);
        ui.painter().text(
            egui::pos2(ruler_row.left() + 4.0, ruler.center().y),
            egui::Align2::LEFT_CENTER,
            format!("{view_start}\u{2013}{view_end}"),
            egui::FontId::proportional(RULER_FONT),
            palette.muted,
        );
        paint_ruler(ui, palette, ruler, list_bottom, &scale);
        let scrub = ui.interact(ruler, ui.scope_id().with("anim-timeline-ruler"), egui::Sense::click_and_drag());
        if scrub.is_pointer_button_down_on()
            && let Some(pointer) = scrub.interact_pointer_pos()
        {
            let to = scale.frame_at(pointer.x);
            if to != self.clip.frame || self.clip.playing {
                self.clip_seek(to);
            }
        }
        let lanes = Lanes {
            name_width,
            start: view_start,
            end: view_end,
        };
        let rows_bottom = if shown.is_empty() {
            let (rect, _) = ui.allocate_exact_size(egui::vec2(width, list_height), egui::Sense::hover());
            let mut hint = ui.new_child(egui::UiBuilder::new().max_rect(rect.shrink2(egui::vec2(4.0, 6.0))));
            hint.add(egui::Label::new(egui::RichText::new(aurora_i18n::tr!("anim-timeline-empty")).color(palette.muted)).wrap());
            ruler.bottom()
        } else {
            let output = ui
                .scope(|ui| {
                    ui.spacing_mut().item_spacing.y = ROW_GAP;
                    egui::ScrollArea::vertical()
                        .id_salt("avatar-studio-animation-timeline-tracks")
                        .auto_shrink([false, false])
                        .max_height(list_height)
                        .show_rows(ui, ROW_HEIGHT, shown.len(), |ui, rows| {
                            for track in rows.filter_map(|index| shown.get(index)) {
                                self.timeline_row(ui, palette, rig, frame, track, lanes);
                            }
                        })
                })
                .inner;
            output.inner_rect.bottom().min(output.inner_rect.top() + output.content_size.y)
        };
        let pointer_down = ui.input(|input| input.pointer.any_down());
        self.finish_timeline_key_drag(pointer_down);
        if !pointer_down {
            self.clip.key_box_armed = false;
        }
        if let Some(key_box) = &self.clip.key_box {
            let area = key_box.area();
            let painter = ui.painter_at(lanes_area);
            painter.rect_filled(area, ui.visuals().window_corner_radius, palette.violet.gamma_multiply(0.12));
            painter.rect_stroke(
                area,
                ui.visuals().window_corner_radius,
                egui::Stroke::new(1.0, palette.violet),
                egui::StrokeKind::Inside,
            );
            if !pointer_down && let Some(key_box) = self.clip.key_box.take() {
                self.clip.selected_keys.extend(key_box.hits);
            }
        }
        let cursor = self.clip.cursor.max(f64::from(start)).min(f64::from(end));
        paint_playhead(ui, palette, ruler, rows_bottom, &scale, cursor, self.clip.frame);
        if let Some((text, error)) = status {
            let color = if error { palette.danger } else { palette.muted };
            ui.add(egui::Label::new(egui::RichText::new(&text).color(color)).truncate())
                .on_hover_text(text);
        }
    }

    /// Ctrl+wheel over the ruler or the lanes zooms around the frame under
    /// the pointer, Shift+wheel (or a horizontal wheel) pans; a plain wheel
    /// is left to the list of tracks.
    fn timeline_wheel(&mut self, ui: &egui::Ui, area: egui::Rect, ruler: egui::Rect, start: u32, end: u32) {
        if !ui.rect_contains_pointer(area) {
            return;
        }
        let Some(pointer) = ui.ctx().pointer_hover_pos() else { return };
        let lane_left = ruler.left() + LANE_PAD;
        let lane_width = (ruler.width() - 2.0 * LANE_PAD).max(1.0);
        let zoom = ui.input(|input| input.zoom_delta());
        if (zoom - 1.0).abs() > 1e-4 {
            (self.clip.timeline_zoom, self.clip.timeline_start) = zoom_timeline_around(
                start,
                end,
                self.clip.timeline_zoom,
                self.clip.timeline_start,
                (pointer.x - lane_left) / lane_width,
                zoom,
            );
        }
        let pan = ui.input(|input| input.smooth_scroll_delta.x);
        if pan == 0.0 {
            return;
        }
        ui.input_mut(|input| input.smooth_scroll_delta.x = 0.0);
        let (left, right) = timeline_view(start, end, self.clip.timeline_zoom, self.clip.timeline_start);
        // the wheel's points become frames; the part of a frame left over
        // carries to the next event, for a smooth pan when zoomed in
        let carry = egui::Id::unique("avatar-studio-animation-timeline-pan");
        let frames = ui.data(|data| data.get_temp::<f32>(carry)).unwrap_or(0.0) - pan * (right - left).max(1) as f32 / lane_width;
        let whole = frames.trunc();
        ui.data_mut(|data| data.insert_temp(carry, frames - whole));
        if whole != 0.0 {
            self.clip.timeline_start = pan_timeline(start, end, self.clip.timeline_zoom, self.clip.timeline_start, whole as i64);
        }
    }

    /// One bone's row: switch and name in the name column, then its lane.
    fn timeline_row(
        &mut self,
        ui: &mut egui::Ui,
        palette: &Palette,
        rig: &Rig,
        frame: Option<&Frame>,
        track: &TimelineTrack,
        lanes: Lanes,
    ) {
        let (row, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), ROW_HEIGHT), egui::Sense::hover());
        let bone = track.bone;
        if self.selected == Some(bone) {
            ui.painter()
                .rect_filled(row, ui.visuals().window_corner_radius, palette.violet.gamma_multiply(0.16));
        }
        let names = egui::Rect::from_min_size(row.min, egui::vec2(lanes.name_width, ROW_HEIGHT));
        let lane = egui::Rect::from_min_max(egui::pos2(row.left() + lanes.name_width, row.top()), row.max);
        let mut name_ui = ui.new_child(
            egui::UiBuilder::new()
                .id_salt(("anim-timeline-name", bone))
                .max_rect(names.shrink2(egui::vec2(4.0, 0.0)))
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        let mut enabled = track.enabled;
        if widgets::checkbox(&mut name_ui, palette, &mut enabled, "")
            .on_hover_text(aurora_i18n::tr!("anim-timeline-track-enabled"))
            .changed()
        {
            self.set_timeline_track_enabled(bone, enabled);
        }
        let name = egui::RichText::new(&track.name).color(if track.enabled { palette.ink } else { palette.muted });
        if name_ui.add(egui::Button::selectable(false, name).truncate()).clicked()
            && let Some(frame) = frame
        {
            self.select(bone, rig, frame);
        }
        self.timeline_lane(ui, palette, rig, frame, track, lane, FrameScale::new(lane, lanes.start, lanes.end));
    }

    /// A bone's keys as diamonds on its lane: hover shows a key's frame,
    /// dragging a key moves it (and every other picked key with it), a click
    /// or a drag elsewhere moves the playhead. Clicking or grabbing a key
    /// picks it (`pick_key`) and selects its bone, so that Delete and the
    /// toolbar act on the keys just picked. Shift or Ctrl and a drag anywhere
    /// over the lanes draws a box instead (`arm_key_box`).
    #[allow(clippy::too_many_arguments)]
    fn timeline_lane(
        &mut self,
        ui: &mut egui::Ui,
        palette: &Palette,
        rig: &Rig,
        frame: Option<&Frame>,
        track: &TimelineTrack,
        lane: egui::Rect,
        scale: FrameScale,
    ) {
        let bone = track.bone;
        let response = ui.interact(
            lane,
            egui::Id::unique(("avatar-studio-animation-timeline-lane", bone)),
            egui::Sense::click_and_drag(),
        );
        let grabbing = self.clip.drag_key.is_some_and(|(dragged, _, _)| dragged == bone);
        let hovered_key = if grabbing {
            None
        } else {
            response.hover_pos().and_then(|pointer| key_under(&track.keys, &scale, pointer.x))
        };
        let modifiers = ui.input(|input| input.modifiers);
        if response.drag_started() && !self.clip.key_box_armed {
            // the key under the press, not under the pointer once the drag
            // threshold is passed
            let origin = ui.input(|input| input.pointer.press_origin()).or(response.interact_pointer_pos());
            let key = origin.and_then(|pointer| key_under(&track.keys, &scale, pointer.x));
            self.clip.drag_key = key.map(|key| (bone, key, key));
            if let Some(key) = key {
                pick_key(&mut self.clip.selected_keys, (bone, key), modifiers, true);
                self.clip_seek(key);
                if let Some(frame) = frame {
                    self.select(bone, rig, frame);
                }
            }
        }
        if let Some(key_box) = self.clip.key_box.as_mut() {
            let hits = keys_in_box(&track.keys, &scale, lane, key_box.area());
            key_box.hits.extend(hits.into_iter().map(|key| (bone, key)));
        }
        if response.dragged()
            && self.clip.key_box.is_none()
            && let Some(pointer) = response.interact_pointer_pos()
        {
            let to = scale.frame_at(pointer.x);
            match self.clip.drag_key {
                Some((dragged, origin, current)) if dragged == bone => {
                    if current != to {
                        self.clip.drag_key = Some((bone, origin, to));
                        self.clip_seek(to);
                    }
                }
                Some(_) => {}
                None if to != self.clip.frame => self.clip_seek(to),
                None => {}
            }
        }
        if response.clicked()
            && let Some(pointer) = response.interact_pointer_pos()
        {
            self.clip_seek(hovered_key.unwrap_or_else(|| scale.frame_at(pointer.x)));
            match hovered_key {
                Some(key) => {
                    pick_key(&mut self.clip.selected_keys, (bone, key), modifiers, false);
                    if let Some(frame) = frame {
                        self.select(bone, rig, frame);
                    }
                }
                // a plain click on an empty spot drops the picked keys
                None if !(modifiers.shift || modifiers.ctrl || modifiers.command) => self.clip.selected_keys.clear(),
                None => {}
            }
        }
        let group = self.group_drag();
        let moving = self.clip.drag_key.filter(|(dragged, _, _)| group.is_none() && *dragged == bone);
        if (moving.is_some() || (grabbing && group.is_some())) && response.dragged() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        } else if let Some(key) = hovered_key {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
            response.on_hover_text(aurora_i18n::tr!("anim-edit-keyframe-at", frame = key));
        }
        let painter = ui.painter();
        let center = lane.center().y;
        let base = if track.enabled {
            palette.muted_dim
        } else {
            palette.muted_dim.gamma_multiply(0.5)
        };
        painter.line_segment(
            [
                egui::pos2(scale.x(f64::from(scale.start)), center),
                egui::pos2(scale.x(f64::from(scale.end)), center),
            ],
            egui::Stroke::new(1.0, base),
        );
        let visible =
            &track.keys[track.keys.partition_point(|key| *key < scale.start)..track.keys.partition_point(|key| *key <= scale.end)];
        for &key in visible {
            let picked = self.clip.selected_keys.contains(&(bone, key));
            let left_behind =
                moving.is_some_and(|(_, origin, to)| origin == key && origin != to) || (picked && group.is_some_and(|delta| delta != 0));
            let color = if left_behind {
                palette.violet.gamma_multiply(0.35)
            } else if picked {
                palette.teal
            } else if !track.enabled {
                palette.muted_dim
            } else if Some(key) == hovered_key || (key == self.clip.frame && !self.clip.playing) {
                palette.violet_light
            } else {
                palette.violet
            };
            let half = if Some(key) == hovered_key { KEY_HALF + 1.0 } else { KEY_HALF };
            paint_key(painter, egui::pos2(scale.x(f64::from(key)), center), half, color);
        }
        if let Some((_, origin, to)) = moving
            && origin != to
            && scale.contains(f64::from(to))
        {
            paint_key(painter, egui::pos2(scale.x(f64::from(to)), center), KEY_HALF + 1.0, palette.teal);
        }
        if let Some(delta) = group.filter(|delta| *delta != 0) {
            for (_, key) in self.clip.selected_keys.range((bone, 0)..=(bone, u32::MAX)) {
                let to = f64::from((i64::from(*key) + delta) as u32);
                if scale.contains(to) {
                    paint_key(painter, egui::pos2(scale.x(to), center), KEY_HALF + 1.0, palette.teal);
                }
            }
        }
    }
}
