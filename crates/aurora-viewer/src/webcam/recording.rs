//! Capture only webcam-authored joint rotations and bake native animation assets.

use super::{
    Frame, MediaClock, Pose, companion,
    retarget::Channels,
    send_script, soundtrack,
    take_tools::{self, AudioPass, RotationFrame},
};
use crate::{
    ik_edit::{PreparedAnimation, write_animation_atomic},
    scene::anim::{Rig, WebcamFeet},
    theme::Palette,
    ui::widgets,
};
use aurora_assets::anim::{Animation, JointMotion, RotKey};
use aurora_i18n::tr;
use glam::Quat;
use std::{
    io::Write,
    process::Stdio,
    sync::{Arc, mpsc},
    time::{Duration, Instant},
};

const INTERVAL: f32 = 1.0 / 30.0;
const ASSET_BYTES: usize = 250_000;
const INVENTORY_NAME_BYTES: usize = 63;
const NAME_PREFIX: &str = "Webcam Tracking";
const LONGEST_ASSET_SUFFIX: &str = " Audio 999";

fn take_name(source: &str, stamp: &str, id: uuid::Uuid) -> String {
    let clean: String = source
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_') {
                c
            } else {
                ' '
            }
        })
        .collect();
    let source = clean.split_ascii_whitespace().collect::<Vec<_>>().join(" ");
    let source = if source.is_empty() { "Source" } else { &source };
    let id = id.simple().to_string();
    let tail = format!(" {stamp} {}", &id[..10]);
    let available = INVENTORY_NAME_BYTES - NAME_PREFIX.len() - 1 - tail.len() - LONGEST_ASSET_SUFFIX.len();
    let source = source[..source.len().min(available)].trim_end();
    format!("{NAME_PREFIX} {source}{tail}")
}

pub(super) fn link_label(raw: &str) -> String {
    let Ok(url) = reqwest::Url::parse(raw) else {
        return "Video link".into();
    };
    let host = url.host_str().unwrap_or_default();
    let platform = match host.trim_start_matches("www.").trim_start_matches("m.") {
        "youtube.com" | "youtu.be" => "YouTube",
        "tiktok.com" | "vm.tiktok.com" | "vt.tiktok.com" => "TikTok",
        "instagram.com" => "Instagram",
        "facebook.com" | "fb.watch" | "web.facebook.com" => "Facebook",
        "x.com" | "twitter.com" | "mobile.twitter.com" => "X",
        _ => return host.trim_start_matches("www.").to_owned(),
    };
    let path_id = url
        .path_segments()
        .and_then(|parts| parts.filter(|part| !part.is_empty()).next_back());
    let video_id = if matches!(platform, "YouTube" | "Facebook") {
        url.query_pairs().find(|(key, _)| key == "v").map(|(_, value)| value.into_owned())
    } else {
        None
    };
    let id = video_id.as_deref().or(path_id).unwrap_or_default();
    if id.is_empty() || !id.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_')) {
        return platform.to_owned();
    }
    format!("{platform} {}", &id[..id.len().min(6)])
}

struct Sample {
    time: f32,
    rotations: Vec<Option<Quat>>,
}

struct Capture {
    name: String,
    starts: Instant,
    paused: Option<Instant>,
    audio_clock: Option<Instant>,
    audio_from: Instant,
    limit: f32,
    next_sample: f32,
    names: Vec<String>,
    samples: Vec<Sample>,
    feet: WebcamFeet,
    foot_phases: Vec<(f32, [bool; 2])>,
    audio_passes: Vec<AudioPass>,
    splits: Vec<f32>,
}

struct TakeData {
    name: String,
    names: Vec<String>,
    timed: Vec<(f32, RotationFrame)>,
    feet: Vec<(f32, [bool; 2])>,
    audio: Vec<AudioPass>,
}

#[derive(Clone, Copy, Debug)]
enum Failure {
    Empty,
    Size,
    Write,
    NoLoop,
    NoBeats,
    NoSound,
    NoHand,
}

enum Output {
    Saved(usize),
    Upload(PreparedAnimation),
    Media(PreparedMedia),
    Cancelled,
    Repaired(Animation, Vec<(f32, RotationFrame)>),
    Loop([f32; 2], Animation),
    Beats(Vec<f32>),
    Waveform(Vec<f32>),
}

#[derive(Clone, Copy)]
enum Export {
    Save,
    Upload,
    Pose,
    Segments,
    Splits,
    SoundFile,
    TakeFiles,
    SoundUpload,
    Gesture,
}

pub(crate) struct PreparedMedia {
    pub(crate) animation: Option<PreparedAnimation>,
    pub(crate) sounds: Vec<(f32, aurora_assets::import_sound::Sound)>,
    pub(crate) name: String,
    pub(crate) direct: bool,
}

#[derive(Clone, Copy)]
enum TimelineDrag {
    Start,
    End,
    Split(usize),
    Playhead,
}

pub(super) struct Recorder {
    active: Option<Capture>,
    clip: Option<Arc<Animation>>,
    original: Option<Arc<Animation>>,
    repaired_frames: Option<Arc<Vec<(f32, RotationFrame)>>>,
    take: Option<Arc<TakeData>>,
    limit: u32,
    countdown: bool,
    range: [f32; 2],
    cursor: f32,
    visible: bool,
    playing: Option<(Instant, f32)>,
    looping: bool,
    seam: Option<Arc<Animation>>,
    dropouts: Vec<f32>,
    beats: Vec<f32>,
    waveform: Option<Vec<f32>>,
    splits: Vec<f32>,
    timeline_drag: Option<TimelineDrag>,
    bpm: u32,
    preview_rate: f32,
    repair_joint: Option<usize>,
    repair_width: f32,
    include_sound: bool,
    review_gesture: bool,
    beats_per_clip: usize,
    job: Option<mpsc::Receiver<Result<Output, Failure>>>,
    saved: usize,
    failure: Option<Failure>,
    upload: Option<PreparedAnimation>,
    media: Option<PreparedMedia>,
}

impl Default for Recorder {
    fn default() -> Self {
        Self {
            active: None,
            clip: None,
            original: None,
            repaired_frames: None,
            take: None,
            limit: 30,
            countdown: false,
            range: [0.0; 2],
            cursor: 0.0,
            visible: false,
            playing: None,
            looping: false,
            seam: None,
            dropouts: Vec::new(),
            beats: Vec::new(),
            waveform: None,
            splits: Vec::new(),
            timeline_drag: None,
            bpm: 120,
            preview_rate: 1.0,
            repair_joint: None,
            repair_width: 0.22,
            include_sound: true,
            review_gesture: false,
            beats_per_clip: 4,
            job: None,
            saved: 0,
            failure: None,
            upload: None,
            media: None,
        }
    }
}

impl Recorder {
    pub(super) fn hide_preview(&mut self) {
        self.visible = false;
        self.playing = None;
    }

    pub(super) fn take_upload(&mut self) -> Option<PreparedAnimation> {
        self.poll();
        self.upload.take()
    }

    pub(super) fn take_media(&mut self) -> Option<PreparedMedia> {
        self.poll();
        self.media.take()
    }

    fn start(&mut self, now: Instant, source: &str) {
        self.hide_preview();
        self.failure = None;
        self.saved = 0;
        let delay = if self.countdown { Duration::from_secs(3) } else { Duration::ZERO };
        self.active = Some(Capture {
            name: take_name(
                source,
                &chrono::Local::now().format("%y%m%d-%H%M%S").to_string(),
                uuid::Uuid::new_v4(),
            ),
            starts: now + delay,
            paused: None,
            audio_clock: None,
            audio_from: now + delay,
            limit: self.limit as f32,
            next_sample: 0.0,
            names: Vec::new(),
            samples: Vec::new(),
            feet: WebcamFeet::default(),
            foot_phases: Vec::new(),
            audio_passes: Vec::new(),
            splits: Vec::new(),
        });
    }

    pub(super) fn is_active(&self) -> bool {
        self.active.is_some()
    }

    pub(super) fn pause(&mut self, now: Instant) {
        if let Some(capture) = &mut self.active
            && capture.paused.is_none()
        {
            capture.paused = Some(now);
            capture.audio_clock = None;
            capture.feet = WebcamFeet::default();
        }
    }

    pub(super) fn tick(
        &mut self,
        rig: &Rig,
        now: Instant,
        pose: Option<&Pose>,
        sound: Option<(&take_tools::AudioSource, MediaClock)>,
        mirrored: bool,
    ) {
        let Some(capture) = &mut self.active else { return };
        if capture.paused.is_some() {
            return;
        }
        let Some(elapsed) = now.checked_duration_since(capture.starts) else {
            return;
        };
        let elapsed = elapsed.as_secs_f32();
        let time = elapsed.min(capture.limit);
        if let Some((source, clock)) = sound.filter(|(_, clock)| clock.began <= now) {
            let same_clock = capture.audio_clock == Some(clock.began);
            if !same_clock || capture.audio_passes.last().is_some_and(|pass| pass.mirrored != mirrored) {
                capture.audio_clock = None;
                if capture.audio_passes.len() < 128 {
                    let from = if same_clock { now } else { capture.audio_from.max(clock.began) };
                    capture.audio_passes.push(AudioPass {
                        source: source.clone(),
                        from: from.duration_since(capture.starts).as_secs_f32(),
                        end: time,
                        offset: clock.position(from),
                        rate: clock.rate,
                        mirrored,
                    });
                    capture.audio_clock = Some(clock.began);
                }
            } else if let Some(pass) = capture.audio_passes.last_mut() {
                pass.end = time;
            }
        } else {
            capture.audio_clock = None;
        }
        if time + 0.001 >= capture.next_sample {
            if capture.names.is_empty() {
                capture.names = rig.canonical_names.clone();
            }
            capture.foot_phases.push((
                time,
                pose.map_or([false; 2], |p| p.feet().map(|foot| foot.is_some_and(|f| f.weight > 0.35))),
            ));
            capture.samples.push(Sample {
                time,
                rotations: pose.map_or_else(Vec::new, |p| p.capture(rig, &mut capture.feet)),
            });
            // Keep real elapsed time across a slow render or a dropped camera frame.
            capture.next_sample = (1.0 + ((time + 0.001) / INTERVAL).floor()) * INTERVAL;
        }
        if elapsed >= capture.limit {
            self.finish();
        }
    }

    pub(super) fn finish(&mut self) {
        let Some(capture) = self.active.take() else { return };
        if capture.samples.is_empty() {
            if Instant::now() >= capture.starts {
                self.failure = Some(Failure::Empty);
            }
            return;
        }
        let timed: Vec<_> = capture.samples.iter().map(|frame| (frame.time, frame.rotations.clone())).collect();
        let Some(clip) = assemble(capture.samples, capture.names.clone()) else {
            self.failure = Some(Failure::Empty);
            return;
        };
        self.range = [0.0, clip.duration];
        self.cursor = 0.0;
        let clip = Arc::new(clip);
        self.original = Some(clip.clone());
        self.repaired_frames = None;
        self.repair_joint = None;
        self.clip = Some(clip);
        self.seam = None;
        self.beats.clear();
        self.waveform = None;
        self.timeline_drag = None;
        self.splits = capture
            .splits
            .into_iter()
            .filter(|&time| INTERVAL < time && time < self.range[1] - INTERVAL)
            .collect();
        self.dropouts = take_tools::dropout_marks(&timed, &capture.names);
        self.take = Some(Arc::new(TakeData {
            name: capture.name,
            names: capture.names,
            timed,
            feet: capture.foot_phases,
            audio: capture.audio_passes,
        }));
    }

    pub(super) fn preview(&mut self, rig: &Rig, now: Instant) -> Option<Arc<Pose>> {
        if !self.visible {
            return None;
        }
        let clip = self.playback_clip()?;
        if let Some((start, time)) = self.playing {
            let time = time + now.saturating_duration_since(start).as_secs_f32() * self.preview_rate;
            let [a, b] = self.range;
            if self.looping && b > a {
                self.cursor = a + (time - a).rem_euclid(b - a);
            } else {
                self.cursor = time.min(b);
                if time >= b {
                    self.playing = None;
                }
            }
        }
        let mut joints = vec![None; rig.len()];
        for joint in &clip.joints {
            if let Some(&index) = rig.names.get(&joint.joint_name) {
                joints[index] = joint.rotation_at(self.cursor);
            }
        }
        Some(Arc::new(Pose::recorded(joints)))
    }

    fn export(&mut self, mode: Export, ctx: &egui::Context) {
        let clip = if matches!(mode, Export::Segments | Export::Splits) {
            self.clip.clone()
        } else {
            self.playback_clip()
        };
        let Some(clip) = clip else { return };
        let range = self.range;
        let looping = self.looping;
        let cursor = self.cursor;
        let segments = if matches!(mode, Export::Splits) {
            take_tools::split_ranges(range, &self.splits)
        } else {
            take_tools::segments(range, &self.beats, self.beats_per_clip)
        };
        let title = tr!("webcam-record-save").to_string();
        let Some(take) = self.take.clone() else { return };
        let name = take.name.clone();
        let pose_name = tr!("webcam-take-pose-name").to_string();
        let sound_title = tr!("webcam-sound-save").to_string();
        let bundle_title = tr!("webcam-take-save-pair").to_string();
        let include_sound = self.include_sound && !take.audio.is_empty();
        let direct = matches!(mode, Export::Gesture) && !self.review_gesture;
        self.spawn(ctx, move || match mode {
            Export::Segments | Export::Splits => save_segments(&clip, &segments, &name),
            Export::Upload => {
                let prepared = prepare(&clip, range, looping, name)?;
                Ok(Output::Upload(prepared))
            }
            Export::Pose => {
                let pose = still_pose(&clip, cursor);
                let prepared = prepare(&pose, [0.0, 1.0], false, pose_name)?;
                save_file(prepared, title)
            }
            Export::Save => {
                let prepared = prepare(&clip, range, looping, name)?;
                save_file(prepared, title)
            }
            Export::TakeFiles => {
                let Some(folder) = rfd::FileDialog::new().set_title(bundle_title).pick_folder() else {
                    return Ok(Output::Cancelled);
                };
                let prepared = prepare(&clip, range, looping, name.clone())?;
                let animation_path = folder.join(format!("{name}.anim"));
                let sound_path = folder.join(format!("{name} Audio.wav"));
                let pcm = soundtrack::render(&take.audio, range).ok_or(Failure::NoSound)?;
                if animation_path.exists() || sound_path.exists() {
                    return Err(Failure::Write);
                }
                write_animation_atomic(&animation_path, &prepared.bytes).map_err(|_| Failure::Write)?;
                if write_animation_atomic(&sound_path, &soundtrack::to_wav(&pcm)).is_err() {
                    let _ = std::fs::remove_file(animation_path);
                    return Err(Failure::Write);
                }
                Ok(Output::Saved(2))
            }
            Export::SoundFile | Export::SoundUpload | Export::Gesture => {
                let animation = if matches!(mode, Export::Gesture) {
                    Some(prepare(&clip, range, false, name.clone())?)
                } else {
                    None
                };
                let pcm = if !matches!(mode, Export::Gesture) || include_sound {
                    soundtrack::render(&take.audio, range).ok_or(Failure::NoSound)?
                } else {
                    Vec::new()
                };
                if matches!(mode, Export::SoundFile) {
                    let Some(mut path) = rfd::FileDialog::new()
                        .set_title(sound_title)
                        .set_file_name(format!("{name} Audio.wav"))
                        .add_filter("WAV", &["wav"])
                        .save_file()
                    else {
                        return Ok(Output::Cancelled);
                    };
                    if !path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("wav")) {
                        path.set_extension("wav");
                    }
                    write_animation_atomic(&path, &soundtrack::to_wav(&pcm)).map_err(|_| Failure::Write)?;
                    Ok(Output::Saved(1))
                } else {
                    Ok(Output::Media(PreparedMedia {
                        animation,
                        sounds: soundtrack::upload_pieces(&pcm),
                        name,
                        direct,
                    }))
                }
            }
        });
    }

    fn playback_clip(&self) -> Option<Arc<Animation>> {
        if self.looping
            && let Some(seam) = &self.seam
        {
            return Some(seam.clone());
        }
        self.clip.clone()
    }

    fn spawn(&mut self, ctx: &egui::Context, job: impl FnOnce() -> Result<Output, Failure> + Send + 'static) {
        self.failure = None;
        self.saved = 0;
        let (send, receive) = mpsc::channel();
        let ctx = ctx.clone();
        let spawned = std::thread::Builder::new().name("webcam-animation-export".into()).spawn(move || {
            let _ = send.send(job());
            ctx.request_repaint();
        });
        if spawned.is_ok() {
            self.job = Some(receive);
        } else {
            self.failure = Some(Failure::Write);
        }
    }

    fn poll(&mut self) {
        let Some(job) = &self.job else { return };
        let result = match job.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => Err(Failure::Write),
        };
        self.job = None;
        match result {
            Ok(Output::Saved(count)) => self.saved = count,
            Ok(Output::Upload(upload)) => self.upload = Some(upload),
            Ok(Output::Media(media)) => self.media = Some(media),
            Ok(Output::Cancelled) => {}
            Ok(Output::Repaired(repaired, timed)) => {
                self.clip = Some(Arc::new(repaired));
                self.repaired_frames = Some(Arc::new(timed));
                self.seam = None;
            }
            Ok(Output::Loop(range, clip)) => {
                self.set_range(range);
                self.looping = true;
                self.seam = Some(Arc::new(clip));
            }
            Ok(Output::Beats(beats)) => self.beats = beats,
            Ok(Output::Waveform(peaks)) => self.waveform = Some(peaks),
            Err(failure) => self.failure = Some(failure),
        }
    }

    fn set_range(&mut self, range: [f32; 2]) {
        self.range = range;
        self.cursor = range[0];
        self.playing = None;
        self.seam = None;
    }

    fn timeline(&mut self, ui: &mut egui::Ui, palette: &Palette, duration: f32) {
        let Some(take) = self.take.clone() else { return };
        if self.waveform.is_none() && self.job.is_none() {
            self.waveform = Some(Vec::new());
            if !take.audio.is_empty() {
                self.spawn(ui.ctx(), move || Ok(Output::Waveform(take_tools::waveform(&take.audio, duration))));
            }
        }
        ui.label(tr!("webcam-timeline-help"));
        let (rect, response) = ui.allocate_exact_size(egui::vec2(ui.available_width().max(1.0), 48.0), egui::Sense::click_and_drag());
        let x = |time: f32| rect.left() + (time / duration).clamp(0.0, 1.0) * rect.width();
        let time_at = |point: egui::Pos2| (((point.x - rect.left()) / rect.width()) * duration).clamp(0.0, duration);
        if self.job.is_none() {
            if response.drag_started()
                && let Some(point) = ui.input(|input| input.pointer.press_origin())
            {
                let nearest = self
                    .splits
                    .iter()
                    .enumerate()
                    .filter(|(_, t)| (x(**t) - point.x).abs() <= 6.0)
                    .min_by(|(_, a), (_, b)| (x(**a) - point.x).abs().total_cmp(&(x(**b) - point.x).abs()));
                self.timeline_drag = Some(if (x(self.range[0]) - point.x).abs() <= 6.0 {
                    TimelineDrag::Start
                } else if (x(self.range[1]) - point.x).abs() <= 6.0 {
                    TimelineDrag::End
                } else if let Some((index, _)) = nearest {
                    TimelineDrag::Split(index)
                } else {
                    TimelineDrag::Playhead
                });
            }
            if (response.clicked() || response.dragged() || response.drag_stopped())
                && let Some(point) = response.interact_pointer_pos()
            {
                let time = time_at(point);
                match self.timeline_drag.unwrap_or(TimelineDrag::Playhead) {
                    TimelineDrag::Start => self.set_range([time.min((self.range[1] - INTERVAL).max(0.0)), self.range[1]]),
                    TimelineDrag::End => self.set_range([self.range[0], time.max((self.range[0] + INTERVAL).min(duration))]),
                    TimelineDrag::Split(index) => {
                        let previous = index.checked_sub(1).map_or(0.0, |prev| self.splits[prev]);
                        let next = self.splits.get(index + 1).copied().unwrap_or(duration);
                        self.splits[index] = time.clamp(previous + INTERVAL, (next - INTERVAL).max(previous + INTERVAL));
                        self.scrub_take(self.splits[index]);
                    }
                    TimelineDrag::Playhead => self.scrub_take(time),
                }
            }
        }
        if !response.dragged() {
            self.timeline_drag = None;
        }

        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, ui.visuals().widgets.inactive.corner_radius, palette.field);
        let point = |t, y| egui::pos2(x(t), y);
        painter.rect_filled(
            egui::Rect::from_min_max(point(self.range[0], rect.top()), point(self.range[1], rect.bottom())),
            0.0,
            palette.violet.gamma_multiply(0.18),
        );
        if let Some(peaks) = &self.waveform {
            let step = duration / peaks.len().max(1) as f32;
            for (i, &amplitude) in peaks.iter().enumerate() {
                let time = (i as f32 + 0.5) * step;
                let height = amplitude * rect.height() * 0.44;
                let center = rect.center().y;
                if height > 0.0 {
                    painter.line_segment(
                        [point(time, center - height), point(time, center + height)],
                        egui::Stroke::new(1.0, palette.teal),
                    );
                }
            }
        }
        for (marks, color, height) in [
            (self.dropouts.as_slice(), palette.amber, 7.0),
            (self.beats.as_slice(), palette.teal, 12.0),
            (self.splits.as_slice(), palette.ink, rect.height()),
            (self.range.as_slice(), palette.violet, rect.height()),
            (&[self.cursor][..], palette.success, rect.height()),
        ] {
            for &time in marks {
                painter.line_segment(
                    [point(time, rect.bottom() - height), point(time, rect.bottom())],
                    egui::Stroke::new(1.5, color),
                );
            }
        }
        ui.add_enabled_ui(self.job.is_none(), |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(format!("{:.2} / {duration:.2} s", self.cursor));
                let near = self.splits.iter().position(|&time| (time - self.cursor).abs() <= INTERVAL);
                let can_split = self.cursor > INTERVAL && self.cursor < duration - INTERVAL && near.is_none();
                if ui.add_enabled(can_split, egui::Button::new(tr!("webcam-timeline-split"))).clicked() {
                    let index = self.splits.partition_point(|&time| time < self.cursor);
                    self.splits.insert(index, self.cursor);
                }
                if let Some(index) = near
                    && ui.button(tr!("webcam-timeline-remove")).clicked()
                {
                    self.splits.remove(index);
                }
                if !self.splits.is_empty() && ui.button(tr!("webcam-take-select-clip")).clicked() {
                    let index = self.splits.partition_point(|&time| time <= self.cursor);
                    let a = index.checked_sub(1).map_or(0.0, |prev| self.splits[prev]);
                    let b = self.splits.get(index).copied().unwrap_or(duration);
                    self.set_range([a, b]);
                    self.visible = true;
                }
                if self.range != [0.0, duration] && ui.button(tr!("webcam-take-whole")).clicked() {
                    self.set_range([0.0, duration]);
                }
                let segments = take_tools::split_ranges(self.range, &self.splits);
                if segments.len() > 1 && ui.button(tr!("webcam-timeline-export")).clicked() {
                    self.export(Export::Splits, ui.ctx());
                }
            });
        });
    }

    fn edit_tools(&mut self, ui: &mut egui::Ui, palette: &Palette, duration: f32, rig: &Arc<Rig>) {
        let Some(take) = self.take.clone() else { return };
        egui::CollapsingHeader::new(tr!("webcam-take-tools")).show(ui, |ui| {
            ui.add_enabled_ui(self.job.is_none(), |ui| {
                ui.horizontal_wrapped(|ui| {
                    if ui.button(tr!("webcam-take-repair")).clicked() {
                        self.repair(ui.ctx(), None, take.clone());
                    }
                    if ui.button(tr!("webcam-take-fix-frame")).clicked() {
                        self.repair(ui.ctx(), Some(self.cursor), take.clone());
                    }
                    if let Some((original, active)) = self.original.as_ref().zip(self.clip.as_ref())
                        && !Arc::ptr_eq(original, active)
                        && ui.button(tr!("webcam-take-original")).clicked()
                    {
                        self.clip = self.original.clone();
                        self.repaired_frames = None;
                        self.seam = None;
                    }
                });
                ui.horizontal_wrapped(|ui| {
                    let selected = self
                        .repair_joint
                        .and_then(|i| take.names.get(i))
                        .map_or(tr!("webcam-repair-all"), String::as_str);
                    egui::ComboBox::from_id_salt("webcam_repair_joint")
                        .selected_text(selected)
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut self.repair_joint, None, tr!("webcam-repair-all"));
                            for (i, name) in take.names.iter().enumerate() {
                                if take.timed.iter().any(|(_, frame)| frame.get(i).is_some_and(Option::is_some)) {
                                    ui.selectable_value(&mut self.repair_joint, Some(i), name);
                                }
                            }
                        });
                    ui.label(tr!("webcam-repair-width"));
                    ui.add(
                        egui::DragValue::new(&mut self.repair_width)
                            .range(0.08..=3.0)
                            .speed(0.02)
                            .suffix(" s"),
                    );
                    if ui.button(tr!("webcam-repair-selection")).clicked() {
                        let half = self.repair_width / 2.0;
                        let range = [self.cursor - half, self.cursor + half];
                        self.repair_range(ui.ctx(), Some(range), self.repair_joint, take.clone());
                    }
                });
                ui.horizontal_wrapped(|ui| {
                    let half = self.repair_width * 0.5;
                    let range = [(self.cursor - half).max(0.0), (self.cursor + half).min(duration)];
                    let has_video = take.audio.iter().any(|pass| pass.from < range[1] && pass.end > range[0]);
                    for (side, label) in [tr!("webcam-retrack-left"), tr!("webcam-retrack-right")].into_iter().enumerate() {
                        if ui
                            .add_enabled(has_video, egui::Button::new(label))
                            .on_disabled_hover_text(tr!("webcam-retrack-video-only"))
                            .clicked()
                        {
                            self.retrack_hand(ui.ctx(), rig.clone(), take.clone(), range, side);
                        }
                    }
                });
                ui.horizontal_wrapped(|ui| {
                    if ui.button(tr!("webcam-take-find-loop")).clicked()
                        && let Some(clip) = self.clip.clone()
                    {
                        let range = self.range;
                        let take = take.clone();
                        let rig = rig.clone();
                        self.spawn(ui.ctx(), move || {
                            let found = take_tools::find_loop(&clip, range, &take.feet).ok_or(Failure::NoLoop)?;
                            let baked = take_tools::bake_loop(&clip, found, &rig, &take.feet);
                            Ok(Output::Loop(found, baked))
                        });
                    }
                    if ui.button(tr!("webcam-take-whole")).clicked() {
                        self.set_range([0.0, duration]);
                    }
                    if ui.button(tr!("webcam-take-save-pose")).clicked() {
                        self.export(Export::Pose, ui.ctx());
                    }
                });
                ui.horizontal_wrapped(|ui| {
                    if ui
                        .add_enabled(!take.audio.is_empty(), egui::Button::new(tr!("webcam-take-audio-beats")))
                        .clicked()
                    {
                        let take = take.clone();
                        self.spawn(ui.ctx(), move || {
                            let beats = take_tools::audio_beats(&take.audio, duration);
                            if beats.len() >= 2 {
                                Ok(Output::Beats(beats))
                            } else {
                                Err(Failure::NoBeats)
                            }
                        });
                    }
                    ui.add(egui::DragValue::new(&mut self.bpm).range(40..=240).suffix(" BPM"));
                    if ui.button(tr!("webcam-take-beat-grid")).clicked() {
                        self.beats = take_tools::beat_grid(self.bpm, self.cursor, duration);
                        self.failure = None;
                    }
                });
                if !self.beats.is_empty() {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(tr!("webcam-take-beats-per-clip"));
                        ui.add(egui::DragValue::new(&mut self.beats_per_clip).range(1..=16));
                        let parts = take_tools::segments(self.range, &self.beats, self.beats_per_clip);
                        if ui
                            .add_enabled(parts.len() > 1, egui::Button::new(tr!("webcam-take-select-clip")))
                            .clicked()
                            && let Some(part) = parts.iter().find(|[a, b]| *a <= self.cursor && self.cursor <= *b)
                        {
                            self.set_range(*part);
                            self.visible = true;
                        }
                        if ui
                            .add_enabled(parts.len() > 1, egui::Button::new(tr!("webcam-take-save-clips")))
                            .clicked()
                        {
                            self.export(Export::Segments, ui.ctx());
                        }
                    });
                }
            });
            if self.job.is_some() {
                ui.spinner();
            }
            ui.horizontal_wrapped(|ui| {
                ui.colored_label(palette.amber, tr!("webcam-take-dropout-key"));
                ui.colored_label(palette.teal, tr!("webcam-take-beat-key"));
            });
        });
    }

    fn repair(&mut self, ctx: &egui::Context, at: Option<f32>, take: Arc<TakeData>) {
        self.repair_range(ctx, at.map(|center| [center - 0.065, center + 0.065]), None, take);
    }

    fn repair_range(&mut self, ctx: &egui::Context, range: Option<[f32; 2]>, joint: Option<usize>, take: Arc<TakeData>) {
        let current = self.repaired_frames.clone();
        self.spawn(ctx, move || {
            let timed = range.and(current.as_deref()).unwrap_or(&take.timed);
            let frames = if let Some([a, b]) = range {
                let joint = joint.map(|index| [index]);
                take_tools::repair_selected(
                    timed,
                    Some([a, b]),
                    joint.as_ref().map(|indices| indices.as_slice()),
                    (b - a + 0.22).max(0.35),
                )
            } else {
                take_tools::repair(timed, None)
            };
            let repaired: Vec<_> = timed.iter().zip(frames).map(|((time, _), rotations)| (*time, rotations)).collect();
            let samples = repaired
                .iter()
                .map(|(time, rotations)| Sample {
                    time: *time,
                    rotations: rotations.clone(),
                })
                .collect();
            assemble(samples, take.names.clone())
                .map(|clip| Output::Repaired(clip, repaired))
                .ok_or(Failure::Empty)
        });
    }

    fn retrack_hand(&mut self, ctx: &egui::Context, rig: Arc<Rig>, take: Arc<TakeData>, [start, end]: [f32; 2], side: usize) {
        let current = self.repaired_frames.clone();
        let Some(clip) = self.clip.clone() else { return };
        self.spawn(ctx, move || {
            let mut frames = current.as_deref().unwrap_or(&take.timed).clone();
            let animation_joints: Vec<_> = clip
                .joints
                .iter()
                .filter_map(|joint| Some((*rig.names.get(&joint.joint_name)?, joint)))
                .collect();
            let recording_joints: Vec<_> = take
                .names
                .iter()
                .enumerate()
                .filter_map(|(index, name)| Some((index, *rig.names.get(name)?)))
                .collect();
            let mut changed = false;
            for (pass_index, pass) in take.audio.iter().enumerate() {
                let pass_end = take.audio.get(pass_index + 1).map_or(pass.end, |next| pass.end.min(next.from));
                let first = frames.partition_point(|(time, _)| *time < start.max(pass.from));
                let last = frames.partition_point(|(time, _)| *time <= end.min(pass_end));
                let Some(selection) = frames.get_mut(first..last).filter(|s| !s.is_empty()) else {
                    continue;
                };
                let times: Vec<_> = selection
                    .iter()
                    .map(|(time, _)| pass.offset + (time - pass.from) * pass.rate)
                    .collect();
                let source_side = side ^ usize::from(pass.mirrored);
                let Ok(hands) = read_hand_pass(pass, source_side, &times) else {
                    continue;
                };
                for ((time, frame), hand) in selection.iter_mut().zip(hands) {
                    let Some(hand) = hand else { continue };
                    let mut observation = if source_side == 0 {
                        Frame {
                            left: Some(hand),
                            ..Frame::default()
                        }
                    } else {
                        Frame {
                            right: Some(hand),
                            ..Frame::default()
                        }
                    };
                    if pass.mirrored {
                        observation = observation.mirrored();
                    }
                    let mut base = rig.local_rot.clone();
                    for &(index, joint) in &animation_joints {
                        if let Some(rotation) = joint.rotation_at(*time) {
                            base[index] = rotation;
                        }
                    }
                    let pose = Pose::new(&observation, None, 1.0, Channels::default());
                    let rotations = pose.retrack_hand(&rig, &base, side);
                    let fade = ((*time - start).min(end - *time) / 0.08).clamp(0.0, 1.0);
                    if fade <= f32::EPSILON {
                        continue;
                    }
                    frame.resize(take.names.len(), None);
                    for &(index, joint) in &recording_joints {
                        if let Some(rotation) = rotations[joint] {
                            frame[index] = Some(frame[index].map_or(rotation, |old| old.slerp(rotation, fade)));
                            changed = true;
                        }
                    }
                }
            }
            if !changed {
                return Err(Failure::NoHand);
            }
            let samples = frames
                .iter()
                .map(|(time, rotations)| Sample {
                    time: *time,
                    rotations: rotations.clone(),
                })
                .collect();
            assemble(samples, take.names.clone())
                .map(|clip| Output::Repaired(clip, frames))
                .ok_or(Failure::Empty)
        });
    }

    /// Returns true when the take has taken over the local avatar preview.
    pub(super) fn ui(
        &mut self,
        ui: &mut egui::Ui,
        palette: &Palette,
        can_record: bool,
        source: impl FnOnce() -> String,
        rig: &Arc<Rig>,
    ) -> bool {
        self.poll();
        let now = Instant::now();
        let was_visible = self.visible;
        widgets::section_header(ui, palette, "video-filled", &tr!("webcam-record-title"));
        ui.horizontal_wrapped(|ui| {
            if let Some(active) = &mut self.active {
                let mark = active.paused.unwrap_or(now);
                let status = if active.paused.is_some() {
                    tr!(
                        "webcam-record-paused",
                        seconds = format!("{:.1}", mark.saturating_duration_since(active.starts).as_secs_f32())
                    )
                } else if now < active.starts {
                    tr!(
                        "webcam-record-counting",
                        seconds = format!("{:.0}", active.starts.duration_since(now).as_secs_f32().ceil())
                    )
                } else {
                    tr!(
                        "webcam-record-running",
                        seconds = format!("{:.1}", now.duration_since(active.starts).as_secs_f32().min(active.limit))
                    )
                };
                let stop = ui.button(tr!("webcam-record-stop")).clicked();
                if let Some(paused) = active.paused {
                    if ui.add_enabled(can_record, egui::Button::new(tr!("webcam-record-resume"))).clicked() {
                        let cut = paused.saturating_duration_since(active.starts).as_secs_f32();
                        if !active.samples.is_empty() && cut >= active.splits.last().copied().unwrap_or(0.0) + INTERVAL {
                            active.splits.push(cut);
                        }
                        active.starts += now.saturating_duration_since(paused);
                        active.audio_from = now.max(active.starts);
                        active.paused = None;
                    }
                } else if ui.button(tr!("webcam-record-hold")).clicked() {
                    self.pause(now);
                }
                ui.colored_label(palette.danger, status);
                ui.ctx().request_repaint_after(Duration::from_millis(33));
                if stop {
                    self.finish();
                }
            } else {
                if ui
                    .add_enabled(can_record && self.job.is_none(), egui::Button::new(tr!("webcam-record-start")))
                    .clicked()
                {
                    self.start(now, &source());
                }
                widgets::checkbox(ui, palette, &mut self.countdown, tr!("webcam-record-countdown"));
                ui.label(tr!("webcam-record-limit"));
                ui.add(egui::DragValue::new(&mut self.limit).range(1..=60).suffix(" s"));
            }
        });
        if self.active.is_none()
            && let Some(clip) = &self.clip
        {
            let duration = clip.duration;
            ui.add_enabled_ui(self.job.is_none(), |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.label(tr!("webcam-record-in"));
                    let before = ui
                        .add(
                            egui::DragValue::new(&mut self.range[0])
                                .range(0.0..=(duration - INTERVAL).max(0.0))
                                .speed(0.03)
                                .fixed_decimals(2),
                        )
                        .changed();
                    if before {
                        self.range[1] = self.range[1].max(self.range[0] + INTERVAL).min(duration);
                    }
                    ui.label(tr!("webcam-record-out"));
                    let earliest_end = (self.range[0] + INTERVAL).min(duration);
                    let after = ui
                        .add(
                            egui::DragValue::new(&mut self.range[1])
                                .range(earliest_end..=duration)
                                .speed(0.03)
                                .fixed_decimals(2),
                        )
                        .changed();
                    if before || after {
                        self.playing = None;
                        self.seam = None;
                        self.cursor = self.cursor.clamp(self.range[0], self.range[1]);
                    }
                })
            });
            self.timeline(ui, palette, duration);
            ui.horizontal_wrapped(|ui| {
                if self.playing.is_some() {
                    if ui.button(tr!("webcam-record-pause")).clicked() {
                        self.playing = None;
                    }
                    ui.ctx().request_repaint();
                } else if ui.button(tr!("webcam-record-preview")).clicked() {
                    if self.cursor >= self.range[1] {
                        self.cursor = self.range[0];
                    }
                    self.visible = true;
                    self.playing = Some((now, self.cursor));
                }
                if self.visible && ui.button(tr!("webcam-record-close-preview")).clicked() {
                    self.hide_preview();
                }
                widgets::checkbox(ui, palette, &mut self.looping, tr!("webcam-record-loop"));
            });
            ui.horizontal_wrapped(|ui| {
                if ui.small_button("−1 s").clicked() {
                    self.scrub_take(self.cursor - 1.0);
                }
                if ui.small_button("+1 s").clicked() {
                    self.scrub_take(self.cursor + 1.0);
                }
                let fps = 30.0;
                if ui.small_button("‹").on_hover_text(tr!("webcam-preview-prev-frame")).clicked() {
                    self.scrub_take(self.cursor - 1.0 / fps);
                }
                if ui.small_button("›").on_hover_text(tr!("webcam-preview-next-frame")).clicked() {
                    self.scrub_take(self.cursor + 1.0 / fps);
                }
                if speed_picker(ui, "webcam_take_speed", &mut self.preview_rate) && self.playing.is_some() {
                    self.playing = Some((now, self.cursor));
                }
            });
            let has_sound = self.take.as_ref().is_some_and(|take| !take.audio.is_empty());
            widgets::divider(ui, palette);
            ui.add_enabled_ui(self.job.is_none(), |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.add_enabled_ui(has_sound, |ui| {
                        widgets::checkbox(ui, palette, &mut self.include_sound, tr!("webcam-gesture-with-sound"));
                    });
                    widgets::checkbox(ui, palette, &mut self.review_gesture, tr!("webcam-gesture-review-first"));
                });
                egui::CollapsingHeader::new(tr!("webcam-take-export")).show(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        if ui.button(tr!("webcam-record-review-upload")).clicked() {
                            self.hide_preview();
                            self.export(Export::Upload, ui.ctx());
                        }
                        if ui.button(tr!("webcam-record-save")).clicked() {
                            self.export(Export::Save, ui.ctx());
                        }
                    });
                    ui.horizontal_wrapped(|ui| {
                        if ui
                            .add_enabled(has_sound, egui::Button::new(tr!("webcam-sound-review-upload")))
                            .clicked()
                        {
                            self.export(Export::SoundUpload, ui.ctx());
                        }
                        if ui.add_enabled(has_sound, egui::Button::new(tr!("webcam-sound-save"))).clicked() {
                            self.export(Export::SoundFile, ui.ctx());
                        }
                        if ui.add_enabled(has_sound, egui::Button::new(tr!("webcam-take-save-pair"))).clicked() {
                            self.export(Export::TakeFiles, ui.ctx());
                        }
                    });
                });
            });
            self.edit_tools(ui, palette, duration, rig);
        }
        if self.saved == 1 {
            ui.colored_label(palette.success, tr!("webcam-export-saved-file"));
        } else if self.saved > 1 {
            ui.colored_label(palette.success, tr!("webcam-export-saved-files", count = self.saved.to_string()));
        }
        if let Some(failure) = self.failure {
            ui.colored_label(
                palette.danger,
                match failure {
                    Failure::Empty => tr!("webcam-record-empty"),
                    Failure::Size => tr!("webcam-record-size"),
                    Failure::Write => tr!("webcam-record-error"),
                    Failure::NoLoop => tr!("webcam-take-no-loop"),
                    Failure::NoBeats => tr!("webcam-take-no-beats"),
                    Failure::NoSound => tr!("webcam-sound-unavailable"),
                    Failure::NoHand => tr!("webcam-retrack-missing"),
                },
            );
        }
        !was_visible && self.visible
    }

    pub(super) fn gesture_available(&self) -> bool {
        self.active.is_none() && self.clip.is_some()
    }

    pub(super) fn gesture_button(&mut self, ui: &mut egui::Ui, palette: &Palette) {
        ui.add_enabled_ui(self.job.is_none(), |ui| {
            let label = tr!(
                "webcam-gesture-one-click",
                seconds = format!("{:.1}", self.range[1] - self.range[0])
            );
            if widgets::primary_button_sized(ui, palette, &label, egui::vec2(ui.available_width(), 32.0))
                .on_hover_text(tr!("webcam-gesture-one-click-tip"))
                .clicked()
            {
                self.hide_preview();
                self.export(Export::Gesture, ui.ctx());
            }
        });
    }

    fn scrub_take(&mut self, time: f32) {
        self.cursor = time.clamp(self.range[0], self.range[1]);
        self.visible = true;
        self.playing = None;
    }
}

pub(super) fn speed_picker(ui: &mut egui::Ui, id: &str, rate: &mut f32) -> bool {
    let before = *rate;
    egui::ComboBox::from_id_salt(id)
        .width(65.0)
        .selected_text(format!("{rate}×"))
        .show_ui(ui, |ui| {
            for speed in [0.5, 0.75, 1.0, 1.25, 1.5, 2.0] {
                ui.selectable_value(rate, speed, format!("{speed}×"));
            }
        });
    before != *rate
}

type HandObservation = Option<[[f32; 3]; 21]>;

fn read_hand_pass(pass: &AudioPass, side: usize, times: &[f32]) -> Result<Vec<HandObservation>, Failure> {
    let mut child = companion()
        .arg("--retrack-hand")
        .arg(&pass.source.path)
        .arg(["left", "right"][side])
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|_| Failure::NoHand)?;
    let mut input = child.stdin.take().expect("piped hand repair input");
    let send = send_script(&mut input).and_then(|()| {
        serde_json::to_writer(&mut input, times).map_err(std::io::Error::other)?;
        input.write_all(b"\n")
    });
    drop(input);
    if send.is_err() {
        let _ = child.kill();
        let _ = child.wait();
        return Err(Failure::NoHand);
    }
    let output = child.wait_with_output().map_err(|_| Failure::NoHand)?;
    let hands = serde_json::Deserializer::from_slice(&output.stdout)
        .into_iter()
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| Failure::NoHand)?;
    if !output.status.success() || hands.len() != times.len() {
        return Err(Failure::NoHand);
    }
    Ok(hands)
}

fn assemble(samples: Vec<Sample>, names: Vec<String>) -> Option<Animation> {
    let duration = samples.last()?.time;
    if samples.len() < 2 || duration < INTERVAL {
        return None;
    }
    let mut tracks = vec![Vec::<RotKey>::new(); names.len()];
    for sample in samples {
        for (track, rotation) in tracks.iter_mut().zip(sample.rotations) {
            let Some(rotation) = rotation.filter(|q| q.is_finite()) else {
                continue;
            };
            if let Some(last) = track.last().filter(|last| sample.time - last.time > 0.24) {
                track.push(RotKey {
                    time: sample.time - 0.11,
                    rotation: last.rotation,
                });
            }
            track.push(RotKey {
                time: sample.time,
                rotation,
            });
        }
    }
    let joints: Vec<_> = names
        .into_iter()
        .zip(tracks)
        .filter_map(|(joint_name, rot_keys)| {
            (joint_name.starts_with('m') && !rot_keys.is_empty()).then_some(JointMotion {
                joint_name,
                priority: -1,
                rot_keys,
                pos_keys: Vec::new(),
            })
        })
        .collect();
    (!joints.is_empty()).then_some(Animation {
        base_priority: 4,
        duration,
        emote_name: String::new(),
        loop_in: 0.0,
        loop_out: duration,
        looping: false,
        ends_after: None,
        ease_in: 0.10,
        ease_out: 0.15,
        hand_pose: 0,
        joints,
    })
}

fn simplify(keys: &[RotKey], degrees: f32) -> Vec<RotKey> {
    let mut keep = vec![false; keys.len()];
    let last = keys.len() - 1;
    keep[0] = true;
    keep[last] = true;
    let min_dot = (degrees.to_radians() * 0.5).cos();
    let mut segments = vec![(0, last)];
    while let Some((a, b)) = segments.pop() {
        let span = keys[b].time - keys[a].time;
        let mut split = None;
        let mut worst = min_dot;
        for i in a + 1..b {
            let u = (keys[i].time - keys[a].time) / span;
            let q = keys[a].rotation.slerp(keys[b].rotation, u);
            let similarity = q.dot(keys[i].rotation).abs();
            if similarity < worst {
                worst = similarity;
                split = Some(i);
            }
        }
        if let Some(i) = split {
            keep[i] = true;
            segments.extend([(a, i), (i, b)]);
        }
    }
    keys.iter().zip(keep).filter_map(|(key, keep)| keep.then(|| key.clone())).collect()
}

fn still_pose(source: &Animation, time: f32) -> Animation {
    let mut pose = source.clone();
    pose.duration = 1.0;
    pose.loop_out = 1.0;
    pose.looping = false;
    for joint in &mut pose.joints {
        if let Some(rotation) = joint.rotation_at(time) {
            joint.rot_keys = [0.0, 1.0].map(|time| RotKey { time, rotation }).into();
        }
    }
    pose
}

fn save_file(prepared: PreparedAnimation, title: String) -> Result<Output, Failure> {
    let Some(mut path) = rfd::FileDialog::new()
        .set_title(title)
        .set_file_name(format!("{}.anim", prepared.name))
        .add_filter("SL .anim", &["anim"])
        .save_file()
    else {
        return Ok(Output::Cancelled);
    };
    if !path
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("anim"))
    {
        path.set_extension("anim");
    }
    write_animation_atomic(&path, &prepared.bytes).map_err(|error| {
        log::warn!("could not save recorded webcam animation: {error}");
        Failure::Write
    })?;
    Ok(Output::Saved(1))
}

fn save_segments(clip: &Animation, segments: &[[f32; 2]], name: &str) -> Result<Output, Failure> {
    let Some(folder) = rfd::FileDialog::new().pick_folder() else {
        return Ok(Output::Cancelled);
    };
    let batch = uuid::Uuid::new_v4().simple().to_string();
    let files: Result<Vec<_>, _> = segments
        .iter()
        .enumerate()
        .map(|(index, &range)| {
            let name = format!("{name} {} {:02}", &batch[..8], index + 1);
            let prepared = prepare(clip, range, false, name)?;
            Ok((folder.join(format!("{}.anim", prepared.name)), prepared.bytes))
        })
        .collect();
    let files = files?;
    for (index, (path, bytes)) in files.iter().enumerate() {
        if let Err(error) = write_animation_atomic(path, bytes) {
            log::warn!("could not save beat clips: {error}");
            for (written, _) in &files[..index] {
                let _ = std::fs::remove_file(written);
            }
            return Err(Failure::Write);
        }
    }
    Ok(Output::Saved(files.len()))
}

fn prepare(source: &Animation, [start, end]: [f32; 2], looping: bool, name: String) -> Result<PreparedAnimation, Failure> {
    let mut clip = source.clone();
    clip.duration = end - start;
    clip.loop_out = clip.duration;
    clip.looping = looping;
    for (track, src) in clip.joints.iter_mut().zip(&source.joints) {
        let first = src.rotation_at(start).ok_or(Failure::Empty)?;
        let last = src.rotation_at(end).ok_or(Failure::Empty)?;
        track.rot_keys.clear();
        track.rot_keys.push(RotKey {
            time: 0.0,
            rotation: first,
        });
        track
            .rot_keys
            .extend(src.rot_keys.iter().filter(|k| k.time > start && k.time < end).map(|k| RotKey {
                time: k.time - start,
                rotation: k.rotation,
            }));
        track.rot_keys.push(RotKey {
            time: clip.duration,
            rotation: last,
        });
    }
    let tracks = clip.joints.clone();
    for degrees in [0.75, 1.25, 2.0] {
        for (joint, raw) in clip.joints.iter_mut().zip(&tracks) {
            joint.rot_keys = simplify(&raw.rot_keys, degrees);
        }
        let bytes = aurora_assets::import_animation::encode(&clip).map_err(|_| Failure::Write)?;
        if bytes.len() <= ASSET_BYTES {
            let animation = aurora_assets::anim::parse_animation_upload(&bytes).map_err(|_| Failure::Write)?;
            return Ok(PreparedAnimation {
                name,
                bytes: Arc::new(bytes),
                animation: Arc::new(animation),
            });
        }
    }
    Err(Failure::Size)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trimmed_native_take_preserves_authored_bones_and_time() {
        let times = [0.0, 0.1, 0.2, 0.3, 0.4];
        let source = assemble(
            times
                .into_iter()
                .map(|time| Sample {
                    time,
                    rotations: vec![Some(Quat::from_rotation_y(time * 2.0)), None, Some(Quat::IDENTITY)],
                })
                .collect(),
            vec!["mChest".into(), "mWristLeft".into(), "PELVIS".into()],
        )
        .unwrap();
        let out = prepare(&source, [0.1, 0.35], true, "Take".into()).unwrap();
        let clip = &out.animation;
        assert!((clip.duration - 0.25).abs() < 1.0e-6);
        assert_eq!(clip.base_priority, 4);
        assert!(clip.looping);
        assert_eq!(clip.joints.len(), 1);
        let track = &clip.joints[0];
        assert_eq!(track.joint_name, "mChest");
        assert_eq!(track.rot_keys.len(), 2);
        assert!(track.pos_keys.is_empty());
        let actual = track.rotation_at(0.125).unwrap();
        assert!(actual.dot(Quat::from_rotation_y(0.45)).abs() > 0.99999);
    }

    #[test]
    fn simplification_keeps_a_fast_gesture_between_matching_endpoints() {
        let keys = [0.0, 0.75, 0.0]
            .into_iter()
            .enumerate()
            .map(|(i, r)| RotKey {
                time: i as f32 * INTERVAL,
                rotation: Quat::from_rotation_z(r),
            })
            .collect::<Vec<_>>();
        assert_eq!(simplify(&keys, 0.75), keys);
    }
}

#[cfg(test)]
mod take_names {
    use super::*;

    #[test]
    fn generated_names_reserve_inventory_space_for_sound_segments() {
        let stamp = "261126-130500";
        let id = uuid::Uuid::from_u128(0x1234567890aabbcc);
        let base = take_name(
            "Very long source | with ⚠ é and controls
",
            stamp,
            id,
        );
        let sound = format!("{base} Audio 003");
        assert!(sound.len() <= INVENTORY_NAME_BYTES);
        assert!(sound.bytes().all(|b| matches!(b, b' '..=b'~') && b != b'|'));
        assert!(base.starts_with("Webcam Tracking Very long"));
        assert!(base.contains(stamp));
        assert_ne!(
            base,
            take_name("Very long source | with ⚠ é and controls", stamp, uuid::Uuid::new_v4())
        );
        assert!(take_name("舞踊🎵", stamp, id).starts_with("Webcam Tracking Source "));
    }

    #[test]
    fn links_use_public_ids_and_never_copy_other_query_fields_or_credentials() {
        assert_eq!(
            link_label("https://www.youtube.com/watch?v=abc123_def&auth=secret"),
            "YouTube abc123"
        );
        assert_eq!(link_label("https://vm.tiktok.com/abc1234/"), "TikTok abc123");
        assert_eq!(
            link_label("https://username:password@example.com/signed/video.mp4?token=secret"),
            "example.com"
        );
    }
}
