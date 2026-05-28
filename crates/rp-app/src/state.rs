use rp_audio_engine::CommandSender;
use rp_library::TrackDatabase;
use rp_waveform::WaveformCache;
use std::path::PathBuf;
use std::sync::Mutex;

/// Application state shared across the app
pub struct AppState {
    /// Command sender for the audio engine
    pub command_sender: Mutex<CommandSender>,
    /// Track database
    pub database: Mutex<TrackDatabase>,
    /// Waveform cache
    pub waveform_cache: Mutex<WaveformCache>,
}

impl AppState {
    pub fn new(
        command_sender: CommandSender,
        database: TrackDatabase,
        cache_dir: PathBuf,
    ) -> Self {
        Self {
            command_sender: Mutex::new(command_sender),
            database: Mutex::new(database),
            waveform_cache: Mutex::new(WaveformCache::new(
                cache_dir,
                256 * 1024 * 1024, // 256 MB cache
            )),
        }
    }
}
