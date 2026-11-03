//! The pre-generation engine: job state, the per-tick work loop, throttling and saving.
//!
//! One rule shapes this file: the Pumpkin API docs warn against holding a lock while
//! calling into the server (the server may call back into the plugin). So every tick
//! runs in phases:
//!   1. lock  -> decide what to do, copy what we need, unlock
//!   2. (no lock) ask the server to load/generate chunks
//!   3. lock  -> record the results, unlock
//!   4. (no lock) broadcast / save

use crate::{
    area::{self, Cursor, Shape},
    config::Config,
    pretty,
};
use pumpkin_plugin_api::Server;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    sync::{Mutex, MutexGuard, OnceLock},
    time::{Duration, Instant},
};
use tracing::{info, warn};

// ---------------------------------------------------------------------------
// Saved data
// ---------------------------------------------------------------------------

/// What `/pregen center`, `radius`, `shape` and `world` have set up (not started yet).
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct Selection {
    /// Empty = use the sender's world, or the config default.
    pub world: String,
    pub shape: String,
    /// Block coordinates. `None` = use the world border's centre.
    pub center: Option<(i32, i32)>,
    pub radius: i32,
}

impl Default for Selection {
    fn default() -> Self {
        Selection {
            world: String::new(),
            shape: "square".to_string(),
            center: None,
            radius: 1000,
        }
    }
}

/// A run in progress. Everything needed to continue after a restart is in here.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Job {
    pub world: String,
    pub shape: String,
    pub center_block: (i32, i32),
    pub radius_blocks: i32,
    pub radius_chunks: i32,
    pub ring: i32,
    pub pos: i32,
    pub total: u64,
    pub done: u64,
    /// `get_chunk` returned a chunk.
    pub generated: u64,
    /// `get_chunk` returned nothing (outside the border, or the server declined).
    pub missed: u64,
    pub active_ms: u64,
    pub paused: bool,
}

impl Job {
    pub fn shape(&self) -> Shape {
        Shape::parse(&self.shape)
    }
    pub fn cursor(&self) -> Cursor {
        Cursor { ring: self.ring, pos: self.pos }
    }
    pub fn center_chunk(&self) -> (i32, i32) {
        (
            area::block_to_chunk(self.center_block.0),
            area::block_to_chunk(self.center_block.1),
        )
    }
}

#[derive(Serialize, Deserialize, Default)]
#[serde(default)]
struct Persisted {
    selection: Selection,
    job: Option<Job>,
}

// ---------------------------------------------------------------------------
// Runtime-only data (not saved)
// ---------------------------------------------------------------------------

#[derive(Default)]
pub struct Runtime {
    /// Smoothed chunks per second.
    pub cps: f64,
    pub lagging: bool,
    pub last_summary: Option<String>,
    pub task_id: Option<u32>,
    sample_at: Option<Instant>,
    sample_done: u64,
    last_tick: Option<Instant>,
    last_progress: Option<Instant>,
    since_world_save: u64,
    ticks_since_persist: u32,
}

impl Runtime {
    fn reset_sampling(&mut self) {
        self.cps = 0.0;
        self.sample_at = None;
        self.last_tick = None;
        self.last_progress = None;
        self.sample_done = 0;
    }
}

pub struct Engine {
    pub cfg: Config,
    pub data_dir: String,
    pub selection: Selection,
    pub job: Option<Job>,
    pub rt: Runtime,
}

static ENGINE: OnceLock<Mutex<Engine>> = OnceLock::new();

/// Locks the engine. Never hold the guard while calling a host (server) function.
pub fn engine() -> MutexGuard<'static, Engine> {
    #[allow(clippy::expect_used)]
    ENGINE
        .get()
        .expect("engine used before init")
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

pub fn init(cfg: Config, data_dir: String) {
    let path = format!("{data_dir}/state.json");
    let saved: Persisted = fs::read_to_string(&path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default();

    let mut job = saved.job;
    if let Some(j) = job.as_mut() {
        j.paused = !cfg.auto_resume;
        info!(
            "Found an unfinished pregeneration of {} ({}). {}",
            j.world,
            pretty::percent(j.done, j.total),
            if cfg.auto_resume { "Resuming." } else { "Use /pregen resume to continue." }
        );
    }

    let engine = Engine { cfg, data_dir, selection: saved.selection, job, rt: Runtime::default() };
    let _ = ENGINE.set(Mutex::new(engine));
}

// ---------------------------------------------------------------------------
// Commands' view of the engine
// ---------------------------------------------------------------------------

impl Engine {
    pub fn persist(&self) {
        let data = Persisted { selection: self.selection.clone(), job: self.job.clone() };
        match serde_json::to_string_pretty(&data) {
            Ok(text) => {
                if let Err(e) = fs::write(format!("{}/state.json", self.data_dir), text) {
                    warn!("Could not save pregen state: {e}");
                }
            }
            Err(e) => warn!("Could not encode pregen state: {e}"),
        }
    }

    pub fn start_job(&mut self, world: String, border_center: (i32, i32)) -> Result<String, String> {
        if self.job.is_some() {
            return Err("A pregeneration already exists. Use /pregen resume, or /pregen cancel first.".into());
        }
        let sel = self.selection.clone();
        let shape = Shape::parse(&sel.shape);
        let center = sel.center.unwrap_or(border_center);
        let radius_chunks = area::blocks_to_chunk_radius(sel.radius);
        let total = area::total_chunks(shape, radius_chunks);

        self.job = Some(Job {
            world: world.clone(),
            shape: shape.as_str().to_string(),
            center_block: center,
            radius_blocks: sel.radius,
            radius_chunks,
            ring: 0,
            pos: 0,
            total,
            done: 0,
            generated: 0,
            missed: 0,
            active_ms: 0,
            paused: false,
        });
        self.rt.reset_sampling();
        self.rt.last_summary = None;
        self.persist();
        Ok(format!(
            "Started: {} chunks, {} of radius {} blocks around ({}, {}) in {}",
            pretty::thousands(total),
            shape.as_str(),
            sel.radius,
            center.0,
            center.1,
            world
        ))
    }

    pub fn pause(&mut self) -> Result<String, String> {
        match self.job.as_mut() {
            None => Err("Nothing is running.".into()),
            Some(j) if j.paused => Err("Already paused.".into()),
            Some(j) => {
                j.paused = true;
                self.rt.last_tick = None;
                self.persist();
                Ok("Paused. Use /pregen resume to continue.".into())
            }
        }
    }

    pub fn resume(&mut self) -> Result<String, String> {
        match self.job.as_mut() {
            None => Err("Nothing to resume. Use /pregen start.".into()),
            Some(j) if !j.paused => Err("Already running.".into()),
            Some(j) => {
                j.paused = false;
                self.rt.reset_sampling();
                self.persist();
                Ok("Resumed.".into())
            }
        }
    }

    pub fn cancel(&mut self) -> Result<String, String> {
        match self.job.take() {
            None => Err("Nothing to cancel.".into()),
            Some(j) => {
                self.rt.reset_sampling();
                self.persist();
                Ok(format!(
                    "Cancelled at {} ({} chunks). Chunks already generated are kept.",
                    pretty::percent(j.done, j.total),
                    pretty::thousands(j.done)
                ))
            }
        }
    }

    /// Lines for /pregen status, as (label, value).
    pub fn status_lines(&self, tps: f64, mspt: f64) -> Vec<(String, String)> {
        let mut out = Vec::new();
        match &self.job {
            None => {
                out.push(("State".into(), "idle".into()));
                if let Some(s) = &self.rt.last_summary {
                    out.push(("Last run".into(), s.clone()));
                }
            }
            Some(j) => {
                let state = if j.paused {
                    "paused"
                } else if self.rt.lagging {
                    "waiting for the server to catch up"
                } else {
                    "running"
                };
                out.push(("State".into(), state.into()));
                out.push(("World".into(), j.world.clone()));
                out.push((
                    "Area".into(),
                    format!(
                        "{} r={} blocks around ({}, {})",
                        j.shape, j.radius_blocks, j.center_block.0, j.center_block.1
                    ),
                ));
                out.push((
                    "Progress".into(),
                    format!(
                        "{} ({} / {} chunks)",
                        pretty::percent(j.done, j.total),
                        pretty::thousands(j.done),
                        pretty::thousands(j.total)
                    ),
                ));
                out.push(("Speed".into(), format!("{:.1} chunks/s", self.rt.cps)));
                out.push((
                    "ETA".into(),
                    if j.paused {
                        "-".into()
                    } else {
                        pretty::eta(j.total.saturating_sub(j.done), self.rt.cps)
                    },
                ));
                out.push(("Elapsed".into(), pretty::duration(j.active_ms / 1000)));
                out.push((
                    "Results".into(),
                    format!("{} loaded, {} missed", pretty::thousands(j.generated), pretty::thousands(j.missed)),
                ));
            }
        }
        out.push(("Server".into(), format!("{tps:.1} TPS, {mspt:.1} ms/tick")));
        out
    }
}

// ---------------------------------------------------------------------------
// The tick loop
// ---------------------------------------------------------------------------

struct Plan {
    world: String,
    shape: Shape,
    radius_chunks: i32,
    center_cx: i32,
    center_cz: i32,
    cursor: Cursor,
    budget: Duration,
    max_chunks: u64,
}

#[derive(Default)]
struct Effects {
    message: Option<String>,
    save_world: bool,
    persist: bool,
}

impl Engine {
    /// Phase 1: should we work this tick, and on what?
    fn plan(&mut self, tps: f64, mspt: f64) -> Option<Plan> {
        let Some(job) = self.job.as_ref() else {
            self.rt.last_tick = None;
            return None;
        };
        if job.paused {
            self.rt.last_tick = None;
            return None;
        }
        // tps of 0 means "not measured yet", not "lagging".
        if (tps > 0.0 && tps < self.cfg.min_tps) || mspt > self.cfg.max_mspt {
            self.rt.lagging = true;
            self.rt.last_tick = None;
            return None;
        }
        self.rt.lagging = false;

        let (cx, cz) = job.center_chunk();
        Some(Plan {
            world: job.world.clone(),
            shape: job.shape(),
            radius_chunks: job.radius_chunks,
            center_cx: cx,
            center_cz: cz,
            cursor: job.cursor(),
            budget: Duration::from_millis(self.cfg.budget_ms_per_tick),
            max_chunks: self.cfg.max_chunks_per_tick,
        })
    }

    /// Phase 3: record what the work loop did.
    fn apply(&mut self, cursor: Cursor, done: u64, generated: u64, missed: u64, finished: bool) -> Effects {
        let mut fx = Effects::default();
        let now = Instant::now();

        // The job can be cancelled between phases (it can't really, but be safe).
        let Some(job) = self.job.as_mut() else { return fx };

        job.ring = cursor.ring;
        job.pos = cursor.pos;
        job.done += done;
        job.generated += generated;
        job.missed += missed;

        // Time only counts while we are actually working.
        if let Some(prev) = self.rt.last_tick {
            let d = now.duration_since(prev);
            if d < Duration::from_secs(5) {
                job.active_ms += d.as_millis() as u64;
            }
        }
        self.rt.last_tick = Some(now);

        // Speed: re-measured every 2 s and smoothed.
        match self.rt.sample_at {
            None => {
                self.rt.sample_at = Some(now);
                self.rt.sample_done = job.done;
            }
            Some(t) => {
                let dt = now.duration_since(t).as_secs_f64();
                if dt >= 2.0 {
                    let inst = job.done.saturating_sub(self.rt.sample_done) as f64 / dt;
                    self.rt.cps = if self.rt.cps <= 0.0 { inst } else { 0.6 * self.rt.cps + 0.4 * inst };
                    self.rt.sample_at = Some(now);
                    self.rt.sample_done = job.done;
                }
            }
        }

        // Periodic world save.
        self.rt.since_world_save += done;
        let every = self.cfg.save_world_every_chunks;
        if every > 0 && self.rt.since_world_save >= every {
            self.rt.since_world_save = 0;
            fx.save_world = true;
        }

        // Save our own progress about every 5 s (100 ticks).
        self.rt.ticks_since_persist += 1;
        if self.rt.ticks_since_persist >= 100 {
            self.rt.ticks_since_persist = 0;
            fx.persist = true;
        }

        if finished {
            let summary = format!(
                "{} chunks in {} ({} loaded, {} missed)",
                pretty::thousands(job.done),
                pretty::duration(job.active_ms / 1000),
                pretty::thousands(job.generated),
                pretty::thousands(job.missed)
            );
            fx.message = Some(format!("[Pregen] Finished {}: {}", job.world, summary));
            fx.save_world = true;
            fx.persist = true;
            self.rt.last_summary = Some(summary);
            self.job = None;
            self.rt.reset_sampling();
            return fx;
        }

        // Progress message.
        let interval = self.cfg.progress_interval_secs;
        if interval > 0 {
            let due = match self.rt.last_progress {
                None => {
                    self.rt.last_progress = Some(now);
                    false
                }
                Some(t) => now.duration_since(t) >= Duration::from_secs(interval),
            };
            if due {
                self.rt.last_progress = Some(now);
                fx.message = Some(format!(
                    "[Pregen] {}: {} ({} / {}) | {:.1} chunks/s | ETA {}",
                    job.world,
                    pretty::percent(job.done, job.total),
                    pretty::thousands(job.done),
                    pretty::thousands(job.total),
                    self.rt.cps,
                    pretty::eta(job.total.saturating_sub(job.done), self.rt.cps)
                ));
            }
        }
        fx
    }

    fn fail_missing_world(&mut self, world: &str) {
        if let Some(j) = self.job.as_mut() {
            j.paused = true;
        }
        self.rt.last_tick = None;
        warn!("World '{world}' no longer exists; pregeneration paused.");
        self.persist();
    }
}

/// Runs every server tick (scheduled in `on_load`). Does nothing unless a run is active.
pub fn tick(server: Server) {
    let tps = server.get_tps();
    let mspt = server.get_mspt();

    // Phase 1
    let plan = engine().plan(tps, mspt);
    let Some(plan) = plan else { return };

    // Phase 2: no lock held while the server loads chunks.
    let Some(world) = server.get_world_by_name(&plan.world) else {
        engine().fail_missing_world(&plan.world);
        return;
    };

    let started = Instant::now();
    let mut cursor = plan.cursor;
    let (mut done, mut generated, mut missed) = (0u64, 0u64, 0u64);
    let mut finished = false;

    loop {
        // Always do at least one chunk, so a tiny budget still makes progress.
        if done >= plan.max_chunks || (done > 0 && started.elapsed() >= plan.budget) {
            break;
        }
        match cursor.next(plan.shape, plan.radius_chunks) {
            None => {
                finished = true;
                break;
            }
            Some((dx, dz)) => {
                // THE key call. See the README: `probe` checks that it really generates.
                if world.get_chunk(plan.center_cx + dx, plan.center_cz + dz).is_some() {
                    generated += 1;
                } else {
                    missed += 1;
                }
                done += 1;
            }
        }
    }

    // Phase 3
    let fx = engine().apply(cursor, done, generated, missed, finished);

    // Phase 4: no lock held.
    if let Some(msg) = &fx.message {
        info!("{msg}");
        if engine().cfg.broadcast_progress {
            server.broadcast(msg);
        }
    }
    if fx.save_world {
        if let Err(e) = server.save_all() {
            warn!("save_all failed: {e}");
        }
    }
    if fx.persist {
        engine().persist();
    }
}
