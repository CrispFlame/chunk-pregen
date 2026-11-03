# Chunk Pregen for Pumpkin

Pre-generate the world around a point so players never wait on new terrain.
Like Chunky: shapes, a centre, a radius, progress, ETA, chunks-per-second,
pause/resume, and it survives restarts.

## Commands (operators only)

| Command | What it does |
|---|---|
| `/pregen world [name]` | Pick the world. No name = the world you are standing in |
| `/pregen shape square\|circle` | Area shape |
| `/pregen center` | Centre on where you stand |
| `/pregen center <x> <z>` | Centre on block coordinates |
| `/pregen radius <blocks>` | Radius in blocks (16 .. `max_radius_blocks`) |
| `/pregen border` | Use the world border's centre and size |
| `/pregen selection` | Show the settings and how many chunks they make |
| `/pregen start` | Begin |
| `/pregen status` | State, %, chunks/s, ETA, elapsed, TPS |
| `/pregen pause` / `resume` / `cancel` | Control the run |
| `/pregen probe` | **Run this first.** Tests that chunk generation works here |

`/pg` is a shortcut for `/pregen`. If you never set a centre, it uses the world border's centre.

## Build and install

```bash
rustup target add wasm32-wasip2
cargo build --release
```

Copy `target/wasm32-wasip2/release/chunk_pregen.wasm` into the server's `plugins/` folder.
It writes `config.toml` and `state.json` into its own data folder on first run.

## FIRST: run `/pregen probe`

The whole plugin rests on one call, `World::get_chunk(x, z)`. Pumpkin's API docs
describe it only as "gets the chunk at these coordinates", so it is not guaranteed to
*generate* a chunk nobody has visited. `/pregen probe` asks for a far-away chunk and
tells you whether one came back, how long it took, and whether it is now loaded.

- **Chunk returned, took a few ms or more:** it generates on demand. Pregen will work.
- **Nothing returned:** it only returns already-loaded chunks. Pregen cannot work with this API yet. See "If probe says no" below.

Then do a small real test (`/pregen radius 64`, `/pregen start`) and check that the
region files in the world folder grow.

## How it stays out of the way

Each tick it works for at most `budget_ms_per_tick` (default 20 of the 50 ms available).
If the server drops below `min_tps` or above `max_mspt`, it pauses itself and resumes when
the server recovers. Progress is saved every ~5 s, and the world is saved every
`save_world_every_chunks`. After a restart, a run waits for `/pregen resume` (or set
`auto_resume = true`).

## How the pieces fit (for learning)

| File | Role | Needs Pumpkin? |
|---|---|---|
| `src/area.rs` | The outward ring walk, circle test, exact chunk counts | No |
| `src/pretty.rs` | `1h 02m 05s`, `33.3%`, ETA | No |
| `src/config.rs` | Reads `config.toml` | Only for file access |
| `src/engine.rs` | Job state, the per-tick loop, throttling, saving | Yes |
| `src/commands.rs` | The `/pregen` command tree | Yes |
| `src/lib.rs` | Plugin entry: permissions, scheduler, commands | Yes |

`area.rs` and `pretty.rs` have unit tests. Run them natively:

```bash
cargo test --target x86_64-unknown-linux-gnu
```

(change the target to your own machine's if it differs, e.g. `aarch64-apple-darwin`).

## What is verified and what is not

I could not compile this (no Rust toolchain in my environment), so treat the first build
as a test. These come from Pumpkin's published API source and WIT files:
the `Plugin` trait and `PluginMetadata` fields, the scheduler functions, the command tree
(`CommandNode::literal/argument/then/execute`), argument types, `Server::get_world_by_name`,
`get_tps`, `get_mspt`, `save_all`, `broadcast`, `World::get_chunk`, `is_chunk_loaded`,
`get_world_border`, and the data-folder permissions.

These I am inferring, and are the first places a compile error may appear:

1. `sender.position()` fields `.x` / `.z` (`commands.rs`, function `sender_block_xz`)
2. `TextComponent::color_named(..)` and `add_child(..)` returning the component for chaining
3. `NamedColor`'s module path (`text::NamedColor`)

Each is one or two lines. Paste me the compiler error and I will fix it.

## If probe says no

Then `get_chunk` cannot trigger generation, and a plugin has no other documented way to
request it. Options: ask the Pumpkin developers (Discord / GitHub) for a "load or generate
chunk" function, or use a method such as teleporting a hidden player along a path so the
server loads chunks naturally. Tell me which and I will adapt the engine, since all
chunk requests go through one line in `engine.rs`.
