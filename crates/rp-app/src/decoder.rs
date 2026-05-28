use rp_audio_engine::deck::AudioBuffer;
use rp_core::{RecordPlayerError, Result};
use std::fs::File;
use std::path::Path;
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

/// Decodes audio files using symphonia
pub struct AudioDecoder;

impl AudioDecoder {
    /// Decode an audio file to an AudioBuffer
    pub fn decode(path: &Path) -> Result<AudioBuffer> {
        let file = File::open(path)?;
        let mss = MediaSourceStream::new(Box::new(file), Default::default());

        // Create a hint to help the probe
        let mut hint = Hint::new();
        if let Some(ext) = path.extension() {
            hint.with_extension(&ext.to_string_lossy());
        }

        // Probe the media source
        let probed = symphonia::default::get_probe()
            .format(&hint, mss, &FormatOptions::default(), &MetadataOptions::default())
            .map_err(|e| RecordPlayerError::Decoding(e.to_string()))?;

        let mut format = probed.format;

        // Find the first audio track
        let track = format
            .tracks()
            .iter()
            .find(|t| t.codec_params.codec != symphonia::core::codecs::CODEC_TYPE_NULL)
            .ok_or_else(|| RecordPlayerError::Decoding("No audio track found".into()))?;

        let track_id = track.id;
        let codec_params = track.codec_params.clone();

        let sample_rate = codec_params
            .sample_rate
            .ok_or_else(|| RecordPlayerError::Decoding("Unknown sample rate".into()))?;

        let channels = codec_params
            .channels
            .ok_or_else(|| RecordPlayerError::Decoding("Unknown channel count".into()))?
            .count() as u16;

        // Create a decoder
        let mut decoder = symphonia::default::get_codecs()
            .make(&codec_params, &DecoderOptions::default())
            .map_err(|e| RecordPlayerError::Decoding(e.to_string()))?;

        // Decode all packets
        let mut samples: Vec<f32> = Vec::new();

        loop {
            let packet = match format.next_packet() {
                Ok(packet) => packet,
                Err(symphonia::core::errors::Error::IoError(err))
                    if err.kind() == std::io::ErrorKind::UnexpectedEof =>
                {
                    break;
                }
                Err(err) => return Err(RecordPlayerError::Decoding(err.to_string())),
            };

            if packet.track_id() != track_id {
                continue;
            }

            let decoded = decoder
                .decode(&packet)
                .map_err(|e| RecordPlayerError::Decoding(e.to_string()))?;

            // Get the audio buffer spec
            let spec = *decoded.spec();
            let duration = decoded.capacity() as u64;

            // Create a sample buffer and copy samples
            let mut sample_buf = SampleBuffer::<f32>::new(duration, spec);
            sample_buf.copy_interleaved_ref(decoded);

            samples.extend_from_slice(sample_buf.samples());
        }

        Ok(AudioBuffer::new(samples, sample_rate, channels))
    }
}
