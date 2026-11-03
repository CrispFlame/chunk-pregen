//! The /pregen command tree.
//!
//!   /pregen                      help
//!   /pregen world [name]         pick the world (no name = the world you are in)
//!   /pregen shape square|circle
//!   /pregen center               centre on you
//!   /pregen center <x> <z>       centre on block coordinates
//!   /pregen radius <blocks>
//!   /pregen border               use the world border's centre and size
//!   /pregen selection            show the settings
//!   /pregen start | pause | resume | cancel | status
//!   /pregen probe                test that chunk generation works on this server

use crate::{
    area::{self, Shape},
    engine::engine,
    pretty,
};
use pumpkin_plugin_api::{
    Context, Server,
    command::{
        Arg, ArgumentType, Command, CommandError, CommandNode, CommandSender, ConsumedArgs,
        StringType,
    },
    command_wit::{Number, PermissionLevel},
    commands::CommandHandler,
    text::{NamedColor, TextComponent},
};
use std::time::Instant;

pub const PERMISSION_NODE: &str = "chunk-pregen:use";

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Only operators may run /pregen: it uses a lot of CPU and disk.
fn guard(sender: &CommandSender) -> Result<(), CommandError> {
    if sender.has_permission_level(PermissionLevel::Three) {
        Ok(())
    } else {
        Err(CommandError::PermissionDenied)
    }
}

fn say(sender: &CommandSender, text: &str) {
    sender.send_message(
        TextComponent::text(&format!("[Pregen] {text}")).color_named(NamedColor::Gold),
    );
}

fn kv(sender: &CommandSender, key: &str, value: &str) {
    sender.send_message(
        TextComponent::text(&format!("{key}: ")).color_named(NamedColor::Gold)
            .add_child(TextComponent::text(value).color_named(NamedColor::Yellow)),
    );
}

fn fail(text: &str) -> CommandError {
    CommandError::CommandFailed(TextComponent::text(text).color_named(NamedColor::Red))
}

fn int_arg(args: &ConsumedArgs, key: &str) -> Option<i32> {
    match args.get_value(key) {
        Arg::Num(Ok(Number::Int32(n))) => Some(n),
        Arg::Num(Ok(Number::Int64(n))) => i32::try_from(n).ok(),
        _ => None,
    }
}

fn text_arg(args: &ConsumedArgs, key: &str) -> Option<String> {
    match args.get_value(key) {
        Arg::Simple(s) | Arg::Msg(s) => Some(s),
        _ => None,
    }
}

/// The sender's position in blocks (players only).
/// If this line fails to compile, the field names of `position` in `common.wit` differ.
fn sender_block_xz(sender: &CommandSender) -> Option<(i32, i32)> {
    sender.position().map(|p| (p.x.floor() as i32, p.z.floor() as i32))
}

/// Which world to use: the selection, else the sender's world, else the config default.
fn resolve_world_name(sender: &CommandSender) -> String {
    let (selected, default) = {
        let e = engine();
        (e.selection.world.clone(), e.cfg.default_world.clone())
    };
    if !selected.is_empty() {
        return selected;
    }
    if let Some(w) = sender.world() {
        return w.get_name();
    }
    default
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

struct Help;
impl CommandHandler for Help {
    fn handle(&self, sender: CommandSender, _s: Server, _a: ConsumedArgs) -> Result<i32, CommandError> {
        guard(&sender)?;
        say(&sender, "Chunk pre-generation. Set it up, then /pregen start:");
        for line in [
            "/pregen world [name]        pick the world",
            "/pregen shape square|circle",
            "/pregen center [x z]        centre (no numbers = where you stand)",
            "/pregen radius <blocks>",
            "/pregen border             use the world border",
            "/pregen selection          show the settings",
            "/pregen start | pause | resume | cancel | status",
            "/pregen probe              test that generation works",
        ] {
            sender.send_message(TextComponent::text(line).color_named(NamedColor::Yellow));
        }
        Ok(1)
    }
}

struct WorldHere;
impl CommandHandler for WorldHere {
    fn handle(&self, sender: CommandSender, _s: Server, _a: ConsumedArgs) -> Result<i32, CommandError> {
        guard(&sender)?;
        let Some(w) = sender.world() else {
            return Err(fail("You are not in a world. Use /pregen world <name>."));
        };
        let name = w.get_name();
        {
            let mut e = engine();
            e.selection.world = name.clone();
            e.persist();
        }
        say(&sender, &format!("World set to {name}"));
        Ok(1)
    }
}

struct WorldNamed;
impl CommandHandler for WorldNamed {
    fn handle(&self, sender: CommandSender, server: Server, args: ConsumedArgs) -> Result<i32, CommandError> {
        guard(&sender)?;
        let Some(name) = text_arg(&args, "name") else {
            return Err(fail("Give a world name."));
        };
        let name = name.trim().to_string();
        if !server.has_world(&name) {
            return Err(fail(&format!("No world called '{name}'.")));
        }
        {
            let mut e = engine();
            e.selection.world = name.clone();
            e.persist();
        }
        say(&sender, &format!("World set to {name}"));
        Ok(1)
    }
}

struct ShapeSet(Shape);
impl CommandHandler for ShapeSet {
    fn handle(&self, sender: CommandSender, _s: Server, _a: ConsumedArgs) -> Result<i32, CommandError> {
        guard(&sender)?;
        {
            let mut e = engine();
            e.selection.shape = self.0.as_str().to_string();
            e.persist();
        }
        say(&sender, &format!("Shape set to {}", self.0.as_str()));
        Ok(1)
    }
}

struct CenterHere;
impl CommandHandler for CenterHere {
    fn handle(&self, sender: CommandSender, _s: Server, _a: ConsumedArgs) -> Result<i32, CommandError> {
        guard(&sender)?;
        let Some((x, z)) = sender_block_xz(&sender) else {
            return Err(fail("Only players have a position. Use /pregen center <x> <z>."));
        };
        {
            let mut e = engine();
            e.selection.center = Some((x, z));
            e.persist();
        }
        say(&sender, &format!("Centre set to ({x}, {z})"));
        Ok(1)
    }
}

struct CenterXz;
impl CommandHandler for CenterXz {
    fn handle(&self, sender: CommandSender, _s: Server, args: ConsumedArgs) -> Result<i32, CommandError> {
        guard(&sender)?;
        let (Some(x), Some(z)) = (int_arg(&args, "x"), int_arg(&args, "z")) else {
            return Err(fail("Usage: /pregen center <x> <z>"));
        };
        {
            let mut e = engine();
            e.selection.center = Some((x, z));
            e.persist();
        }
        say(&sender, &format!("Centre set to ({x}, {z})"));
        Ok(1)
    }
}

struct Radius;
impl CommandHandler for Radius {
    fn handle(&self, sender: CommandSender, _s: Server, args: ConsumedArgs) -> Result<i32, CommandError> {
        guard(&sender)?;
        let Some(r) = int_arg(&args, "blocks") else {
            return Err(fail("Usage: /pregen radius <blocks>"));
        };
        let max = engine().cfg.max_radius_blocks;
        if r > max {
            return Err(fail(&format!("The maximum radius is {max} blocks (see config.toml).")));
        }
        {
            let mut e = engine();
            e.selection.radius = r;
            e.persist();
        }
        say(&sender, &format!("Radius set to {r} blocks"));
        Ok(1)
    }
}

struct Border;
impl CommandHandler for Border {
    fn handle(&self, sender: CommandSender, server: Server, _a: ConsumedArgs) -> Result<i32, CommandError> {
        guard(&sender)?;
        let name = resolve_world_name(&sender);
        let Some(world) = server.get_world_by_name(&name) else {
            return Err(fail(&format!("No world called '{name}'.")));
        };
        let border = world.get_world_border();
        let (cx, cz) = (border.get_center_x(), border.get_center_z());
        let half = border.get_diameter() / 2.0;

        let max = engine().cfg.max_radius_blocks;
        let radius = (half.ceil() as i64).clamp(16, max as i64) as i32;
        {
            let mut e = engine();
            e.selection.center = Some((cx.floor() as i32, cz.floor() as i32));
            e.selection.radius = radius;
            e.persist();
        }
        let note = if half.ceil() as i64 > max as i64 {
            format!(" (the border is bigger; capped at the {max}-block maximum)")
        } else {
            String::new()
        };
        say(&sender, &format!("Using the border of {name}: centre ({}, {}), radius {radius}{note}", cx.floor() as i32, cz.floor() as i32));
        Ok(1)
    }
}

struct ShowSelection;
impl CommandHandler for ShowSelection {
    fn handle(&self, sender: CommandSender, _s: Server, _a: ConsumedArgs) -> Result<i32, CommandError> {
        guard(&sender)?;
        let world = resolve_world_name(&sender);
        let sel = engine().selection.clone();
        let shape = Shape::parse(&sel.shape);
        let chunks = area::blocks_to_chunk_radius(sel.radius);
        say(&sender, "Current settings (not started yet):");
        kv(&sender, "World", &world);
        kv(&sender, "Shape", shape.as_str());
        kv(
            &sender,
            "Centre",
            &match sel.center {
                Some((x, z)) => format!("({x}, {z})"),
                None => "world border centre".to_string(),
            },
        );
        kv(&sender, "Radius", &format!("{} blocks", sel.radius));
        kv(&sender, "Chunks", &pretty::thousands(area::total_chunks(shape, chunks)));
        Ok(1)
    }
}

struct Start;
impl CommandHandler for Start {
    fn handle(&self, sender: CommandSender, server: Server, _a: ConsumedArgs) -> Result<i32, CommandError> {
        guard(&sender)?;
        let name = resolve_world_name(&sender);
        let Some(world) = server.get_world_by_name(&name) else {
            return Err(fail(&format!("No world called '{name}'.")));
        };
        let border = world.get_world_border();
        let border_center = (border.get_center_x().floor() as i32, border.get_center_z().floor() as i32);

        let result = engine().start_job(name, border_center);
        match result {
            Ok(msg) => {
                say(&sender, &msg);
                say(&sender, "Use /pregen status to watch it, /pregen pause to stop.");
                Ok(1)
            }
            Err(e) => Err(fail(&e)),
        }
    }
}

struct Pause;
impl CommandHandler for Pause {
    fn handle(&self, sender: CommandSender, _s: Server, _a: ConsumedArgs) -> Result<i32, CommandError> {
        guard(&sender)?;
        let r = engine().pause();
        r.map(|m| { say(&sender, &m); 1 }).map_err(|e| fail(&e))
    }
}

struct Resume;
impl CommandHandler for Resume {
    fn handle(&self, sender: CommandSender, _s: Server, _a: ConsumedArgs) -> Result<i32, CommandError> {
        guard(&sender)?;
        let r = engine().resume();
        r.map(|m| { say(&sender, &m); 1 }).map_err(|e| fail(&e))
    }
}

struct Cancel;
impl CommandHandler for Cancel {
    fn handle(&self, sender: CommandSender, _s: Server, _a: ConsumedArgs) -> Result<i32, CommandError> {
        guard(&sender)?;
        let r = engine().cancel();
        r.map(|m| { say(&sender, &m); 1 }).map_err(|e| fail(&e))
    }
}

struct Status;
impl CommandHandler for Status {
    fn handle(&self, sender: CommandSender, server: Server, _a: ConsumedArgs) -> Result<i32, CommandError> {
        guard(&sender)?;
        // Ask the server first, so no lock is held during the host calls.
        let (tps, mspt) = (server.get_tps(), server.get_mspt());
        let lines = engine().status_lines(tps, mspt);
        say(&sender, "Status");
        for (k, v) in lines {
            kv(&sender, &k, &v);
        }
        Ok(1)
    }
}

/// Answers the one question this plugin depends on: does `World::get_chunk` actually
/// load or generate a chunk that has never been visited?
struct Probe;
impl CommandHandler for Probe {
    fn handle(&self, sender: CommandSender, server: Server, _a: ConsumedArgs) -> Result<i32, CommandError> {
        guard(&sender)?;
        let name = resolve_world_name(&sender);
        let Some(world) = server.get_world_by_name(&name) else {
            return Err(fail(&format!("No world called '{name}'.")));
        };

        // A chunk just outside the selected area, so it is very likely never visited.
        let (center, radius) = {
            let e = engine();
            (e.selection.center.unwrap_or((0, 0)), e.selection.radius)
        };
        let cx = area::block_to_chunk(center.0) + area::blocks_to_chunk_radius(radius) + 64;
        let cz = area::block_to_chunk(center.1);

        let before = world.is_chunk_loaded(cx, cz);
        let t = Instant::now();
        let got = world.get_chunk(cx, cz).is_some();
        let ms = t.elapsed().as_millis();
        let after = world.is_chunk_loaded(cx, cz);

        say(&sender, &format!("Probe of chunk ({cx}, {cz}) in {name}:"));
        kv(&sender, "Loaded before", &before.to_string());
        kv(&sender, "get_chunk returned a chunk", &got.to_string());
        kv(&sender, "Time", &format!("{ms} ms"));
        kv(&sender, "Loaded after", &after.to_string());
        let verdict = match (got, ms) {
            (true, m) if m >= 1 => "Looks good: the server produced a chunk on demand. Pregen will work.",
            (true, _) => "A chunk came back instantly. Either it already existed, or generation is very fast. Run pregen on a small radius and check that the region files grow.",
            (false, _) => "No chunk came back. It may be outside the world border, or get_chunk only returns chunks that are already loaded. See the README.",
        };
        say(&sender, verdict);
        Ok(1)
    }
}

// ---------------------------------------------------------------------------
// Registration
// ---------------------------------------------------------------------------

fn literal(name: &str, handler: impl CommandHandler + 'static) -> CommandNode {
    CommandNode::literal(name).execute(handler)
}

pub fn register(context: &Context) {
    let int_any = || ArgumentType::Integer((None, None));

    let world = CommandNode::literal("world").execute(WorldHere).then(
        CommandNode::argument("name", ArgumentType::String(StringType::Greedy)).execute(WorldNamed),
    );
    let shape = CommandNode::literal("shape")
        .then(literal("square", ShapeSet(Shape::Square)))
        .then(literal("circle", ShapeSet(Shape::Circle)));
    let center = CommandNode::literal("center").execute(CenterHere).then(
        CommandNode::argument("x", int_any())
            .then(CommandNode::argument("z", int_any()).execute(CenterXz)),
    );
    let radius = CommandNode::literal("radius").then(
        CommandNode::argument("blocks", ArgumentType::Integer((Some(16), Some(2_000_000))))
            .execute(Radius),
    );

    let names = ["pregen".to_string(), "pg".to_string()];
    let command = Command::new(&names, "Pre-generate chunks around a point")
        .execute(Help)
        .then(world)
        .then(shape)
        .then(center)
        .then(radius)
        .then(literal("border", Border))
        .then(literal("selection", ShowSelection))
        .then(literal("start", Start))
        .then(literal("pause", Pause))
        .then(literal("resume", Resume))
        .then(literal("continue", Resume))
        .then(literal("cancel", Cancel))
        .then(literal("status", Status))
        .then(literal("progress", Status))
        .then(literal("probe", Probe));

    context.register_command(command, PERMISSION_NODE);
}
