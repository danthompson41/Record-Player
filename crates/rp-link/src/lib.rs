//! Ableton Link 4 bindings used by recordplayer.
//!
//! Vendored from the `link_4` branch of [`rusty_link`](https://github.com/anzbert/rusty_link)
//! and extended with sink-side `LinkAudio` bindings. The upstream crate had a
//! placeholder for LinkAudio at the time of vendoring (May 2026); this crate
//! adds the audio-broadcast surface we actually need (one sink per deck) on top
//! of the C wrapper that already ships with Ableton Link 4.0.
//!
//! Only the parts recordplayer uses are wrapped:
//!   * `AblLink` + `SessionState` — tempo/phase sync (basic Link).
//!   * `LinkAudio` + `LinkAudioSink` — broadcast each deck's audio as a Link
//!     Audio channel. Receiving (sources, channel discovery) is intentionally
//!     omitted.

#[allow(
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    dead_code
)]
mod rust_bindings {
    include!(concat!(env!("OUT_DIR"), "/link_bindings.rs"));
}

mod abl_link;
mod host_time_filter;
mod link_audio;
mod session_state;
mod split;

pub use abl_link::AblLink;
pub use host_time_filter::HostTimeFilter;
pub use link_audio::{LinkAudio, LinkAudioSink, SinkBuffer};
pub use session_state::SessionState;
