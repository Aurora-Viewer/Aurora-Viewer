//! Which GPU the renderer opens (#894). wgpu's `request_adapter` with
//! `PowerPreference::HighPerformance` already puts a discrete GPU first, but
//! only among the adapters Vulkan shows and that can present to the window,
//! and it says nothing of the others. Here every adapter is listed in the
//! log with its type, memory and whether it can present, then tried in an
//! order of our own: the user's choice (`AURORA_GPU`, Preferences ›
//! Graphics), then discrete before integrated before the rest, the larger
//! video memory first. An adapter that fails to open hands over to the next
//! instead of ending the session.
//!
//! On hybrid PCs NVIDIA's implicit `VK_LAYER_NV_optimus` layer can hide the
//! discrete GPU from Vulkan altogether, following the NVIDIA Control
//! Panel's "Preferred graphics processor" for the program; Windows ›
//! Graphics settings do not reach it. When only an integrated GPU is seen,
//! the log says so (`hint`). The Legacy Viewer, on OpenGL, exports
//! `NvOptimusEnablement` / `AmdPowerXpressRequestHighPerformance` (llgl.cpp);
//! the viewer exports them too (aurora-viewer's main.rs), for the drivers
//! that read them.

/// The kind of an adapter, in the order the automatic choice prefers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    Discrete,
    Integrated,
    Other,
    Virtual,
    Cpu,
}

impl Kind {
    pub fn from_wgpu(t: wgpu::DeviceType) -> Kind {
        match t {
            wgpu::DeviceType::DiscreteGpu => Kind::Discrete,
            wgpu::DeviceType::IntegratedGpu => Kind::Integrated,
            wgpu::DeviceType::Other => Kind::Other,
            wgpu::DeviceType::VirtualGpu => Kind::Virtual,
            wgpu::DeviceType::Cpu => Kind::Cpu,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Kind::Discrete => "discrete",
            Kind::Integrated => "integrated",
            Kind::Other => "other",
            Kind::Virtual => "virtual",
            Kind::Cpu => "software",
        }
    }
}

/// One adapter Vulkan shows.
#[derive(Debug, Clone)]
pub struct Candidate {
    pub name: String,
    pub kind: Kind,
    /// Device-local memory (MB), when it could be read.
    pub vram_mb: Option<u64>,
    /// It can present to the viewer's window.
    pub surface: bool,
}

/// The adapter asked for: none (automatic), its number in the log's list,
/// or a part of its name (any case).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Wanted {
    Auto,
    Index(usize),
    Name(String),
}

impl Wanted {
    /// `AURORA_GPU` or the setting: empty or `auto` for the automatic choice.
    pub fn parse(value: &str) -> Wanted {
        let v = value.trim();
        if v.is_empty() || v.eq_ignore_ascii_case("auto") {
            Wanted::Auto
        } else if let Ok(i) = v.parse::<usize>() {
            Wanted::Index(i)
        } else {
            Wanted::Name(v.to_lowercase())
        }
    }

    fn find(&self, candidates: &[Candidate]) -> Option<usize> {
        match self {
            Wanted::Auto => None,
            Wanted::Index(i) => (*i < candidates.len()).then_some(*i),
            Wanted::Name(n) => candidates.iter().position(|c| c.name.to_lowercase().contains(n.as_str())),
        }
    }
}

/// Why the first adapter of a plan comes first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    /// The one asked for.
    Wanted,
    /// One was asked for but is missing or cannot present: automatic.
    WantedMissing,
    Automatic,
}

/// The adapters to try, in order (indices into the candidates), and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    pub order: Vec<usize>,
    pub reason: Reason,
}

/// The order to try the adapters in. Those that cannot present to the
/// window are left out: wgpu could not draw to it with them.
pub fn plan(candidates: &[Candidate], wanted: &Wanted) -> Plan {
    let mut auto: Vec<usize> = (0..candidates.len()).filter(|&i| candidates[i].surface).collect();
    // stable: the loader's order breaks the remaining ties
    auto.sort_by_key(|&i| (candidates[i].kind, std::cmp::Reverse(candidates[i].vram_mb.unwrap_or(0))));
    let reason = match (wanted, wanted.find(candidates)) {
        (Wanted::Auto, _) => Reason::Automatic,
        (_, Some(i)) if candidates[i].surface => {
            auto.retain(|&j| j != i);
            auto.insert(0, i);
            Reason::Wanted
        }
        _ => Reason::WantedMissing,
    };
    Plan { order: auto, reason }
}

/// Why the automatic choice may not be what the user expects. The log
/// says it at startup, and Preferences › Graphics › Graphics card shows it,
/// since a tester on a hybrid PC could not tell why the integrated GPU drew
/// the viewer (#894).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Notice {
    /// Only an integrated GPU is visible to Vulkan.
    NoDiscrete,
    /// A discrete GPU is visible but cannot present to the window.
    DiscreteCannotPresent,
}

impl Notice {
    pub fn log_line(self) -> &'static str {
        match self {
            Notice::NoDiscrete => {
                "no discrete GPU visible to Vulkan: on a hybrid PC the NVIDIA Control Panel's \
                 \"Preferred graphics processor\" for aurora-viewer.exe (VK_LAYER_NV_optimus) can hide it, \
                 and a driver left in that state until the PC restarts; \
                 Windows › Graphics settings do not apply to Vulkan"
            }
            Notice::DiscreteCannotPresent => {
                "the discrete GPU cannot present to the viewer's window (is the monitor plugged into the motherboard?)"
            }
        }
    }
}

/// The notice for these adapters, if any: no discrete GPU shown, or one
/// that cannot present.
pub fn hint(candidates: &[Candidate]) -> Option<Notice> {
    let discrete = candidates.iter().filter(|c| c.kind == Kind::Discrete);
    if discrete.clone().count() == 0 {
        candidates.iter().any(|c| c.kind == Kind::Integrated).then_some(Notice::NoDiscrete)
    } else if discrete.clone().all(|c| !c.surface) {
        Some(Notice::DiscreteCannotPresent)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(name: &str, kind: Kind, vram: Option<u64>, surface: bool) -> Candidate {
        Candidate {
            name: name.into(),
            kind,
            vram_mb: vram,
            surface,
        }
    }

    #[test]
    fn automatic_puts_discrete_first_then_more_memory() {
        let list = [
            c("Intel(R) Graphics", Kind::Integrated, Some(512), true),
            c("NVIDIA GeForce RTX 5070", Kind::Discrete, Some(12227), true),
            c("Microsoft Basic Render Driver", Kind::Cpu, None, true),
            c("AMD Radeon RX 6400", Kind::Discrete, Some(4080), true),
        ];
        let p = plan(&list, &Wanted::Auto);
        assert_eq!(p.order, vec![1, 3, 0, 2]);
        assert_eq!(p.reason, Reason::Automatic);
    }

    #[test]
    fn adapters_that_cannot_present_are_left_out() {
        let list = [
            c("Intel(R) Graphics", Kind::Integrated, Some(512), true),
            c("NVIDIA GeForce RTX 5070", Kind::Discrete, Some(12227), false),
        ];
        assert_eq!(plan(&list, &Wanted::Auto).order, vec![0]);
        // asked for by name, it still cannot be used
        let p = plan(&list, &Wanted::parse("rtx"));
        assert_eq!(p.order, vec![0]);
        assert_eq!(p.reason, Reason::WantedMissing);
        assert_eq!(hint(&list), Some(Notice::DiscreteCannotPresent));
    }

    #[test]
    fn the_wanted_adapter_comes_first_and_the_rest_stay_as_fallbacks() {
        let list = [
            c("NVIDIA GeForce RTX 5070", Kind::Discrete, Some(12227), true),
            c("Intel(R) Graphics", Kind::Integrated, Some(512), true),
        ];
        let p = plan(&list, &Wanted::parse(" intel "));
        assert_eq!(p.order, vec![1, 0]);
        assert_eq!(p.reason, Reason::Wanted);
        let p = plan(&list, &Wanted::parse("1"));
        assert_eq!(p.order, vec![1, 0]);
        // an index or a name that matches nothing: automatic
        let p = plan(&list, &Wanted::parse("7"));
        assert_eq!((p.order, p.reason), (vec![0, 1], Reason::WantedMissing));
        let p = plan(&list, &Wanted::parse("radeon"));
        assert_eq!(p.reason, Reason::WantedMissing);
    }

    #[test]
    fn a_saved_gpu_that_is_gone_falls_back_to_the_automatic_choice() {
        // #894: the card chosen in Preferences disabled in Device Manager,
        // or hidden by the driver: the viewer still starts, on what is left
        let only_intel = [c("Intel(R) UHD Graphics", Kind::Integrated, Some(128), true)];
        let p = plan(&only_intel, &Wanted::parse("NVIDIA GeForce RTX 3060 Laptop GPU"));
        assert_eq!((p.order, p.reason), (vec![0], Reason::WantedMissing));
        let only_nvidia = [c("NVIDIA GeForce RTX 3060 Laptop GPU", Kind::Discrete, Some(6144), true)];
        let p = plan(&only_nvidia, &Wanted::parse("Intel(R) UHD Graphics"));
        assert_eq!((p.order, p.reason), (vec![0], Reason::WantedMissing));
    }

    #[test]
    fn no_adapter_that_can_present_leaves_nothing_to_try() {
        let list = [c("NVIDIA GeForce RTX 3060", Kind::Discrete, Some(6144), false)];
        assert!(plan(&list, &Wanted::Auto).order.is_empty());
        assert!(plan(&[], &Wanted::Auto).order.is_empty());
    }

    #[test]
    fn parse_reads_auto_numbers_and_names() {
        assert_eq!(Wanted::parse(""), Wanted::Auto);
        assert_eq!(Wanted::parse("Auto"), Wanted::Auto);
        assert_eq!(Wanted::parse("0"), Wanted::Index(0));
        assert_eq!(Wanted::parse("NVIDIA GeForce"), Wanted::Name("nvidia geforce".into()));
    }

    #[test]
    fn hint_when_only_an_integrated_gpu_is_seen() {
        let list = [c("Intel(R) Graphics", Kind::Integrated, Some(512), true)];
        assert_eq!(hint(&list), Some(Notice::NoDiscrete));
        assert!(Notice::NoDiscrete.log_line().contains("NV_optimus"));
        let list = [c("NVIDIA GeForce RTX 4090", Kind::Discrete, Some(24138), true)];
        assert_eq!(hint(&list), None);
    }
}
