use serde::Deserialize;
use std::{fs, path::Path};
use tracing::{info, warn};

/// The shipped file doubles as the first-run file.
const DEFAULT_TOML: &str = include_str!("../config.toml");

#[derive(Deserialize, Clone, Debug)]
#[serde(default)]
pub struct Config {
    pub default_world: String,
    pub max_radius_blocks: i32,
    pub budget_ms_per_tick: u64,
    pub max_chunks_per_tick: u64,
    pub min_tps: f64,
    pub max_mspt: f64,
    pub progress_interval_secs: u64,
    pub broadcast_progress: bool,
    pub save_world_every_chunks: u64,
    pub auto_resume: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            default_world: "minecraft:overworld".to_string(),
            max_radius_blocks: 100_000,
            budget_ms_per_tick: 20,
            max_chunks_per_tick: 64,
            min_tps: 18.0,
            max_mspt: 45.0,
            progress_interval_secs: 30,
            broadcast_progress: true,
            save_world_every_chunks: 5000,
            auto_resume: false,
        }
    }
}

impl Config {
    /// Keep silly values from breaking the run loop.
    fn sanitized(mut self) -> Config {
        self.budget_ms_per_tick = self.budget_ms_per_tick.clamp(1, 45);
        self.max_chunks_per_tick = self.max_chunks_per_tick.max(1);
        self.max_radius_blocks = self.max_radius_blocks.clamp(16, 2_000_000);
        self
    }
}

/// Reads `<data_dir>/config.toml`, creating it on first run. Any problem falls back
/// to the built-in defaults, so a bad config never stops the plugin from loading.
pub fn load_or_create(data_dir: &str) -> Config {
    let path = format!("{data_dir}/config.toml");

    if !Path::new(&path).exists() {
        match fs::write(&path, DEFAULT_TOML) {
            Ok(()) => info!("Created default config at {path}"),
            Err(e) => warn!("Could not write {path} ({e}); using built-in defaults"),
        }
    }

    match fs::read_to_string(&path) {
        Ok(text) => match toml::from_str::<Config>(&text) {
            Ok(cfg) => cfg.sanitized(),
            Err(e) => {
                warn!("Invalid {path}: {e}. Using built-in defaults");
                Config::default()
            }
        },
        Err(_) => Config::default(),
    }
}
