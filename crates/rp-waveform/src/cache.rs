use crate::mipmap::WaveformMipmap;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

/// Cache for waveform data with LRU eviction
pub struct WaveformCache {
    /// In-memory cache
    cache: HashMap<u64, Arc<WaveformMipmap>>,
    /// Cache directory for disk persistence
    cache_dir: PathBuf,
    /// Maximum memory usage in bytes
    max_memory: usize,
    /// Current memory usage estimate
    current_memory: usize,
}

impl WaveformCache {
    pub fn new(cache_dir: PathBuf, max_memory: usize) -> Self {
        Self {
            cache: HashMap::new(),
            cache_dir,
            max_memory,
            current_memory: 0,
        }
    }

    /// Get a waveform from cache by track hash
    pub fn get(&self, track_hash: u64) -> Option<Arc<WaveformMipmap>> {
        self.cache.get(&track_hash).cloned()
    }

    /// Insert a waveform into cache
    pub fn insert(&mut self, track_hash: u64, waveform: WaveformMipmap) {
        let size = self.estimate_size(&waveform);

        // Evict if necessary
        while self.current_memory + size > self.max_memory && !self.cache.is_empty() {
            self.evict_one();
        }

        self.current_memory += size;
        self.cache.insert(track_hash, Arc::new(waveform));
    }

    /// Check if a waveform exists on disk
    pub fn exists_on_disk(&self, track_hash: u64) -> bool {
        self.disk_path(track_hash).exists()
    }

    /// Get the disk path for a track's waveform
    fn disk_path(&self, track_hash: u64) -> PathBuf {
        self.cache_dir.join(format!("{:016x}.waveform", track_hash))
    }

    /// Estimate memory size of a waveform
    fn estimate_size(&self, waveform: &WaveformMipmap) -> usize {
        let mut size = std::mem::size_of::<WaveformMipmap>();
        for level in &waveform.levels {
            size += level.data.len() * std::mem::size_of::<crate::mipmap::WaveformPoint>();
        }
        size
    }

    /// Evict one item from cache (simple FIFO for now)
    fn evict_one(&mut self) {
        if let Some(key) = self.cache.keys().next().copied() {
            if let Some(waveform) = self.cache.remove(&key) {
                self.current_memory -= self.estimate_size(&waveform);
            }
        }
    }

    /// Clear all cached waveforms
    pub fn clear(&mut self) {
        self.cache.clear();
        self.current_memory = 0;
    }
}
