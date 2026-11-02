//! Aurora Viewer media plugin host.
//!
//! Second Life viewers render web pages and videos in separate processes:
//! `SLPlugin` loads a media plugin library (`media_plugin_cef` for the web
//! through Dullahan / CEF, `media_plugin_libvlc` for video and audio streams,
//! `media_plugin_gstreamer` instead on Linux) and talks to the viewer through
//! LLSD XML messages on a loopback TCP socket; pixels arrive in a shared
//! memory segment. This crate is the viewer side
//! of that protocol, so the very same plugins as Firestorm's are used:
//!
//! * [`process::PluginProcess`]: LLPluginProcessParent (launch, handshake,
//!   heartbeat, shared memory, shutdown).
//! * [`media::MediaPlugin`]: LLPluginClassMedia (size negotiation, frames,
//!   events, input, navigation, transport, volume).
//! * [`process::PluginPaths`]: where SLPlugin and the plugins are found
//!   (a folder chosen by the user, the viewer's own, else Aurora's pack).
//! * `stream_volume` (Linux): a web page's volume, on the sound server.
//!
//! Derived from indra/llplugin of the Second Life / Firestorm viewer source,
//! Copyright (C) Linden Research, Inc. and The Phoenix Firestorm Project,
//! originally LGPL 2.1.

pub mod media;
pub mod message;
pub mod process;
mod shm;
#[cfg(target_os = "linux")]
mod stream_volume;

pub use media::{
    BrowserSettings, Cookie, Frame, KeyEvent, MediaEvent, MediaPlugin, MediaStatus, Modifiers, MouseEvent, Priority, Rect,
    TIME_MEDIA_PLUGIN, mime_from_scheme, next_power_of_2, plugin_for_mime,
};
pub use message::PluginMessage;
pub use process::{PluginPaths, PluginProcess, ProcState};
