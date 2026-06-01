pub mod transport;
pub mod deck;
pub mod mixer;
pub mod engine;
pub mod commands;
pub mod buffer;

pub use engine::{AudioEngine, start_audio};
pub use transport::GlobalTransport;
pub use deck::Deck;
pub use mixer::Mixer;
pub use commands::{command_channel, Command, CommandSender, CommandReceiver};

// Re-export the Link surface the host app needs (AppState shares the Arc with
// the engine and toggles enable/disable from the UI thread).
pub use rp_link::{LinkAudio, SessionState};
