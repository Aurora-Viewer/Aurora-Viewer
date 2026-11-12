//! What the startup splash shows: the step of the start, its share of the
//! bar, its line of text, and the lines the viewer sends the splash process.
//! Pure, so the tests run it on every system.

/// The steps of a start, in order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// The process started.
    Starting,
    /// Settings, caches, interface state (`App::new`).
    Settings,
    /// Vulkan (Metal) instance, adapters, device.
    Gpu,
    /// The renderer's pipelines: `done` of an expected `total`.
    Shaders { done: u32, total: u32 },
    /// The renderer is ready; the interface's first frame is being made.
    Interface,
    /// The window showed its first frame.
    Ready,
    /// An update, shown by the updater's helper (`aurora_updater::install`)
    /// between the old viewer's window and the new one's splash.
    Update(UpdateStep),
}

/// The steps of an update's switch, in order (`HelperStage`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UpdateStep {
    Closing,
    Installing,
    ClearingCache,
    Launching,
}

impl UpdateStep {
    pub fn of(stage: aurora_updater::install::HelperStage) -> Option<UpdateStep> {
        use aurora_updater::install::HelperStage;
        Some(match stage {
            HelperStage::Closing => UpdateStep::Closing,
            HelperStage::Installing => UpdateStep::Installing,
            HelperStage::ClearingCache => UpdateStep::ClearingCache,
            HelperStage::Launching => UpdateStep::Launching,
            HelperStage::Handed => return None,
        })
    }

    fn name(self) -> &'static str {
        match self {
            UpdateStep::Closing => "update-closing",
            UpdateStep::Installing => "update-installing",
            UpdateStep::ClearingCache => "update-cache",
            UpdateStep::Launching => "update-launching",
        }
    }
}

/// Where the bar stands at each step; the pipelines take most of it, since
/// they are most of a slow start.
const STARTING: f32 = 0.03;
const SETTINGS: f32 = 0.08;
const GPU: f32 = 0.14;
const SHADERS_END: f32 = 0.90;
const INTERFACE: f32 = 0.95;

impl Step {
    /// The share of the bar at this step, 0 to 1.
    pub fn fraction(self) -> f32 {
        match self {
            Step::Starting => STARTING,
            Step::Settings => SETTINGS,
            Step::Gpu => GPU,
            Step::Shaders { done, total } => {
                let part = if total == 0 { 0.0 } else { (done.min(total) as f32) / total as f32 };
                GPU + (SHADERS_END - GPU) * part
            }
            Step::Interface => INTERFACE,
            Step::Ready => 1.0,
            // the switch's own bar; the new viewer's splash starts its own
            Step::Update(step) => match step {
                UpdateStep::Closing => 0.15,
                UpdateStep::Installing => 0.40,
                UpdateStep::ClearingCache => 0.60,
                UpdateStep::Launching => 0.85,
            },
        }
    }

    /// The step's line under the bar.
    pub fn status(self) -> String {
        self.status_in(aurora_i18n::language())
    }

    pub fn status_in(self, language: &str) -> String {
        match self {
            Step::Starting => aurora_i18n::tr_in!(language, "splash-starting").to_owned(),
            Step::Settings => aurora_i18n::tr_in!(language, "splash-settings").to_owned(),
            Step::Gpu => aurora_i18n::tr_in!(language, "splash-gpu").to_owned(),
            Step::Shaders { done, total } => aurora_i18n::tr_in!(
                language,
                "splash-shaders",
                done = aurora_i18n::integer(i64::from(done.min(total))),
                total = aurora_i18n::integer(i64::from(total))
            ),
            Step::Interface => aurora_i18n::tr_in!(language, "splash-interface").to_owned(),
            Step::Ready => aurora_i18n::tr_in!(language, "splash-ready").to_owned(),
            Step::Update(UpdateStep::Closing) => aurora_i18n::tr_in!(language, "splash-update-closing").to_owned(),
            Step::Update(UpdateStep::Installing) => aurora_i18n::tr_in!(language, "splash-update-installing").to_owned(),
            Step::Update(UpdateStep::ClearingCache) => aurora_i18n::tr_in!(language, "splash-update-cache").to_owned(),
            Step::Update(UpdateStep::Launching) => aurora_i18n::tr_in!(language, "splash-update-launching").to_owned(),
        }
    }

    /// The line sent to the splash process.
    pub fn line(self) -> String {
        match self {
            Step::Starting => "step starting".into(),
            Step::Settings => "step settings".into(),
            Step::Gpu => "step gpu".into(),
            Step::Shaders { done, total } => format!("step shaders {done} {total}"),
            Step::Interface => "step interface".into(),
            Step::Ready => "step ready".into(),
            Step::Update(step) => format!("step {}", step.name()),
        }
    }

    /// A step as `AURORA_DEMO_SPLASH` or a line of the viewer names it
    /// (`shaders:23/61` or `shaders 23 61`).
    pub fn parse(text: &str) -> Option<Step> {
        let mut words = text.trim().split([' ', ':', '/']).filter(|w| !w.is_empty());
        let step = match words.next()? {
            "starting" => Step::Starting,
            "settings" => Step::Settings,
            "gpu" => Step::Gpu,
            "shaders" => {
                let done = words.next()?.parse().ok()?;
                let total = words.next()?.parse().ok()?;
                Step::Shaders { done, total }
            }
            "interface" => Step::Interface,
            "ready" => Step::Ready,
            "update-closing" => Step::Update(UpdateStep::Closing),
            "update-installing" => Step::Update(UpdateStep::Installing),
            "update-cache" => Step::Update(UpdateStep::ClearingCache),
            "update-launching" => Step::Update(UpdateStep::Launching),
            _ => return None,
        };
        words.next().is_none().then_some(step)
    }
}

/// What the viewer tells the splash process, one line each.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Message {
    Step(Step),
    /// The version under the title, when it is not this executable's: the
    /// one an update installs (the helper runs the old executable).
    Version(String),
    /// The viewer's window shows: close.
    Done,
}

/// Longest version line taken (`0.9.8-beta.20261112.1` is far shorter).
const MAX_VERSION: usize = 64;

impl Message {
    pub fn line(&self) -> String {
        match self {
            Message::Step(step) => step.line(),
            Message::Version(version) => format!("version {version}"),
            Message::Done => "done".into(),
        }
    }

    pub fn parse(line: &str) -> Option<Message> {
        match line.trim() {
            "done" => Some(Message::Done),
            other => {
                if let Some(version) = other.strip_prefix("version ") {
                    let version = version.trim();
                    let plain = version.chars().all(|c| c.is_ascii_alphanumeric() || ".-+".contains(c));
                    return (!version.is_empty() && version.len() <= MAX_VERSION && plain)
                        .then(|| Message::Version(version.to_owned()));
                }
                Step::parse(other.strip_prefix("step ")?).map(Message::Step)
            }
        }
    }
}

/// The bar only moves forward: a step met again (another pipeline total)
/// never takes it back.
#[derive(Clone, Copy, Debug, Default)]
pub struct Bar {
    shown: f32,
}

impl Bar {
    pub fn advance(&mut self, step: Step) -> f32 {
        self.shown = self.shown.max(step.fraction());
        self.shown
    }
}

/// The step of the start, from the viewer's own phase and the renderer's
/// progress (`aurora_render::start_progress`).
pub fn step_of(viewer: ViewerPhase, render: aurora_render::start_progress::Snapshot) -> Step {
    use aurora_render::start_progress::Phase;
    if viewer == ViewerPhase::Shown {
        return Step::Ready;
    }
    match render.phase {
        Phase::Ready => Step::Interface,
        Phase::Pipelines => Step::Shaders {
            done: render.pipelines,
            total: render.expected,
        },
        Phase::GpuDevice => Step::Gpu,
        Phase::Starting if viewer == ViewerPhase::Settings => Step::Settings,
        Phase::Starting => Step::Starting,
    }
}

/// The viewer's side of the start.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum ViewerPhase {
    Starting = 0,
    Settings = 1,
    Shown = 2,
}

impl ViewerPhase {
    pub fn from_u8(v: u8) -> ViewerPhase {
        match v {
            1 => ViewerPhase::Settings,
            2 => ViewerPhase::Shown,
            _ => ViewerPhase::Starting,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aurora_render::start_progress::{Phase, Snapshot};

    #[test]
    fn the_bar_follows_the_start_and_never_goes_back() {
        let steps = [
            Step::Starting,
            Step::Settings,
            Step::Gpu,
            Step::Shaders { done: 0, total: 140 },
            Step::Shaders { done: 70, total: 140 },
            Step::Shaders { done: 140, total: 140 },
            Step::Interface,
            Step::Ready,
        ];
        let fractions: Vec<f32> = steps.iter().map(|s| s.fraction()).collect();
        assert!(fractions.windows(2).all(|w| w[0] <= w[1]), "{fractions:?}");
        assert_eq!(Step::Ready.fraction(), 1.0);
        assert!((Step::Shaders { done: 70, total: 140 }.fraction() - 0.52).abs() < 1e-6);
        // more pipelines than expected, or none expected: no overflow
        assert_eq!(Step::Shaders { done: 200, total: 140 }.fraction(), SHADERS_END);
        assert_eq!(Step::Shaders { done: 3, total: 0 }.fraction(), GPU);
        let mut bar = Bar::default();
        assert_eq!(
            bar.advance(Step::Shaders { done: 100, total: 140 }),
            Step::Shaders { done: 100, total: 140 }.fraction()
        );
        // the total grew (this start makes more pipelines than the last)
        let held = bar.advance(Step::Shaders { done: 101, total: 300 });
        assert_eq!(held, Step::Shaders { done: 100, total: 140 }.fraction());
        assert_eq!(bar.advance(Step::Interface), INTERFACE);
    }

    #[test]
    fn an_update_moves_forward_and_hands_over() {
        use aurora_updater::install::HelperStage;
        let steps: Vec<Step> = [
            HelperStage::Closing,
            HelperStage::Installing,
            HelperStage::ClearingCache,
            HelperStage::Launching,
        ]
        .into_iter()
        .map(|s| Step::Update(UpdateStep::of(s).expect("a step of the switch")))
        .collect();
        let fractions: Vec<f32> = steps.iter().map(|s| s.fraction()).collect();
        assert!(fractions.windows(2).all(|w| w[0] < w[1]), "{fractions:?}");
        assert!(fractions.iter().all(|f| *f > 0.0 && *f < 1.0));
        // handed to the new viewer: no step, the window closes
        assert_eq!(UpdateStep::of(HelperStage::Handed), None);
    }

    #[test]
    fn the_viewer_and_the_renderer_make_the_step() {
        let render = |phase, pipelines, expected| Snapshot {
            phase,
            pipelines,
            expected,
        };
        assert_eq!(step_of(ViewerPhase::Starting, render(Phase::Starting, 0, 140)), Step::Starting);
        assert_eq!(step_of(ViewerPhase::Settings, render(Phase::Starting, 0, 140)), Step::Settings);
        assert_eq!(step_of(ViewerPhase::Settings, render(Phase::GpuDevice, 0, 140)), Step::Gpu);
        assert_eq!(
            step_of(ViewerPhase::Settings, render(Phase::Pipelines, 23, 61)),
            Step::Shaders { done: 23, total: 61 }
        );
        assert_eq!(step_of(ViewerPhase::Settings, render(Phase::Ready, 141, 141)), Step::Interface);
        assert_eq!(step_of(ViewerPhase::Shown, render(Phase::Ready, 141, 141)), Step::Ready);
    }

    #[test]
    fn lines_read_back_as_sent() {
        for message in [
            Message::Step(Step::Starting),
            Message::Step(Step::Settings),
            Message::Step(Step::Gpu),
            Message::Step(Step::Shaders { done: 23, total: 61 }),
            Message::Step(Step::Interface),
            Message::Step(Step::Ready),
            Message::Step(Step::Update(UpdateStep::Closing)),
            Message::Step(Step::Update(UpdateStep::Installing)),
            Message::Step(Step::Update(UpdateStep::ClearingCache)),
            Message::Step(Step::Update(UpdateStep::Launching)),
            Message::Version("0.9.8-beta.20261112.1".into()),
            Message::Done,
        ] {
            assert_eq!(Message::parse(&message.line()), Some(message));
        }
        // a version line is a version, never text to show as it comes
        for bad in ["version ", "version a b", "version <b>", &format!("version {}", "9".repeat(65))] {
            assert_eq!(Message::parse(bad), None, "{bad:?}");
        }
        assert_eq!(Step::parse("shaders:23/61"), Some(Step::Shaders { done: 23, total: 61 }));
        for bad in [
            "",
            "step",
            "step shaders",
            "step shaders 1",
            "step shaders a b",
            "step gpu extra",
            "hello",
            "step nope",
        ] {
            assert_eq!(Message::parse(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn every_step_has_its_text_in_every_language() {
        for language in ["en", "fr", "ja"] {
            for step in [
                Step::Starting,
                Step::Settings,
                Step::Gpu,
                Step::Interface,
                Step::Ready,
                Step::Update(UpdateStep::Closing),
                Step::Update(UpdateStep::Installing),
                Step::Update(UpdateStep::ClearingCache),
                Step::Update(UpdateStep::Launching),
            ] {
                let text = step.status_in(language);
                assert!(!text.is_empty() && !text.starts_with("splash-"), "{language} {step:?}: {text}");
            }
            let shaders = Step::Shaders { done: 23, total: 61 }.status_in(language);
            assert!(shaders.contains("23") && shaders.contains("61"), "{language}: {shaders}");
        }
        assert_eq!(Step::Gpu.status_in("en"), "Preparing the graphics card…");
        assert_eq!(Step::Shaders { done: 23, total: 61 }.status_in("en"), "Compiling shaders (23/61)…");
        assert_ne!(Step::Gpu.status_in("fr"), Step::Gpu.status_in("en"));
    }
}
