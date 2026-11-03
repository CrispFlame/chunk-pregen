//! Chunk Pregen: pre-generate chunks around a point (like Chunky, for Pumpkin).
//!
//! Layout:
//!   area.rs      pure geometry (shapes, the outward walk, counts), unit-tested
//!   pretty.rs    progress/ETA text, unit-tested
//!   config.rs    config.toml
//!   engine.rs    job state, per-tick work loop, throttling, saving
//!   commands.rs  the /pregen command tree

mod area;
mod commands;
mod config;
mod engine;
mod pretty;

use pumpkin_plugin_api::{
    Context, Plugin, PluginMetadata, register_plugin, scheduler,
    permission::{Permission, PermissionDefault},
    permissions::permissions,
};
use tracing::info;

struct ChunkPregen;

impl Plugin for ChunkPregen {
    fn new() -> Self {
        ChunkPregen
    }

    fn metadata(&self) -> PluginMetadata {
        PluginMetadata {
            name: "Chunk Pregen".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            authors: vec!["Chunk Pregen contributors".into()],
            description: "Pre-generate chunks around a point, with progress, ETA and pause/resume".into(),
            dependencies: vec![],
            // The plugin keeps its config and saved progress in its own data folder.
            permissions: vec![
                permissions::FS_READ_DATA.into(),
                permissions::FS_WRITE_DATA.into(),
            ],
        }
    }

    fn on_load(&self, context: Context) -> pumpkin_plugin_api::Result<()> {
        let data_dir = context.get_data_folder();
        let cfg = config::load_or_create(&data_dir);
        engine::init(cfg, data_dir);

        context.register_permission(&Permission {
            node: commands::PERMISSION_NODE.to_string(),
            description: "Lists /pregen (running it still needs operator level)".to_string(),
            default: PermissionDefault::Allow,
            children: Vec::new(),
        })?;
        commands::register(&context);

        // Runs every tick; returns immediately when no run is active.
        let task = scheduler::schedule_repeating_task(20, 1, engine::tick);
        engine::engine().rt.task_id = Some(task);

        info!("Chunk Pregen loaded. Try /pregen");
        Ok(())
    }

    fn on_unload(&self, _context: Context) -> pumpkin_plugin_api::Result<()> {
        let task = {
            let mut e = engine::engine();
            e.persist();
            e.rt.task_id.take()
        };
        if let Some(id) = task {
            scheduler::cancel_task(id);
        }
        Ok(())
    }
}

register_plugin!(ChunkPregen);
