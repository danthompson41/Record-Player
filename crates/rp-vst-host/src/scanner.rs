use std::path::PathBuf;

/// Scans for VST3 plugins on the system
pub struct PluginScanner {
    search_paths: Vec<PathBuf>,
}

impl PluginScanner {
    pub fn new() -> Self {
        Self {
            search_paths: Self::default_search_paths(),
        }
    }

    /// Get the default VST3 search paths for the current platform
    fn default_search_paths() -> Vec<PathBuf> {
        let mut paths = Vec::new();

        #[cfg(target_os = "macos")]
        {
            paths.push(PathBuf::from("/Library/Audio/Plug-Ins/VST3"));
            if let Some(home) = dirs::home_dir() {
                paths.push(home.join("Library/Audio/Plug-Ins/VST3"));
            }
        }

        #[cfg(target_os = "windows")]
        {
            paths.push(PathBuf::from("C:\\Program Files\\Common Files\\VST3"));
        }

        #[cfg(target_os = "linux")]
        {
            paths.push(PathBuf::from("/usr/lib/vst3"));
            if let Some(home) = dirs::home_dir() {
                paths.push(home.join(".vst3"));
            }
        }

        paths
    }

    /// Scan for available plugins
    pub fn scan(&self) -> Vec<PluginInfo> {
        // TODO: Implement actual VST3 scanning
        Vec::new()
    }
}

impl Default for PluginScanner {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone)]
pub struct PluginInfo {
    pub name: String,
    pub vendor: String,
    pub path: PathBuf,
    pub uid: [u8; 16],
}
