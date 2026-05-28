use serde::{Deserialize, Serialize};

/// A single point in the waveform visualization
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[repr(C)]
pub struct WaveformPoint {
    /// Minimum sample value in this range (scaled to i16)
    pub min: i16,
    /// Maximum sample value in this range
    pub max: i16,
    /// RMS value for visual intensity
    pub rms: u16,
}

impl WaveformPoint {
    pub fn new(min: f32, max: f32, rms: f32) -> Self {
        Self {
            min: (min * i16::MAX as f32) as i16,
            max: (max * i16::MAX as f32) as i16,
            rms: (rms * u16::MAX as f32) as u16,
        }
    }

    /// Get normalized min value (-1.0 to 1.0)
    pub fn min_normalized(&self) -> f32 {
        self.min as f32 / i16::MAX as f32
    }

    /// Get normalized max value (-1.0 to 1.0)
    pub fn max_normalized(&self) -> f32 {
        self.max as f32 / i16::MAX as f32
    }

    /// Get normalized RMS value (0.0 to 1.0)
    pub fn rms_normalized(&self) -> f32 {
        self.rms as f32 / u16::MAX as f32
    }
}

/// A single mipmap level containing downsampled waveform data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MipmapLevel {
    /// Reduction factor (1, 2, 4, 8, 16, ...)
    pub reduction: u32,
    /// Waveform data for this level
    pub data: Vec<WaveformPoint>,
}

impl MipmapLevel {
    pub fn new(reduction: u32) -> Self {
        Self {
            reduction,
            data: Vec::new(),
        }
    }

    /// Number of points in this level
    pub fn len(&self) -> usize {
        self.data.len()
    }

    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }
}

/// Complete waveform mipmap with multiple resolution levels
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WaveformMipmap {
    /// Original sample rate
    pub sample_rate: u32,
    /// Total samples in the original audio
    pub total_samples: u64,
    /// Number of channels
    pub channels: u8,
    /// Mipmap levels (from highest to lowest resolution)
    pub levels: Vec<MipmapLevel>,
}

impl WaveformMipmap {
    pub fn new(sample_rate: u32, total_samples: u64, channels: u8) -> Self {
        Self {
            sample_rate,
            total_samples,
            channels,
            levels: Vec::new(),
        }
    }

    /// Select the appropriate mipmap level for a given zoom
    /// samples_per_pixel: how many audio samples should fit in one pixel
    pub fn get_level_for_zoom(&self, samples_per_pixel: f64) -> Option<&MipmapLevel> {
        // Find the level with the highest reduction that's still
        // finer than our target
        let target_reduction = samples_per_pixel.floor() as u32;

        self.levels
            .iter()
            .rev()
            .find(|l| l.reduction <= target_reduction.max(1))
            .or_else(|| self.levels.first())
    }

    /// Get waveform points for a visible range at appropriate detail level
    pub fn get_visible_range(
        &self,
        start_sample: u64,
        end_sample: u64,
        target_pixels: u32,
    ) -> Vec<WaveformPoint> {
        let range_samples = end_sample.saturating_sub(start_sample);
        let samples_per_pixel = range_samples as f64 / target_pixels as f64;

        let Some(level) = self.get_level_for_zoom(samples_per_pixel) else {
            return Vec::new();
        };

        // Calculate indices in this level
        let start_idx = (start_sample / level.reduction as u64) as usize;
        let end_idx = ((end_sample / level.reduction as u64) as usize).min(level.data.len());

        if start_idx >= level.data.len() {
            return Vec::new();
        }

        level.data[start_idx..end_idx].to_vec()
    }

    /// Get the duration in seconds
    pub fn duration_seconds(&self) -> f64 {
        self.total_samples as f64 / self.sample_rate as f64
    }
}
