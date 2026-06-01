//! Sink-side LinkAudio bindings (broadcast only).
//!
//! Wraps the LinkAudio C API added in Ableton Link 4.0 — but only the surface
//! recordplayer uses: enable/disable Link Audio, set the peer name, create
//! sinks, and commit per-block audio buffers. Source-side functionality
//! (subscribing to other peers' channels, channel discovery, change callbacks)
//! is intentionally omitted; we publish per-deck audio but do not consume any.
//!
//! Sample format on the wire is interleaved 16-bit signed PCM — the convention
//! Link Audio uses regardless of the host's internal sample format. Callers
//! are responsible for the f32→i16 conversion before `commit`.

use crate::rust_bindings::*;
use crate::session_state::SessionState;
use std::ffi::{CStr, CString};

/// A LinkAudio instance. This is a *separate* construct from `AblLink`: an
/// application picks one or the other. We use `LinkAudio` exclusively because
/// it provides the full Link tempo/phase surface *plus* audio sharing.
///
/// The underlying handle is `Send + Sync` (the C library is thread-safe for
/// the methods we use), so it can be shared via `Arc<LinkAudio>` between the
/// UI thread (enable/disable, peer name) and the audio thread (capture/commit
/// session state, sink buffer commits).
pub struct LinkAudio {
    pub(crate) link: abl_link,
}

unsafe impl Send for LinkAudio {}
unsafe impl Sync for LinkAudio {}

impl Drop for LinkAudio {
    fn drop(&mut self) {
        unsafe { abl_link_destroy(self.link) }
    }
}

impl LinkAudio {
    /// Construct a LinkAudio instance with an initial tempo and a peer name
    /// shown to other Link peers. Link itself is enabled by default; Link
    /// Audio is *not* — call [`enable_link_audio`] to start announcing sinks.
    ///
    /// Realtime-safe: no
    pub fn new(bpm: f64, peer_name: &str) -> Self {
        let link = unsafe { abl_link_create(bpm) };
        let me = Self { link };
        me.set_peer_name(peer_name);
        me
    }

    // --- basic Link surface (mirrors AblLink) -----------------------------

    /// Realtime-safe: yes
    pub fn is_enabled(&self) -> bool {
        unsafe { abl_link_is_enabled(self.link) }
    }

    /// Realtime-safe: no
    pub fn enable(&self, enable: bool) {
        unsafe { abl_link_enable(self.link, enable) }
    }

    /// Realtime-safe: yes
    pub fn num_peers(&self) -> u64 {
        unsafe { abl_link_num_peers(self.link) }
    }

    /// Realtime-safe: yes
    pub fn clock_micros(&self) -> i64 {
        unsafe { abl_link_clock_micros(self.link) }
    }

    /// Capture the current Link session state from the audio thread. Pair
    /// with [`commit_audio_session_state`] if mutations are made.
    ///
    /// Realtime-safe: yes
    pub fn capture_audio_session_state(&self, state: &mut SessionState) {
        unsafe { abl_link_capture_audio_session_state(self.link, state.session_state) }
    }

    /// Realtime-safe: yes
    pub fn commit_audio_session_state(&self, state: &SessionState) {
        unsafe { abl_link_commit_audio_session_state(self.link, state.session_state) }
    }

    /// Capture from an application (non-audio) thread.
    ///
    /// Realtime-safe: no
    pub fn capture_app_session_state(&self, state: &mut SessionState) {
        unsafe { abl_link_capture_app_session_state(self.link, state.session_state) }
    }

    /// Commit from an application (non-audio) thread.
    ///
    /// Realtime-safe: no
    pub fn commit_app_session_state(&self, state: &SessionState) {
        unsafe { abl_link_commit_app_session_state(self.link, state.session_state) }
    }

    // --- Link Audio: enable / peer name -----------------------------------

    /// Realtime-safe: yes
    pub fn is_link_audio_enabled(&self) -> bool {
        unsafe { abl_link_audio_is_link_audio_enabled(self.link) }
    }

    /// Realtime-safe: no
    pub fn enable_link_audio(&self, enable: bool) {
        unsafe { abl_link_audio_enable_link_audio(self.link, enable) }
    }

    /// Set the peer name shown in remote channel listings. Names longer than
    /// 256 bytes are truncated by the underlying C library.
    ///
    /// Realtime-safe: no
    pub fn set_peer_name(&self, name: &str) {
        // CString::new only fails on interior NULs; strip them defensively.
        let cleaned: String = name.chars().filter(|c| *c != '\0').collect();
        if let Ok(c) = CString::new(cleaned) {
            unsafe { abl_link_audio_set_peer_name(self.link, c.as_ptr()) }
        }
    }

    /// Read the current peer name. Bound to 256 bytes to match the C library's
    /// internal cap (see `abl_link_audio_set_peer_name`).
    ///
    /// Realtime-safe: no
    pub fn peer_name(&self) -> String {
        let mut buf = vec![0u8; 257]; // 256 + trailing NUL
        unsafe {
            abl_link_audio_peer_name(
                self.link,
                buf.as_mut_ptr() as *mut std::os::raw::c_char,
                buf.len(),
            );
            CStr::from_ptr(buf.as_ptr() as *const std::os::raw::c_char)
                .to_string_lossy()
                .into_owned()
        }
    }
}

/// A sink that announces an audio channel to the Link session. Audio is sent
/// only when at least one remote peer has subscribed to this sink's channel,
/// so an idle sink costs nothing on the wire.
///
/// Lifetime: the sink keeps a borrow on the `LinkAudio` it was created from
/// (encoded in Rust by holding a raw handle whose lifetime is `'static` —
/// users must ensure the `LinkAudio` outlives every `LinkAudioSink`). In
/// recordplayer both live for the lifetime of the audio engine.
///
/// `Send` — sinks are shipped from the UI thread (where they are created) into
/// the audio thread (which retains/commits buffers) via the engine struct.
pub struct LinkAudioSink {
    sink: abl_link_audio_sink,
}

unsafe impl Send for LinkAudioSink {}

impl Drop for LinkAudioSink {
    fn drop(&mut self) {
        unsafe { abl_link_audio_sink_destroy(self.sink) }
    }
}

impl LinkAudioSink {
    /// Create a new sink with a channel name and the maximum buffer size in
    /// samples (frames × channels). For stereo at 1024-frame blocks this is
    /// `1024 * 2 = 2048`. `request_max_num_samples` can grow this later.
    ///
    /// Realtime-safe: no
    pub fn new(link: &LinkAudio, name: &str, max_num_samples: usize) -> Self {
        let cleaned: String = name.chars().filter(|c| *c != '\0').collect();
        let c = CString::new(cleaned).expect("name without NULs");
        let sink = unsafe {
            abl_link_audio_sink_create(link.link, c.as_ptr(), max_num_samples)
        };
        Self { sink }
    }

    /// Realtime-safe: no
    pub fn set_name(&self, name: &str) {
        let cleaned: String = name.chars().filter(|c| *c != '\0').collect();
        if let Ok(c) = CString::new(cleaned) {
            unsafe { abl_link_audio_sink_set_name(self.sink, c.as_ptr()) }
        }
    }

    /// Realtime-safe: yes — buffer-pool growth is a no-op when the requested
    /// size is ≤ the current one.
    pub fn request_max_num_samples(&self, n: usize) {
        unsafe { abl_link_audio_sink_request_max_num_samples(self.sink, n) }
    }

    /// Realtime-safe: yes
    pub fn max_num_samples(&self) -> usize {
        unsafe { abl_link_audio_sink_max_num_samples(self.sink) }
    }

    /// Retain a buffer for the audio thread to write into and commit. Returns
    /// `None` when no buffer is available (no remote source is subscribed, or
    /// the pool is momentarily exhausted). Callers must `commit` or drop —
    /// drop releases the buffer back to the pool.
    ///
    /// Realtime-safe: yes
    pub fn retain_buffer(&self) -> Option<SinkBuffer<'_>> {
        let handle = unsafe { abl_link_audio_sink_retain_buffer(self.sink) };
        let valid = unsafe { abl_link_audio_sink_buffer_is_valid(&handle) };
        if !valid {
            // The C side may have returned a non-valid handle (no source
            // subscribed). Drop it by calling release, then signal None.
            let mut h = handle;
            unsafe { abl_link_audio_sink_buffer_release(&mut h) };
            return None;
        }
        Some(SinkBuffer {
            handle,
            committed: false,
            _marker: std::marker::PhantomData,
        })
    }
}

/// RAII handle to a retained sink buffer. Holds a raw `int16_t *` and a max
/// sample capacity; writers fill `samples()` then call `commit`. If dropped
/// without committing, the buffer is released back to the pool.
pub struct SinkBuffer<'a> {
    handle: abl_link_audio_sink_buffer_handle,
    committed: bool,
    _marker: std::marker::PhantomData<&'a LinkAudioSink>,
}

impl<'a> SinkBuffer<'a> {
    /// Maximum number of i16 samples the buffer can hold (frames × channels).
    pub fn max_num_samples(&self) -> usize {
        self.handle.max_num_samples
    }

    /// Mutable slice of the sample buffer. Interleaved i16, length =
    /// `max_num_samples()`. Writer fills `numFrames * numChannels` samples
    /// and then calls `commit`.
    ///
    /// # Safety
    /// The underlying buffer is owned by the Link C library; the slice is
    /// only valid until the `SinkBuffer` is consumed by `commit` or dropped.
    /// Borrow checker enforces this through `&mut self`.
    pub fn samples(&mut self) -> &mut [i16] {
        let n = self.handle.max_num_samples;
        // SAFETY: pointer is non-null and valid for `max_num_samples` writes
        // while we hold the retained handle; C guarantees this.
        unsafe { std::slice::from_raw_parts_mut(self.handle.samples, n) }
    }

    /// Commit the buffer to the Link session. `beats_at_buffer_begin` must
    /// match the Link beat that corresponds to the *first frame* of this
    /// buffer (computed via `SessionState::beat_at_time(host_time_at_block_start, quantum)`),
    /// and `quantum` must match what the rest of the app uses for phase math.
    ///
    /// Returns false if the C library rejects the commit (e.g., too many
    /// samples). Realtime-safe: yes.
    pub fn commit(
        mut self,
        state: &SessionState,
        beats_at_buffer_begin: f64,
        quantum: f64,
        num_frames: usize,
        num_channels: usize,
        sample_rate: u32,
    ) -> bool {
        let ok = unsafe {
            abl_link_audio_sink_buffer_commit(
                &mut self.handle,
                state.session_state,
                beats_at_buffer_begin,
                quantum,
                num_frames,
                num_channels,
                sample_rate,
            )
        };
        self.committed = true;
        ok
    }
}

impl<'a> Drop for SinkBuffer<'a> {
    fn drop(&mut self) {
        // The C API only documents an explicit release path; calling release
        // on an already-committed handle is undefined, so only release if
        // commit was not called.
        if !self.committed {
            unsafe { abl_link_audio_sink_buffer_release(&mut self.handle) };
        }
    }
}
