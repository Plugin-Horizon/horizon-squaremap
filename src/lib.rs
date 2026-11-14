//! squaremap as an arismoko native mod (trait mod on `mod_api`). The renderer, tile and JSON
//! writers, commands and the internal web server are the unchanged `horizon-squaremap` crate
//! (`core/`); this file only wires it to hooks and host methods. See PORT_NOTES.md for what is
//! missing on arismoko and what each fallback loses.
//!
//! The map starts in `server_started` only once the block-state and biome tables can be read
//! (`render_tables`, TODO(arismoko-api: block_state_table)). Until then the commands answer, the
//! config is read and the web server does not start.
//!
//! Config: `config/squaremap-server.toml` (registered as a Server config; a world's
//! `serverconfig/squaremap-server.toml` wins). The web server binds 127.0.0.1 by default.
use horizon_squaremap as sm;
use mod_api::{
    BlockChangedEvent, BlockMetadata, ChunkLoadEvent, ChunkUnloadEvent, Command, CommandArgumentValue, CommandSource,
    HostBlockStates,
    Config, ConfigKind, ConfigLoadingEvent, ConfigReloadingEvent, Flow, HostSide, Hooks, LivingRef, PlayerCommandEvent,
    RegisterCommandsEvent, RegisterConfigsEvent, RunModCommandEvent, ServerHost, ServerStartedEvent,
    ServerStoppingEvent, ServerTickEvent, ServerTickPhase, Suggestion, TabCompleteEvent, Text,
};
use mod_api::text::{TextClick, TextHover, TextStyle};
use std::path::PathBuf;

const ID: &str = "squaremap";

/// The dimensions squaremap draws, by their registry ids.
const DIMENSIONS: [&str; 3] = ["minecraft:overworld", "minecraft:the_nether", "minecraft:the_end"];

fn log(line: &str) {
    eprintln!("[squaremap] {line}");
}

/// One `key=value` line of `server.properties` (the key is never a secret here: `level-name`,
/// `max-players`).
fn property(key: &str) -> Option<String> {
    let text = std::fs::read_to_string("server.properties").ok()?;
    text.lines()
        .find_map(|line| line.strip_prefix(key)?.strip_prefix('=').map(|v| v.trim().to_owned()))
        .filter(|v| !v.is_empty())
}

/// The world folder, as the Paper plugin reads `level-name`.
fn level_dir() -> PathBuf {
    property("level-name").unwrap_or_else(|| "world".into()).into()
}

/// `max-players`, for the player count squaremap shows: the server's live limit, the file when none.
fn max_players(host: &dyn ServerHost) -> usize {
    host.server_max_players()
        .map(|n| n as usize)
        .or_else(|| property("max-players").and_then(|v| v.parse().ok()))
        .unwrap_or(20)
}

/// The game's resource folder: the engine's `--resources <dir>` argument (or `HORIZON_RESOURCES`).
fn resources_dir() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("HORIZON_RESOURCES") {
        return Some(PathBuf::from(p));
    }
    let raw = std::fs::read("/proc/self/cmdline").ok()?;
    let args: Vec<String> = raw.split(|b| *b == 0).map(|a| String::from_utf8_lossy(a).into_owned()).collect();
    args.iter()
        .enumerate()
        .find_map(|(i, a)| match a.strip_prefix("--resources=") {
            Some(v) => Some(v.to_owned()),
            None => (a.as_str() == "--resources").then(|| args.get(i + 1).cloned()).flatten(),
        })
        .map(PathBuf::from)
}

/// The block-state table, biome table and world env the core starts with. arismoko has no registry
/// listing, so the block ids come from the resource folder's `registries.json` and the biomes from its
/// worldgen files (`sm::vanilla`); the host answers each block's states and their map colours.
/// Cause of "not started" before this: the function returned None unconditionally, so `server_started`
/// never reached `sm::start` (no hook, level-name or capability problem).
// TODO(arismoko-api: biome_zoom): the biome zoom (`quart_at_block`) and the swamp grass noise the
//   core uses for chunks it reads from the region files: `WorldEnv::plain()` (unfuzzed quart cell,
//   flat swamp grass) stands in.
fn render_tables(host: &dyn ServerHost) -> Option<(Vec<sm::RawState>, Vec<sm::RawBiome>, sm::WorldEnv)> {
    let dir = resources_dir()?;
    let mut states = Vec::new();
    // The public host path (`block_key`, `block_states_of` and `block_state_map_color` only exist on the
    // hidden backend, arismoko/horizon#3927): every key by its registered name, then the metadata view.
    let meta = host.block_states();
    let keys: std::collections::HashMap<String, _> = host
        .block_keys()
        .into_iter()
        .filter_map(|key| meta.name(key).ok().map(|n| (n.to_owned(), key)))
        .collect();
    for name in sm::vanilla::block_names(&dir) {
        let Some(&key) = keys.get(&name).or_else(|| keys.get(&format!("minecraft:{name}"))) else { continue };
        let Some(range) = meta.states_of(key) else { continue };
        let default_map_color = host
            .block_state_default(key)
            .and_then(|s| meta.map_color(s))
            .unwrap_or(0);
        for state in range.iter() {
            let Some(text) = host.block_state_string(state) else { continue };
            let (block, properties) = sm::vanilla::parse_state(&text);
            let prop = |k: &str| properties.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str());
            let age = prop("age").and_then(|v| v.parse::<u8>().ok());
            let fluid = if block == "minecraft:lava" {
                sm::Fluid::Lava
            } else if block == "minecraft:water"
                || prop("waterlogged") == Some("true")
                || matches!(
                    block.as_str(),
                    "minecraft:bubble_column" | "minecraft:kelp" | "minecraft:kelp_plant" | "minecraft:seagrass" | "minecraft:tall_seagrass"
                )
            {
                sm::Fluid::Water
            } else {
                sm::Fluid::None
            };
            states.push(sm::RawState {
                name: block,
                properties,
                age,
                map_color: meta.map_color(state).unwrap_or(0),
                default_map_color,
                map_rgb: None,
                fluid,
            });
        }
    }
    let biomes = sm::vanilla::biomes(&dir);
    if states.is_empty() || biomes.is_empty() {
        return None;
    }
    Some((states, biomes, sm::WorldEnv::plain()))
}

/// The border of a dimension's level, when it is loaded.
fn level_border(host: &mut dyn ServerHost, dimension: &str) -> Option<[f64; 4]> {
    let level = host.level(dimension)?;
    level.world_border(&*host)
}

/// The players squaremap draws. Spectators are left out with the `spectators` hide rule.
// TODO(arismoko-api: living_effects): the `invisible` hide rule (invisibility effect) is not applied.
// TODO(arismoko-api: item_in_tag): the `map-invisibility-equipment` hide rule is not applied.
fn publish_players(host: &dyn ServerHost) {
    let mut list = Vec::new();
    for p in host.players().server_players() {
        let (Some(name), Some(world), Some(position)) =
            (p.name(host), p.dimension(host), p.entity().position(host))
        else {
            continue;
        };
        let world = world.to_string();
        if sm::hide_rules(&world).spectators && p.is_spectator(host) == Some(true) {
            continue;
        }
        let entity = p.entity();
        let living = LivingRef::new(entity);
        list.push(sm::Player {
            uuid: p.uuid(),
            name,
            world,
            position,
            yaw: entity.rotation(host).map_or(0.0, |r| r[0]),
            health: living.health(host).unwrap_or(0.0),
            // `Attributes.ARMOR` with every modifier, as the game answers it.
            armor: living.attribute_value(host, "minecraft:armor").map(|v| v.floor() as i32),
            // The nickname, where one is set: the closest arismoko has to the display name.
            display_name: p.nickname(host).flatten(),
        });
    }
    sm::publish_players(&list, max_players(host));
}

/// The players the command line may name (`/squaremap hide <player>`).
fn online(host: &dyn ServerHost) -> Vec<(u128, String)> {
    host.players()
        .server_players()
        .into_iter()
        .filter_map(|p| Some((p.uuid(), p.name(host)?)))
        .collect()
}

/// squaremap's `[squaremap]` prefix (`Messages.COMMAND_PREFIX`): a gradient on the word, and a
/// hover and a click (`/<label> help`) on each piece.
fn prefix() -> Vec<Text> {
    const FROM: (u32, u32, u32) = (0xC0, 0x28, 0xFF);
    const TO: (u32, u32, u32) = (0x5B, 0x00, 0xFF);
    let click = TextClick::RunCommand(format!("/{} help", sm::commands::main_label()));
    let hover = Some("Click for help");
    let mut out = vec![styled("[", Some(0xFFFFFF), Some(click.clone()), hover)];
    let name = "squaremap";
    let last = (name.chars().count() - 1) as f64;
    for (i, c) in name.chars().enumerate() {
        let t = i as f64 / last;
        let mix = |a: u32, b: u32| (a as f64 + (b as f64 - a as f64) * t).round() as u32;
        let color = (mix(FROM.0, TO.0) << 16) | (mix(FROM.1, TO.1) << 8) | mix(FROM.2, TO.2);
        out.push(styled(c.to_string().as_str(), Some(color), Some(click.clone()), hover));
    }
    out.push(styled("]", Some(0xFFFFFF), Some(click.clone()), hover));
    out.push(styled(" ", Some(0xFFFFFF), None, None));
    out
}

/// One piece of text with squaremap's colour, click and hover.
fn styled(text: &str, color: Option<u32>, click: Option<TextClick>, hover: Option<&str>) -> Text {
    let mut style = TextStyle::default();
    // TODO(arismoko-api: text_colour_format): colours are sent as `#RRGGBB`; check Horizon accepts that form.
    style.color = color.map(|c| format!("#{c:06X}"));
    style.click = click;
    style.hover = hover.map(|h| TextHover::Text(Box::new(Text::literal(h))));
    Text::literal(text).with_style(style)
}

/// One of squaremap's lines as Horizon text: the prefix on errors, then its parts.
fn line_text(line: &sm::commands::Line) -> Text {
    let mut text = Text::literal("");
    if line.prefixed {
        for piece in prefix() {
            text = text.append(piece);
        }
    }
    for part in &line.parts {
        let click = part.click.as_ref().map(|c| match c {
            sm::commands::Click::Run(command) => TextClick::RunCommand(command.clone()),
            sm::commands::Click::Open(url) => TextClick::OpenUrl(url.clone()),
        });
        text = text.append(styled(&part.text, part.color, click, part.hover));
    }
    text
}

#[derive(Default)]
struct Mod;

#[mod_api::horizon_mod]
impl Hooks for Mod {
    fn register_configs(&self, _: HostSide, event: &mut RegisterConfigsEvent<'_>) {
        // `config/squaremap-server.toml`, written with the default when missing.
        event.configs.push(Config::new(ID, ConfigKind::Server, sm::default_config()));
    }

    fn config_loading(&self, _: HostSide, event: &mut ConfigLoadingEvent<'_>) {
        if event.config.mod_id == ID {
            sm::config_loaded(&event.config.text);
        }
    }

    fn config_reloading(&self, _: HostSide, event: &mut ConfigReloadingEvent<'_>) {
        if event.config.mod_id == ID {
            sm::config_loaded(&event.config.text);
        }
    }

    fn server_started(&self, host: &mut dyn ServerHost, _: &mut ServerStartedEvent) {
        match render_tables(&*host) {
            Some((states, biomes, env)) => match sm::start(&level_dir(), states, biomes, env) {
                Ok(()) => log("map started"),
                Err(e) => log(&format!("not started: {e}")),
            },
            None => log(
                "not started: block-state or biome tables could not be built (resource folder with \
                 registries.json and data/minecraft/worldgen/biome not found, or the host lists no states)",
            ),
        }
    }

    fn server_stopping(&self, _: &mut dyn ServerHost, _: &mut ServerStoppingEvent) {
        sm::world_closed();
    }

    fn server_tick(&self, host: &mut dyn ServerHost, event: &mut ServerTickEvent<'_>) {
        if !matches!(event.phase, ServerTickPhase::ServerTickEnd { world_running: true, .. }) || !sm::running() {
            return;
        }
        for dimension in DIMENSIONS {
            let Some(w) = sm::world_index(dimension) else { continue };
            // TODO(arismoko-api: chunk_snapshot): loaded chunks are not snapshotted. Due chunks are
            // handed to the core as "not loaded", so they are drawn from the region files instead.
            for (cx, cz) in sm::due_chunks(w) {
                sm::not_loaded(w, cx, cz);
            }
            if sm::markers_due(w) {
                // TODO(arismoko-api: level_world_spawn): no spawn marker (`None`).
                if let Some(border) = level_border(host, dimension) {
                    sm::publish_markers(w, None, border);
                }
            }
        }
        if sm::players_due() {
            publish_players(&*host);
        }
    }

    fn chunk_load(&self, _: &mut dyn ServerHost, e: &mut ChunkLoadEvent<'_>) {
        if let Some(w) = sm::world_index(e.dimension) {
            sm::chunk_loaded(w, e.chunk[0], e.chunk[1], e.new);
        }
    }

    fn chunk_unload(&self, _: &mut dyn ServerHost, e: &mut ChunkUnloadEvent<'_>) {
        if let Some(w) = sm::world_index(e.dimension) {
            sm::chunk_unloaded(w, e.chunk[0], e.chunk[1]);
        }
    }

    fn block_changed(&self, _: &dyn mod_api::LevelHost, e: &mut BlockChangedEvent) {
        if let Some(w) = sm::world_index(e.dimension.key().id()) {
            sm::block_changed(w, e.pos.x, e.pos.z);
        }
    }

    fn register_commands(&self, _: &mut dyn ServerHost, event: &mut RegisterCommandsEvent<'_>) {
        for (usage, level) in sm::commands::usages() {
            // squaremap's `word?` (the client asks for world names) is plain `word` here; the
            // names come from `tab_complete` below.
            // Each subcommand path carries squaremap's node (`squaremap.command.fullrender`); the level
            // decides when no permissions mod answers. The bare label and `confirm` have none.
            let command = Command::op(&usage.replace("word?", "word"), level);
            event.commands.push(match sm::commands::permission_node(&usage) {
                Some(node) => command.with_node(&node),
                None => command,
            });
        }
    }

    fn player_command(&self, host: &mut dyn ServerHost, event: &mut PlayerCommandEvent<'_>) -> Flow {
        // A player without the sub-command's node (`squaremap.command.<sub>`, operators by default) gets
        // squaremap's no-permission line instead of the engine's "Unknown command".
        if !sm::commands::is_ours(event.command) {
            return Flow::Continue;
        }
        let Some(node) = sm::commands::permission_node(event.command) else { return Flow::Continue };
        let allowed = host.player(event.player).is_some_and(|p| p.has_permission(&*host, &node));
        if allowed {
            return Flow::Continue;
        }
        event.replies.push(Text::literal("\u{a7}cI'm sorry, but you do not have permission to perform this command. Please contact the server administrators if you believe that this is in error."));
        event.cancelled = true;
        Flow::Cancel
    }

    fn run_mod_command(&self, host: &mut dyn ServerHost, event: &mut RunModCommandEvent<'_>) -> Flow {
        // The command line as squaremap parses it: the literals, then the arguments in order.
        let mut words = event.command.literals.clone();
        for argument in &event.command.arguments {
            match &argument.value {
                CommandArgumentValue::String(s) => words.push(s.clone()),
                CommandArgumentValue::Integer(i) => words.push(i.to_string()),
                CommandArgumentValue::Player(p) => words.extend(p.name(&*host)),
                _ => {}
            }
        }
        let line = words.join(" ");
        if !sm::commands::is_ours(&line) {
            return Flow::Continue;
        }
        let sender = match &event.source {
            CommandSource::Player { player, permission } => {
                let (Some(name), Some(world), Some(at)) =
                    (player.name(&*host), player.dimension(&*host), player.entity().position(&*host))
                else {
                    return Flow::Continue;
                };
                // The usage's node already gated the command (`with_node`); the core is told 4 when the node
                // (or, without one, the level) lets the player through. `hide`/`show` for others asks its own node.
                let allowed = match sm::commands::permission_node(&line) {
                    Some(node) => player.has_permission(&*host, &node),
                    None => permission.level() >= sm::commands::OP_LEVEL,
                };
                let op = if allowed { 4 } else { 0 };
                let others = sm::commands::others_node(&line).is_some_and(|n| player.has_permission(&*host, &n));
                sm::commands::Sender {
                    player: Some(sm::commands::SenderPlayer {
                        uuid: player.uuid(),
                        name,
                        world: world.to_string(),
                        x: at[0],
                        z: at[2],
                    }),
                    permission: op,
                    others,
                }
            }
            CommandSource::Console { .. } => {
                sm::commands::Sender { player: None, permission: 4, others: true }
            }
            _ => return Flow::Continue,
        };
        let lines = sm::commands::command(&line, &sender, &online(&*host));
        for l in &lines {
            let _ = host.commands().feedback(line_text(l));
        }
        Flow::Cancel
    }

    fn tab_complete(&self, _: &mut dyn ServerHost, e: &mut TabCompleteEvent<'_>) {
        // TODO(arismoko-api: verify): event field names `command`, `range`, `suggestions` are assumed.
        for text in sm::commands::world_completions(e.command) {
            if !e.suggestions.iter().any(|s| s.text == text) {
                e.suggestions.push(Suggestion::new((e.range[0], e.range[1]), text));
            }
        }
    }
}
