//! Bounded declarative rig controls shared by authoring and scripted runtime.
//! These helpers never extend an avatar's skinned rig or execute script code.
use super::hand_ik::ConstraintMode;
use glam::{Quat, Vec3};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

pub const MAX_BYTES: usize = 8192;
pub const MAX_NODES: usize = 16;
pub const MAX_BINDINGS: usize = 16;
pub const MAX_KEYS: usize = 64;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, Hash)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Target {
    /// Product-local named link. Nil object inherits the publishing root.
    Link {
        #[serde(with = "object_reference")]
        object: Uuid,
        link: String,
    },
    /// Concrete editor pick retained until its root/name can be captured.
    Prim {
        id: Uuid,
        #[serde(default)]
        name: String,
    },
    /// Nil denotes the controlled avatar, making copies reusable.
    Bone {
        #[serde(default)]
        avatar: Uuid,
        bone: String,
    },
    Node {
        id: Uuid,
    },
    Source,
}
impl Default for Target {
    fn default() -> Self {
        Self::Bone {
            avatar: Uuid::nil(),
            bone: "mPelvis".into(),
        }
    }
}

fn bone_name(s: &str) -> bool {
    !s.is_empty() && s.len() <= 64 && s.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_')
}
impl Target {
    pub fn valid(&self) -> bool {
        match self {
            Self::Link { link, .. } => !link.is_empty() && link.len() <= 256,
            Self::Prim { id, name } => (!id.is_nil() || !name.is_empty()) && name.len() <= 256,
            Self::Bone { bone, .. } => bone_name(bone),
            Self::Node { id } => !id.is_nil(),
            Self::Source => true,
        }
    }
}

pub const ROOT_LINK: &str = "_ROOT_";

mod object_reference {
    use super::*;
    pub fn serialize<S: serde::Serializer>(id: &Uuid, serializer: S) -> Result<S::Ok, S::Error> {
        if id.is_nil() {
            serializer.serialize_str("source")
        } else {
            id.serialize(serializer)
        }
    }
    pub fn deserialize<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Uuid, D::Error> {
        let value = String::deserialize(deserializer)?;
        if value == "source" {
            Ok(Uuid::nil())
        } else {
            Uuid::parse_str(&value).map_err(serde::de::Error::custom)
        }
    }
}

impl Definition {
    pub fn targets(&self) -> impl Iterator<Item = &Target> {
        self.nodes.iter().map(|n| &n.parent).chain(self.bindings.iter().map(|b| &b.target))
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Transform {
    pub position: [f32; 3],
    pub rotation: [f32; 4],
}
impl Default for Transform {
    fn default() -> Self {
        Self {
            position: [0.0; 3],
            rotation: [0.0, 0.0, 0.0, 1.0],
        }
    }
}
impl Transform {
    pub fn new(p: Vec3, q: Quat) -> Self {
        Self {
            position: p.to_array(),
            rotation: q.normalize().to_array(),
        }
    }
    pub fn p(self) -> Vec3 {
        Vec3::from_array(self.position)
    }
    pub fn q(self) -> Quat {
        Quat::from_array(self.rotation).normalize()
    }
    pub fn valid(self) -> bool {
        self.p().is_finite() && self.p().length_squared() <= 64.0 * 64.0 && {
            let q = Quat::from_array(self.rotation);
            q.is_finite() && (0.9..=1.1).contains(&q.length_squared())
        }
    }
    pub fn compose(self, child: Self) -> Self {
        Self::new(self.p() + self.q() * child.p(), self.q() * child.q())
    }
    pub fn inverse(self) -> Self {
        let q = self.q().inverse();
        Self::new(q * (-self.p()), q)
    }
    pub fn interpolate(self, other: Self, t: f32) -> Self {
        Self::new(self.p().lerp(other.p(), t), self.q().slerp(other.q(), t))
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Curve {
    Step,
    #[default]
    Linear,
    Smooth,
    Bezier {
        controls: [f32; 4],
    },
}
impl Curve {
    pub fn valid(self) -> bool {
        match self {
            Self::Bezier {
                controls: [x1, y1, x2, y2],
            } => {
                [x1, y1, x2, y2].iter().all(|v| v.is_finite())
                    && (0.0..=1.0).contains(&x1)
                    && (0.0..=1.0).contains(&x2)
                    && x1 <= x2
                    && (-2.0..=3.0).contains(&y1)
                    && (-2.0..=3.0).contains(&y2)
            }
            _ => true,
        }
    }
    pub fn sample(self, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        match self {
            Self::Step => {
                if t >= 1.0 {
                    1.0
                } else {
                    0.0
                }
            }
            Self::Linear => t,
            Self::Smooth => t * t * (3.0 - 2.0 * t),
            Self::Bezier {
                controls: [x1, y1, x2, y2],
            } => {
                let b = |u: f32, a: f32, c: f32| 3.0 * (1.0 - u) * (1.0 - u) * u * a + 3.0 * (1.0 - u) * u * u * c + u * u * u;
                let (mut lo, mut hi) = (0.0, 1.0);
                for _ in 0..16 {
                    let mid = (lo + hi) * 0.5;
                    if b(mid, x1, x2) < t { lo = mid } else { hi = mid }
                }
                b((lo + hi) * 0.5, y1, y2)
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Playback {
    Once,
    #[default]
    Loop,
    PingPong,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Function {
    #[default]
    Sine,
    Triangle,
    Saw,
    Pulse,
    Ramp,
}
impl Function {
    fn sample(self, u: f32) -> f32 {
        match self {
            Self::Sine => (u * std::f32::consts::TAU).sin(),
            Self::Triangle => 1.0 - 4.0 * (u - 0.5).abs(),
            Self::Saw => 2.0 * u - 1.0,
            Self::Pulse => {
                if u < 0.5 {
                    1.0
                } else {
                    -1.0
                }
            }
            Self::Ramp => u,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Key {
    pub time: f32,
    pub transform: Transform,
    #[serde(default)]
    pub curve: Curve,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Motion {
    Keys {
        duration: f32,
        #[serde(default)]
        playback: Playback,
        keys: Vec<Key>,
    },
    Function {
        duration: f32,
        #[serde(default)]
        playback: Playback,
        #[serde(default)]
        function: Function,
        position: [f32; 3],
        degrees: [f32; 3],
        #[serde(default)]
        phase: f32,
        #[serde(default)]
        curve: Curve,
    },
}
impl Motion {
    pub fn duration(&self) -> f32 {
        match self {
            Self::Keys { duration, .. } | Self::Function { duration, .. } => *duration,
        }
    }
    pub fn playback(&self) -> Playback {
        match self {
            Self::Keys { playback, .. } | Self::Function { playback, .. } => *playback,
        }
    }
    pub fn valid(&self) -> bool {
        let d = self.duration();
        if !d.is_finite() || !(1.0 / 30.0..=600.0).contains(&d) {
            return false;
        }
        match self {
            Self::Keys { keys, .. } => {
                !keys.is_empty()
                    && keys.len() <= 32
                    && keys
                        .iter()
                        .all(|k| k.time.is_finite() && (0.0..=d).contains(&k.time) && k.transform.valid() && k.curve.valid())
                    && keys.windows(2).all(|w| w[1].time - w[0].time > 0.00001)
            }
            Self::Function {
                position,
                degrees,
                phase,
                curve,
                ..
            } => {
                Vec3::from_array(*position).is_finite()
                    && Vec3::from_array(*position).length_squared() <= 4096.0
                    && degrees.iter().all(|d| d.is_finite() && d.abs() <= 36000.0)
                    && phase.is_finite()
                    && phase.abs() <= 1000.0
                    && curve.valid()
            }
        }
    }
    pub fn sample(&self, time: f64) -> Transform {
        if !time.is_finite() || !self.valid() {
            return Transform::default();
        }
        let duration = self.duration();
        let phase = time / f64::from(duration);
        let u = (match self.playback() {
            Playback::Once => phase.clamp(0.0, 1.0),
            Playback::Loop => phase.rem_euclid(1.0),
            Playback::PingPong => 1.0 - (phase.rem_euclid(2.0) - 1.0).abs(),
        }) as f32;
        match self {
            Self::Keys { keys, .. } => {
                let time = u * duration;
                let index = keys.partition_point(|key| key.time <= time);
                if index == 0 {
                    return keys[0].transform;
                }
                if index == keys.len() {
                    return keys[index - 1].transform;
                }
                let a = &keys[index - 1];
                let b = &keys[index];
                a.transform
                    .interpolate(b.transform, a.curve.sample((time - a.time) / (b.time - a.time)))
            }
            Self::Function {
                function,
                position,
                degrees,
                phase: offset,
                curve,
                ..
            } => {
                let phase = (u + offset).rem_euclid(1.0);
                // A one-shot ramp must reach its endpoint rather than wrap.
                let phase = if *offset == 0.0 && u == 1.0 { 1.0 } else { phase };
                let value = function.sample(curve.sample(phase));
                let degrees = Vec3::from_array(*degrees) * value;
                Transform::new(
                    Vec3::from_array(*position) * value,
                    Quat::from_euler(
                        glam::EulerRot::ZYX,
                        degrees.z.to_radians(),
                        degrees.y.to_radians(),
                        degrees.x.to_radians(),
                    ),
                )
            }
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Node {
    pub id: Uuid,
    pub name: String,
    pub parent: Target,
    #[serde(default)]
    pub offset: Transform,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub motion: Option<Motion>,
}

fn yes() -> bool {
    true
}
fn full() -> f32 {
    1.0
}
/// Runtime entry conditions; authoring previews retain immediate pose control.
#[derive(Clone, Copy, Debug, Serialize, PartialEq)]
pub struct Activation {
    /// Distance before this binding's own correction; zero is unlimited.
    /// Other contacts can bring a waiting child into range.
    pub distance: f32,
    pub ease_in: f32,
    pub ease_out: f32,
}
impl Default for Activation {
    fn default() -> Self {
        Self {
            distance: 0.0,
            ease_in: 0.2,
            ease_out: 0.2,
        }
    }
}
impl Activation {
    pub fn valid(&self) -> bool {
        [(self.distance, 64.0), (self.ease_in, 60.0), (self.ease_out, 60.0)]
            .into_iter()
            .all(|(value, max)| value.is_finite() && (0.0..=max).contains(&value))
    }
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

impl<'de> Deserialize<'de> for Activation {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        // Already installed declarations can contain this obsolete field.
        // Discard it on input; never retain, execute or export a viewer delay.
        #[derive(Deserialize)]
        #[serde(default, deny_unknown_fields)]
        struct Input {
            distance: f32,
            ease_in: f32,
            ease_out: f32,
            #[serde(rename = "delay")]
            _discarded: Option<serde::de::IgnoredAny>,
        }
        impl Default for Input {
            fn default() -> Self {
                let a = Activation::default();
                Self {
                    distance: a.distance,
                    ease_in: a.ease_in,
                    ease_out: a.ease_out,
                    _discarded: None,
                }
            }
        }
        let a = Input::deserialize(deserializer)?;
        Ok(Self {
            distance: a.distance,
            ease_in: a.ease_in,
            ease_out: a.ease_out,
        })
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Binding {
    pub bone: String,
    pub target: Target,
    #[serde(default)]
    pub offset: Transform,
    #[serde(default = "yes")]
    pub orient: bool,
    #[serde(default)]
    pub contact: [f32; 3],
    #[serde(default)]
    pub root: Option<String>,
    #[serde(default)]
    pub mode: ConstraintMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pole: Option<[f32; 3]>,
    #[serde(default = "full")]
    pub weight: f32,
    #[serde(default = "yes")]
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Activation::is_default")]
    pub activation: Activation,
}
impl Binding {
    pub fn valid(&self) -> bool {
        bone_name(&self.bone)
            && self.activation.valid()
            && self.target.valid()
            && self.offset.valid()
            && self.root.as_deref().is_none_or(bone_name)
            && Vec3::from_array(self.contact).is_finite()
            && Vec3::from_array(self.contact).length_squared() <= 0.09
            && self.pole.is_none_or(|p| {
                self.mode == ConstraintMode::Ik && Vec3::from_array(p).is_finite() && Vec3::from_array(p).length_squared() <= 4096.0
            })
            && self.weight.is_finite()
            && (0.0..=1.0).contains(&self.weight)
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Definition {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub profiles: Vec<Profile>,
    #[serde(default)]
    pub nodes: Vec<Node>,
    #[serde(default)]
    pub bindings: Vec<Binding>,
}

/// A named selection of shared bindings; profiles never depend on animations.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    pub name: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bones: Vec<String>,
    /// Explicit slots distinguish several targets for the same joint.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bindings: Option<Vec<usize>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub smoothing: Option<[f32; 2]>,
}
impl Profile {
    pub fn contains(&self, index: usize, bone: &str) -> bool {
        self.bindings
            .as_ref()
            .map_or_else(|| self.bones.iter().any(|b| b == bone), |members| members.contains(&index))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Limit,
    Invalid,
    Cycle,
    Missing,
    Duplicate,
}
impl Definition {
    /// Authoring removes now-unused rule members together with their bindings.
    pub fn prune_profiles(&mut self) {
        for profile in &mut self.profiles {
            profile.bones.retain(|bone| self.bindings.iter().any(|b| &b.bone == bone));
            if let Some(members) = &mut profile.bindings {
                members.retain(|&i| i < self.bindings.len());
            }
        }
        self.profiles.retain(|p| p.bindings.is_some() || !p.bones.is_empty());
    }
    pub fn remove_binding(&mut self, index: usize) {
        if index >= self.bindings.len() {
            return;
        }
        self.bindings.remove(index);
        for profile in &mut self.profiles {
            if let Some(members) = &mut profile.bindings {
                members.retain(|&i| i != index);
                for i in members {
                    *i -= usize::from(*i > index);
                }
            }
        }
        self.prune_profiles();
    }
    pub fn validate(&self) -> Result<(), Error> {
        if self.nodes.len() > MAX_NODES || self.bindings.len() > MAX_BINDINGS || self.profiles.len() > MAX_BINDINGS {
            return Err(Error::Limit);
        }
        let mut ids = HashSet::new();
        let mut keys = 0;
        for node in &self.nodes {
            if node.id.is_nil() || !ids.insert(node.id) {
                return Err(Error::Duplicate);
            }
            if node.name.is_empty()
                || node.name.len() > 256
                || !node.parent.valid()
                || !node.offset.valid()
                || node.motion.as_ref().is_some_and(|m| !m.valid())
            {
                return Err(Error::Invalid);
            }
            if let Some(Motion::Keys { keys: k, .. }) = &node.motion {
                keys += k.len();
            }
        }
        if keys > MAX_KEYS {
            return Err(Error::Limit);
        }
        let mut bones = HashSet::new();
        for binding in &self.bindings {
            if !binding.valid() {
                return Err(Error::Invalid);
            }
            bones.insert(&binding.bone);
        }
        let mut names = HashSet::new();
        for profile in &self.profiles {
            if profile.name.trim().is_empty()
                || profile.name.trim() != profile.name
                || profile.name.len() > 64
                || profile.name.chars().any(|c| c.is_control() || c == '|')
                || !names.insert(&profile.name)
            {
                return Err(Error::Invalid);
            }
            if profile
                .smoothing
                .is_some_and(|values| values.iter().any(|v| !v.is_finite() || !(0.0..=60.0).contains(v)))
            {
                return Err(Error::Invalid);
            }
            if let Some(members) = &profile.bindings {
                if !profile.bones.is_empty() {
                    return Err(Error::Invalid);
                }
                if members.len() > MAX_BINDINGS {
                    return Err(Error::Limit);
                }
                let mut seen = HashSet::new();
                for &index in members {
                    if index >= self.bindings.len() {
                        return Err(Error::Missing);
                    }
                    if !seen.insert(index) {
                        return Err(Error::Duplicate);
                    }
                }
            } else if profile.bones.is_empty() || profile.bones.len() > MAX_BINDINGS {
                return Err(Error::Limit);
            }
            let mut assigned = HashSet::new();
            for bone in &profile.bones {
                if !bones.contains(bone) {
                    return Err(Error::Missing);
                }
                if !assigned.insert(bone) {
                    return Err(Error::Duplicate);
                }
            }
        }
        for target in self.nodes.iter().map(|n| &n.parent).chain(self.bindings.iter().map(|b| &b.target)) {
            if let Target::Node { id } = target
                && !ids.contains(id)
            {
                return Err(Error::Missing);
            }
        }
        let nodes: HashMap<_, _> = self.nodes.iter().map(|n| (n.id, n)).collect();
        for node in &self.nodes {
            let mut seen = HashSet::new();
            let mut current = node.id;
            while let Some(node) = nodes.get(&current) {
                if !seen.insert(current) {
                    return Err(Error::Cycle);
                }
                let Target::Node { id } = &node.parent else { break };
                current = *id;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn link_targets_round_trip_root_and_source_references_without_child_keys() {
        use super::*;
        for object in [Uuid::nil(), Uuid::from_u128(123)] {
            for link in [ROOT_LINK, "Grip", "Poignée"] {
                let target = Target::Link { object, link: link.into() };
                let json = serde_json::to_value(&target).unwrap();
                assert_eq!(json["kind"], "link");
                assert!(json.get("id").is_none());
                assert!(target.valid());
                assert_eq!(serde_json::from_value::<Target>(json).unwrap(), target);
            }
        }
        assert!(
            !Target::Link {
                object: Uuid::nil(),
                link: String::new()
            }
            .valid()
        );
        assert!(
            !Target::Link {
                object: Uuid::nil(),
                link: "x".repeat(257)
            }
            .valid()
        );
        assert!(serde_json::from_str::<Target>(r#"{"kind":"link","object":"invalid","link":"Grip"}"#).is_err());
    }
    use super::*;
    #[test]
    fn named_profiles_share_bindings_and_reject_duplicate_missing_or_unbounded_members() {
        let valid = serde_json::json!({
            "profiles":[{"name":"drive","bones":["mWristLeft","mWristRight"]},{"name":"gas_only","bones":["mWristRight"]}],
            "bindings":[{"bone":"mWristLeft","target":{"kind":"source"}},{"bone":"mWristRight","target":{"kind":"source"}}]
        });
        let definition: Definition = serde_json::from_value(valid.clone()).unwrap();
        assert_eq!(definition.validate(), Ok(()));
        assert_eq!(
            serde_json::from_str::<Definition>(&serde_json::to_string(&definition).unwrap()).unwrap(),
            definition
        );
        for invalid in [
            serde_json::json!([]),
            serde_json::json!(["mHead"]),
            serde_json::json!(["mWristLeft", "mWristLeft"]),
            serde_json::json!(vec!["mWristLeft"; 17]),
        ] {
            let mut value = valid.clone();
            value["profiles"][0]["bones"] = invalid;
            assert!(serde_json::from_value::<Definition>(value).unwrap().validate().is_err());
        }
        let mut value = valid.clone();
        value["profiles"][1]["name"] = "drive".into();
        assert!(serde_json::from_value::<Definition>(value).unwrap().validate().is_err());
        let mut value = valid;
        value["profiles"][0]["animations"] = serde_json::json!(["ready_F"]);
        assert!(
            serde_json::from_value::<Definition>(value).is_err(),
            "animation conditions are no longer supported"
        );
    }
    #[test]
    fn activation_defaults_and_authored_values_round_trip_with_bounded_finite_parameters() {
        let default: Activation = serde_json::from_str("{}").unwrap();
        assert_eq!(default, Activation::default());
        assert!(default.valid());
        let authored = Activation {
            distance: 0.4,
            ease_in: 0.6,
            ease_out: 0.8,
        };
        assert_eq!(
            serde_json::from_str::<Activation>(&serde_json::to_string(&authored).unwrap()).unwrap(),
            authored
        );
        assert!(serde_json::to_value(authored).unwrap().get("delay").is_none());
        let mut installed = serde_json::to_value(authored).unwrap();
        installed["delay"] = serde_json::json!(3600);
        assert_eq!(serde_json::from_value::<Activation>(installed).unwrap(), authored);
        for index in 0..3 {
            for value in [f32::NAN, f32::INFINITY, -0.01, 3601.0] {
                let mut invalid = authored;
                match index {
                    0 => invalid.distance = value,
                    1 => invalid.ease_in = value,
                    _ => invalid.ease_out = value,
                }
                assert!(!invalid.valid());
            }
        }
        assert!(serde_json::from_str::<Activation>("{\"unknown\":1}").is_err());
    }
    #[test]
    fn control_hierarchies_reject_cycles_dangling_parents_duplicate_bones_and_unbounded_data() {
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        let node = |id, parent| Node {
            id,
            name: "Control".into(),
            parent,
            offset: Transform::default(),
            motion: None,
        };
        let mut definition = Definition {
            profiles: Vec::new(),
            nodes: vec![node(a, Target::default()), node(b, Target::Node { id: a })],
            bindings: Vec::new(),
        };
        assert_eq!(definition.validate(), Ok(()));
        definition.nodes[0].parent = Target::Node { id: b };
        assert_eq!(definition.validate(), Err(Error::Cycle));
        definition.nodes[0].parent = Target::Node { id: Uuid::new_v4() };
        assert_eq!(definition.validate(), Err(Error::Missing));
        definition.nodes[0].parent = Target::default();
        definition.nodes[0].offset.position[0] = 65.0;
        assert_eq!(definition.validate(), Err(Error::Invalid));
        definition.nodes[0].offset = Transform::default();
        let binding: Binding = serde_json::from_value(serde_json::json!({"bone":"mHead","target":{"kind":"node","id":a}})).unwrap();
        definition.bindings = vec![binding.clone(), binding];
        assert_eq!(definition.validate(), Ok(()), "independent associations may share a source joint");
    }
    #[test]
    fn keyed_rotation_translation_playback_and_bezier_use_the_same_metric_math() {
        let mut motion = Motion::Keys {
            duration: 2.0,
            playback: Playback::Once,
            keys: vec![
                Key {
                    time: 0.0,
                    transform: Transform::default(),
                    curve: Curve::Linear,
                },
                Key {
                    time: 2.0,
                    transform: Transform::new(Vec3::X, Quat::from_rotation_z(1.0)),
                    curve: Curve::Smooth,
                },
            ],
        };
        assert!(motion.valid());
        assert!((motion.sample(1.0).p().x - 0.5).abs() < 1e-5);
        assert!(motion.sample(1.0).q().abs_diff_eq(Quat::from_rotation_z(0.5), 1e-5));
        assert_eq!(motion.sample(3.0).p(), Vec3::X);
        if let Motion::Keys { playback, .. } = &mut motion {
            *playback = Playback::PingPong;
        }
        assert!((motion.sample(3.0).p().x - 0.5).abs() < 1e-5);
        if let Motion::Keys { playback, .. } = &mut motion {
            *playback = Playback::Loop;
        }
        assert!(motion.sample(2.0).p().length() < 1e-5);
        let bezier = Curve::Bezier {
            controls: [0.25, 0.0, 0.75, 1.0],
        };
        assert!(bezier.valid());
        assert!((bezier.sample(0.5) - 0.5).abs() < 1e-4);
        let function = Motion::Function {
            duration: 2.0,
            playback: Playback::Once,
            function: Function::Ramp,
            position: [1.0, 0.0, 0.0],
            degrees: [0.0, 0.0, 90.0],
            phase: 0.0,
            curve: Curve::Linear,
        };
        assert!(function.sample(3.0).p().abs_diff_eq(Vec3::X, 1e-5));
        assert!(
            function
                .sample(3.0)
                .q()
                .abs_diff_eq(Quat::from_rotation_z(std::f32::consts::FRAC_PI_2), 1e-5)
        );
        let mut long = function.clone();
        if let Motion::Function { playback, .. } = &mut long {
            *playback = Playback::Loop;
        }
        assert!(
            (long.sample(100_000_001.0).p().x - 0.5).abs() < 1e-5,
            "large script uptime retains sub-cycle phase"
        );
    }
}
