// squaremap for Horizon: the game side. A support module in `game.rs` (so it reaches the
// game's `native_*` helpers); the hooks' helpers only call into it. Everything here runs on
// the server thread, except `Snap`, which the render thread reads.
//
// Loaded chunks of every level are drawn live from `native_chunk_snapshot`, coloured with
// `native_block_map_color` and `native_biome_tint`; the core reads region files only for
// chunks that are not loaded (full and radius renders, biome cells past a snapshot).
use super::Game;
use horizon_client::biome::GrassModifier;
use horizon_client::blocks::Fluid;
use horizon_client::mod_api::{EquipmentSlot, NativeChunkSnapshot, ServerTickPhase, WorldPhase};
use horizon_core::position::BlockPos;
use horizon_squaremap as sm;
use horizon_world::registry::BlockStateId;

const DIMENSIONS: [&str; 3] = ["minecraft:overworld", "minecraft:the_nether", "minecraft:the_end"];

/// The swamp's dry grass (`GrassColorModifier.SWAMP` where the noise is high), for a column
/// past the snapshot's grid.
const SWAMP_FALLBACK: u32 = 0x6A7039;

/// A chunk and its eight neighbours, as they were when the tick reached it.
struct Snap {
    cx: i32,
    cz: i32,
    chunks: [Option<NativeChunkSnapshot>; 9],
    min_y: i32,
    top_y: i32,
    ceiling: bool,
    swamp: Option<sm::SwampGrid>,
}

impl Snap {
    fn at(&self, x: i32, z: i32) -> Option<&NativeChunkSnapshot> {
        let (dx, dz) = ((x >> 4) - self.cx, (z >> 4) - self.cz);
        if dx.abs() > 1 || dz.abs() > 1 {
            return None;
        }
        self.chunks[((dz + 1) * 3 + dx + 1) as usize].as_ref()
    }
    fn centre(&self) -> &NativeChunkSnapshot {
        self.chunks[4].as_ref().expect("a snap is only made with its centre")
    }
}

impl sm::ChunkView for Snap {
    fn chunk(&self) -> (i32, i32) {
        (self.cx, self.cz)
    }
    fn min_y(&self) -> i32 {
        self.min_y
    }
    fn top_y(&self) -> i32 {
        self.top_y
    }
    fn has_ceiling(&self) -> bool {
        self.ceiling
    }
    fn surface(&self, x: i32, z: i32) -> Option<i32> {
        self.at(x, z)?.surface(x, z)
    }
    fn state(&self, x: i32, y: i32, z: i32) -> u32 {
        self.at(x, z).and_then(|c| c.block_state(x, y, z)).map_or(u32::MAX, |s| s.0)
    }
    fn quart_at_block(&self, x: i32, y: i32, z: i32) -> [i32; 3] {
        // A cell outside the snapshot is read from the region files by the core, else
        // the nearest cell of the chunk itself.
        self.centre().biome_cell(x, y, z)
    }
    fn noise_biome(&self, qx: i32, qy: i32, qz: i32) -> Option<u16> {
        self.at(qx << 2, qz << 2)?.noise_biome(qx, qy, qz)
    }
    fn swamp_grass(&self, _biome: u16, x: i32, z: i32) -> u32 {
        self.swamp.as_ref().map_or(SWAMP_FALLBACK, |g| g.get(x, z))
    }
}

thread_local! {
    /// Biome ids with the swamp's grass modifier, from the world's biome table.
    static SWAMPS: std::cell::RefCell<Vec<u16>> = const { std::cell::RefCell::new(Vec::new()) };
}

fn biome_table(game: &Game<'_>) -> std::sync::Arc<horizon_client::biome::Biomes> {
    game.world_biomes.clone().unwrap_or_else(|| game.res.biomes().clone())
}

/// The world opened or closed.
pub(super) fn world_phase(game: &mut Game<'_>, phase: WorldPhase) {
    match phase {
        WorldPhase::WorldOpenReady => {
            let Some(dir) = game.native_world_dir() else { return };
            let registry = &game.pack.assets.registry;
            let facts = game.res.facts();
            let mut states = Vec::with_capacity(registry.len());
            for id in 0..registry.len() as u32 {
                let state = BlockStateId(id);
                let (name, age, properties) = registry.state(state).map_or((String::new(), None, Vec::new()), |s| {
                    let properties: Vec<(String, String)> =
                        s.properties.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
                    let age = properties.iter().find(|(k, _)| k == "age").and_then(|(_, v)| v.parse().ok());
                    (s.name.clone(), age, properties)
                });
                // `BlockState.getMapColor` as the game answers it: (id, col).
                let (map_color, map_rgb) = game.native_block_map_color(state).unwrap_or((0, 0));
                let default_map_color = registry
                    .default_state(&name)
                    .and_then(|d| game.native_block_map_color(d))
                    .map_or(map_color, |(id, _)| id);
                let fluid = match facts.get(state).fluid {
                    Fluid::None => sm::Fluid::None,
                    Fluid::Water => sm::Fluid::Water,
                    Fluid::Lava => sm::Fluid::Lava,
                };
                states.push(sm::RawState {
                    name,
                    properties,
                    age,
                    map_color,
                    default_map_color,
                    map_rgb: Some(map_rgb),
                    fluid,
                });
            }
            // Biome ids as the snapshots give them (`native_biome_name`), their colours from
            // `native_biome_tint`; the climate and modifier, which the tint doesn't carry, from
            // the world's biome table.
            let table = biome_table(game);
            let mut raw = Vec::new();
            let mut swamps = Vec::new();
            for id in 0..=u16::MAX {
                let Some(name) = game.native_biome_name(id).map(str::to_owned) else { break };
                let tint = game.native_biome_tint(id, 0, 0);
                let b = table.get(id);
                let modifier = match b.modifier {
                    GrassModifier::None => sm::GrassModifier::None,
                    GrassModifier::DarkForest => sm::GrassModifier::DarkForest,
                    GrassModifier::Swamp => sm::GrassModifier::Swamp,
                };
                if modifier == sm::GrassModifier::Swamp {
                    swamps.push(id);
                }
                raw.push(sm::RawBiome {
                    name,
                    temperature: b.temperature,
                    downfall: b.downfall,
                    grass: tint.as_ref().map_or(b.grass, |t| t.grass),
                    foliage: tint.as_ref().map_or(b.foliage, |t| t.foliage),
                    dry_foliage: tint.as_ref().map_or(b.dry_foliage, |t| t.dry_foliage),
                    water: tint.as_ref().map_or(b.water, |t| t.water),
                    modifier: if tint.is_some() { sm::grass_modifier_after_tint(modifier) } else { modifier },
                });
            }
            SWAMPS.with(|s| *s.borrow_mut() = swamps);
            // For chunks the core reads from the region files (off the server thread, where
            // the helpers can't be asked): the game's biome zoom and swamp grass noise.
            let zoom = game.streaming.biome_zoom;
            let grass = table.clone();
            let env = sm::WorldEnv {
                zoom: Box::new(move |x, y, z| zoom.quart_at_block(BlockPos::new(x, y, z))),
                swamp_grass: Box::new(move |b, x, z| grass.grass(b, f64::from(x), f64::from(z))),
            };
            if let Err(e) = sm::start(&dir, states, raw, env) {
                eprintln!("[squaremap] not started: {e}");
            }
        }
        WorldPhase::WorldCloseFinal | WorldPhase::WorldCloseFailed => sm::world_closed(),
        _ => {}
    }
}

/// At the end of each server tick: snapshot every level's due chunks, publish markers and
/// players.
pub(super) fn server_ticked(game: &mut Game<'_>, phase: ServerTickPhase) {
    if !matches!(phase, ServerTickPhase::ServerTickEnd { .. }) || !sm::running() {
        return;
    }
    for dimension in DIMENSIONS {
        level(game, dimension);
    }
    if sm::players_due() {
        publish_players(game);
    }
}

fn level(game: &mut Game<'_>, dimension: &str) {
    let Some(w) = sm::world_index(dimension) else { return };
    let due = sm::due_chunks(w);
    let mut shaped = false;
    for (cx, cz) in due {
        // Not loaded (or its level isn't open): drawn when it loads again, or by a render
        // from the region files.
        let Some(centre) = game.native_chunk_snapshot(dimension, [cx, cz]) else {
            sm::not_loaded(w, cx, cz);
            continue;
        };
        let (min_y, top_y) = centre.height();
        let ceiling = centre.has_ceiling();
        if !shaped {
            sm::level_shape(w, min_y, top_y, ceiling);
            shaped = true;
        }
        let mut chunks: [Option<NativeChunkSnapshot>; 9] = std::array::from_fn(|i| {
            let (dx, dz) = (i as i32 % 3 - 1, i as i32 / 3 - 1);
            if (dx, dz) == (0, 0) { None } else { game.native_chunk_snapshot(dimension, [cx + dx, cz + dz]) }
        });
        chunks[4] = Some(centre);
        let swamp = has_swamp(&chunks, min_y, top_y).then(|| {
            let id = SWAMPS.with(|s| s.borrow().first().copied()).unwrap_or(0);
            // The swamp modifier ignores the biome's own grass, so one swamp id answers for all.
            sm::SwampGrid::take(cx, cz, |x, z| game.native_biome_tint(id, x, z).map_or(SWAMP_FALLBACK, |t| t.grass))
        });
        sm::submit(w, Box::new(Snap { cx, cz, chunks, min_y, top_y, ceiling, swamp }));
    }
    if sm::markers_due(w) {
        let spawn = game.level.world_spawn.map(|[x, _, z]| [x, z]);
        let border = game.native_world_border(dimension).unwrap_or_else(|| game.border.bounds());
        sm::publish_markers(w, spawn, border);
    }
}

/// Whether any noise cell of the snapshots is a swamp biome.
fn has_swamp(chunks: &[Option<NativeChunkSnapshot>; 9], min_y: i32, top_y: i32) -> bool {
    SWAMPS.with(|s| {
        let swamps = s.borrow();
        if swamps.is_empty() {
            return false;
        }
        chunks.iter().flatten().any(|c| {
            let [cx, cz] = c.chunk();
            (0..4).any(|qx| {
                (0..4).any(|qz| {
                    ((min_y >> 2)..(top_y >> 2)).any(|qy| {
                        c.noise_biome(cx * 4 + qx, qy, cz * 4 + qz).is_some_and(|b| swamps.contains(&b))
                    })
                })
            })
        })
    })
}

const ARMOUR_SLOTS: [EquipmentSlot; 4] =
    [EquipmentSlot::Head, EquipmentSlot::Chest, EquipmentSlot::Legs, EquipmentSlot::Feet];

/// The player markers, from the player reads (`native_server_players`, `native_effects`,
/// `native_player_equipment`, `native_attribute`, `native_max_players`).
fn publish_players(game: &mut Game<'_>) {
    let players = game.native_server_players();
    let mut list = Vec::new();
    for p in players {
        let hide = sm::hide_rules(&p.dimension);
        // Game mode 3: spectator.
        if hide.spectators && p.game_mode == 3 {
            continue;
        }
        if hide.invisible
            && game.native_effects(p.uuid).is_some_and(|e| e.iter().any(|e| e.id == "minecraft:invisibility"))
        {
            continue;
        }
        let worn: Vec<_> = ARMOUR_SLOTS.iter().filter_map(|s| game.native_player_equipment(p.uuid, *s)).collect();
        if hide.map_invisibility_equipment
            && worn.iter().any(|s| game.native_server_item_in_tag(s, "minecraft:map_invisibility_equipment") == Ok(true))
        {
            continue;
        }
        // `Attributes.ARMOR` with every modifier, where the player has a body to ask.
        let armor = game
            .native_attribute(p.uuid, "minecraft:armor")
            .map(|(_, value)| value.floor() as i32)
            .unwrap_or_else(|| worn.iter().map(|s| armor_points(&s.item)).sum());
        list.push(sm::Player {
            uuid: p.uuid,
            name: p.name,
            world: p.dimension,
            position: p.position,
            yaw: p.yaw,
            health: p.health,
            armor: Some(armor),
            display_name: p.nickname,
        });
    }
    let max = game.native_max_players().map_or(0, |m| m as usize);
    sm::publish_players(&list, max);
}

/// The armour value an item gives where worn (vanilla's armour materials; an item's own
/// attribute modifiers are not read). Only where a guest has no body to ask.
fn armor_points(item: &str) -> i32 {
    let Some(name) = item.strip_prefix("minecraft:") else { return 0 };
    if name == "turtle_helmet" {
        return 2;
    }
    let Some((material, piece)) = name.rsplit_once('_') else { return 0 };
    let slot = match piece {
        "helmet" => 0,
        "chestplate" => 1,
        "leggings" => 2,
        "boots" => 3,
        _ => return 0,
    };
    let values = match material {
        "leather" => [1, 3, 2, 1],
        "copper" => [2, 4, 3, 1],
        "chainmail" => [2, 5, 4, 1],
        "golden" => [2, 5, 3, 1],
        "iron" => [2, 6, 5, 2],
        "diamond" | "netherite" => [3, 8, 6, 3],
        _ => return 0,
    };
    values[slot]
}

/// A chunk joined a level.
pub(super) fn chunk_loaded(dimension: &str, chunk: [i32; 2], new: bool) {
    if let Some(w) = sm::world_index(dimension) {
        sm::chunk_loaded(w, chunk[0], chunk[1], new);
    }
}

/// A chunk left a level.
pub(super) fn chunk_unloaded(dimension: &str, chunk: [i32; 2]) {
    if let Some(w) = sm::world_index(dimension) {
        sm::chunk_unloaded(w, chunk[0], chunk[1]);
    }
}

/// The map's commands for the command tree.
pub(super) fn register_commands(commands: &mut Vec<crate::game::native_server::NativeCommand>) {
    use crate::game::native_server::NativeCommand;
    for (usage, level) in sm::commands::usages() {
        let command = if level == 0 { NativeCommand::new(&usage) } else { NativeCommand::op(&usage, level) };
        commands.push(match sm::commands::usage_permission(&usage) {
            Some(node) => command.permission(&node),
            None => command,
        });
    }
}

fn online(game: &Game<'_>) -> Vec<(u128, String)> {
    game.native_server_players().into_iter().map(|p| (p.uuid, p.name)).collect()
}

/// The world names a command's first argument may take: the client asks the server for them
/// (`word?` in the usages), and squaremap's `MapWorldParser` suggestions are pushed over the word.
pub(super) fn tab_complete(command: &str, range: [usize; 2], suggestions: &mut Vec<horizon_client::mod_api::Suggestion>) {
    for text in sm::commands::world_completions(command) {
        if !suggestions.iter().any(|s| s.text == text) {
            suggestions.push(horizon_client::mod_api::Suggestion { range: (range[0], range[1]), text, tooltip: None, int_value: None });
        }
    }
}

/// squaremap's `[squaremap]` prefix (`Messages.COMMAND_PREFIX`): a gradient on the word, and a
/// hover and a click (`/<label> help`) on the whole prefix.
fn prefix() -> Vec<horizon_client::text::Text> {
    use horizon_client::text::{ClickEvent, Style, Text};
    const FROM: (u32, u32, u32) = (0xC0, 0x28, 0xFF);
    const TO: (u32, u32, u32) = (0x5B, 0x00, 0xFF);
    let click = ClickEvent::RunCommand(format!("/{} help", sm::commands::main_label()));
    let styled = |text: &str, color: u32| {
        Text::styled(
            text,
            Style {
                color: Some(color),
                hover: Some(Box::new(Text::literal("Click for help"))),
                click: Some(click.clone()),
                ..Style::default()
            },
        )
    };
    let name = "squaremap";
    let last = (name.chars().count() - 1) as f64;
    let mut out = vec![styled("[", horizon_client::text::WHITE)];
    for (i, c) in name.chars().enumerate() {
        let t = i as f64 / last;
        let mix = |a: u32, b: u32| (a as f64 + (b as f64 - a as f64) * t).round() as u32;
        let color = (mix(FROM.0, TO.0) << 16) | (mix(FROM.1, TO.1) << 8) | mix(FROM.2, TO.2);
        out.push(styled(c.to_string().as_str(), color));
    }
    out.push(styled("]", horizon_client::text::WHITE));
    out.push(styled(" ", horizon_client::text::WHITE));
    out
}

/// One of squaremap's lines as Horizon text: the prefix on errors, then its parts.
fn line_text(line: &sm::commands::Line) -> horizon_client::text::Text {
    use horizon_client::text::{ClickEvent, Style, Text};
    let mut text = Text::empty();
    if line.prefixed {
        for piece in prefix() {
            text.push(piece);
        }
    }
    for part in &line.parts {
        let click = part.click.as_ref().map(|c| match c {
            sm::commands::Click::Run(command) => ClickEvent::RunCommand(command.clone()),
            sm::commands::Click::Open(url) => ClickEvent::OpenUrl(url.clone()),
        });
        let style = Style {
            color: part.color,
            hover: part.hover.map(|h| Box::new(Text::literal(h))),
            click,
            ..Style::default()
        };
        text.push(Text::styled(part.text.clone(), style));
    }
    text
}

fn answer(game: &mut Game<'_>, lines: Vec<sm::commands::Line>) {
    for line in lines {
        game.native_feedback(line_text(&line));
    }
}

/// A player's command.
pub(super) fn perform_command(
    game: &mut Game<'_>,
    player: &crate::game::native_server::NativeServerPlayer,
    command: &str,
    ran: &mut bool,
) {
    if !sm::commands::is_ours(command) {
        return;
    }
    *ran = true;
    // A subcommand's node decides on its own: a permissions mod that grants it lets anyone run
    // it, one that denies it refuses an operator too; without a mod the operator level decides.
    let permission = match sm::commands::permission_node(command) {
        Some(node) if game.native_has_permission(player.uuid, &node, sm::commands::OP_LEVEL) => 4,
        Some(_) => 0,
        None => player.permission,
    };
    let others = match sm::commands::others_node(command) {
        Some(node) => game.native_has_permission(player.uuid, &node, sm::commands::OP_LEVEL),
        None => true,
    };
    let sender = sm::commands::Sender {
        player: Some(sm::commands::SenderPlayer {
            uuid: player.uuid,
            name: player.name.clone(),
            world: player.dimension.clone(),
            x: player.position[0],
            z: player.position[2],
        }),
        permission,
        others,
    };
    let lines = sm::commands::command(command, &sender, &online(game));
    answer(game, lines);
}

/// The console's command.
pub(super) fn perform_console_command(game: &mut Game<'_>, command: &str, ran: &mut bool) {
    if !sm::commands::is_ours(command) {
        return;
    }
    *ran = true;
    let sender = sm::commands::Sender { player: None, permission: 4, others: true };
    let lines = sm::commands::command(command, &sender, &online(game));
    answer(game, lines);
}

/// A block of a level changed through `Level.setBlock`.
pub(super) fn block_changed(dimension: horizon_world::level::Dimension, x: i32, z: i32) {
    use horizon_world::level::Dimension as D;
    let id = match dimension {
        D::Overworld => "minecraft:overworld",
        D::Nether => "minecraft:the_nether",
        D::End => "minecraft:the_end",
    };
    if let Some(w) = sm::world_index(id) {
        sm::block_changed(w, x, z);
    }
}
