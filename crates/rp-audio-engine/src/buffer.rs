/// Audio buffer utilities and management

/// A simple ring buffer for audio samples
pub struct AudioRingBuffer {
    buffer: Vec<f32>,
    write_pos: usize,
    read_pos: usize,
    capacity: usize,
}

impl AudioRingBuffer {
    pub fn new(capacity: usize) -> Self {
        Self {
            buffer: vec![0.0; capacity],
            write_pos: 0,
            read_pos: 0,
            capacity,
        }
    }

    /// Number of samples available to read
    pub fn available(&self) -> usize {
        if self.write_pos >= self.read_pos {
            self.write_pos - self.read_pos
        } else {
            self.capacity - self.read_pos + self.write_pos
        }
    }

    /// Number of samples that can be written
    pub fn space(&self) -> usize {
        self.capacity - self.available() - 1
    }

    /// Write samples to the buffer
    pub fn write(&mut self, samples: &[f32]) -> usize {
        let to_write = samples.len().min(self.space());

        for &sample in &samples[..to_write] {
            self.buffer[self.write_pos] = sample;
            self.write_pos = (self.write_pos + 1) % self.capacity;
        }

        to_write
    }

    /// Read samples from the buffer
    pub fn read(&mut self, output: &mut [f32]) -> usize {
        let to_read = output.len().min(self.available());

        for sample in &mut output[..to_read] {
            *sample = self.buffer[self.read_pos];
            self.read_pos = (self.read_pos + 1) % self.capacity;
        }

        to_read
    }

    /// Clear the buffer
    pub fn clear(&mut self) {
        self.write_pos = 0;
        self.read_pos = 0;
    }
}

/// Pre-allocated scratch buffer for audio processing
pub struct ScratchBuffer {
    buffer: Vec<f32>,
}

impl ScratchBuffer {
    pub fn new(max_frames: usize, channels: usize) -> Self {
        Self {
            buffer: vec![0.0; max_frames * channels],
        }
    }

    pub fn as_slice(&self) -> &[f32] {
        &self.buffer
    }

    pub fn as_mut_slice(&mut self) -> &mut [f32] {
        &mut self.buffer
    }

    pub fn clear(&mut self) {
        self.buffer.fill(0.0);
    }
}
