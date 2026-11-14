//! `/squaremap` (and its aliases, `/map` by default): squaremap's commands with their permission
//! nodes, English messages, colours, hovers and clicks (`Messages`; MIT, Copyright (c) 2021-2024
//! Jason Penilla and Contributors). The game side hands over the line and who sent it, and sends
//! the returned lines back (`glue.rs`).
use crate::json::WORLDS;
use crate::render::{Mode, PROGRESS_INTERVAL, PROGRESS_LOGGING};
use crate::runtime::{self, RenderRefused};
use std::sync::atomic::Ordering;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Who runs a command.
#[derive(Clone, Debug, Default)]
pub struct Sender {
    /// None for the console.
    pub player: Option<SenderPlayer>,
    /// The operator level (4 for the console; 4 too when a permissions mod grants the node).
    pub permission: u8,
    /// Whether the sender may target another player (`squaremap.command.hide.others` and
    /// `show.others`, which the console has).
    pub others: bool,
}

#[derive(Clone, Debug)]
pub struct SenderPlayer {
    pub uuid: u128,
    pub name: String,
    /// `minecraft:overworld`...
    pub world: String,
    pub x: f64,
    pub z: f64,
}

/// The operator level the map's commands need (squaremap's nodes default to operators on
/// Paper). `confirm` and the bare label have no node, so everyone may run them.
pub const OP_LEVEL: u8 = 2;

/// The node the bare label is registered with. The host gates a whole root command by the
/// first node its usages give, so this one stays open (level 0) unless a permissions mod denies
/// it; without it the first subcommand's node would hide the root from players who lack it.
pub const ROOT_NODE: &str = "squaremap.command";

/// The colours of squaremap's MiniMessage tags (NamedTextColor) and of its help.
const RED: u32 = 0xFF5555;
const GREEN: u32 = 0x55FF55;
const GRAY: u32 = 0xAAAAAA;
const WHITE: u32 = 0xFFFFFF;
const PRIMARY: u32 = 0x5B00FF;

/// The usages to register, each with the operator level it needs, for the configured label and
/// aliases.
pub fn usages() -> Vec<(String, u8)> {
    let c = runtime::config().settings.commands;
    let mut labels = vec![c.main_command_label.clone()];
    labels.extend(c.main_command_aliases.iter().filter(|a| **a != c.main_command_label).cloned());
    let mut out = Vec::new();
    for l in labels.iter().filter(|l| !l.is_empty() && !l.contains(char::is_whitespace)) {
        out.push((l.clone(), 0));
        out.push((format!("{l} confirm"), 0));
        // World names are `minecraft:<name>` keys, which a `word` argument refuses (no colon); a world that
        // comes last is `text` instead. `radiusrender` has the world first, so it stays `word?` (`overworld`, `the_nether`).
        for u in [
            "help [query:text]",
            "fullrender [world:text]",
            "radiusrender <world:word?> <radius:integer>",
            "radiusrender <world:word?> <radius:integer> <x:word> <z:word>",
            "cancelrender [world:text]",
            "pauserender [world:text]",
            "resetmap <world:text>",
            "progresslogging",
            "progresslogging toggle",
            "progresslogging rate <seconds:integer>",
            "hide [player:player]",
            "show [player:player]",
            "reload",
            "link",
        ] {
            out.push((format!("{l} {u}"), OP_LEVEL));
        }
    }
    out
}

/// squaremap's permission node for a subcommand (`squaremap.command.<sub>`); None for
/// `confirm` and the bare label. `hide`/`show` check their `.others` node on top, see
/// `others_node`.
fn node_of(sub: Option<&str>) -> Option<String> {
    const NODED: [&str; 11] =
        ["help", "cancelrender", "fullrender", "pauserender", "radiusrender", "resetmap", "progresslogging", "hide", "show", "reload", "link"];
    let sub = sub?.to_ascii_lowercase();
    NODED.contains(&sub.as_str()).then(|| format!("squaremap.command.{sub}"))
}

/// The permission node a usage (as `usages` lists it) is registered with.
pub fn usage_permission(usage: &str) -> Option<String> {
    let mut words = usage.split_whitespace();
    words.next()?;
    match words.next() {
        None => Some(ROOT_NODE.to_owned()),
        sub => node_of(sub),
    }
}

/// The permission node a command line needs (see `usage_permission`).
pub fn permission_node(line: &str) -> Option<String> {
    let words: Vec<&str> = line.trim().trim_start_matches('/').split_whitespace().collect();
    node_of(words.get(1).copied())
}

/// The node `hide <player>` / `show <player>` need to target someone else.
pub fn others_node(line: &str) -> Option<String> {
    let words: Vec<&str> = line.trim().trim_start_matches('/').split_whitespace().collect();
    match words.get(1).map(|s| s.to_ascii_lowercase()).as_deref() {
        Some("hide") => Some("squaremap.command.hide.others".to_owned()),
        Some("show") => Some("squaremap.command.show.others".to_owned()),
        _ => None,
    }
}

/// Whether a command line is one of the map's (its first word is the label or an alias).
pub fn is_ours(line: &str) -> bool {
    let first = line.split_whitespace().next().unwrap_or("").trim_start_matches('/');
    let c = runtime::config().settings.commands;
    first.eq_ignore_ascii_case(&c.main_command_label) || c.main_command_aliases.iter().any(|a| first.eq_ignore_ascii_case(a))
}

/// The label messages show (`main-command-label`).
pub fn main_label() -> String {
    runtime::config().settings.commands.main_command_label
}

/// What a click on a part of a message does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Click {
    /// `run_command`, with its slash.
    Run(String),
    /// `open_url`.
    Open(String),
}

/// A piece of a message: its text, its colour (None: the chat's own), and an optional hover
/// text and click.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Part {
    pub text: String,
    pub color: Option<u32>,
    pub hover: Option<&'static str>,
    pub click: Option<Click>,
}

/// One message. `prefixed` lines carry squaremap's `[squaremap]` prefix: squaremap puts it on
/// command errors only, feedback comes without it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Line {
    pub prefixed: bool,
    pub parts: Vec<Part>,
}

impl Line {
    /// The text without colours, hovers and clicks.
    pub fn text(&self) -> String {
        self.parts.iter().map(|p| p.text.as_str()).collect()
    }
}

fn part(text: impl Into<String>) -> Part {
    Part { text: text.into(), color: None, hover: None, click: None }
}

fn colored(text: impl Into<String>, color: u32) -> Part {
    Part { color: Some(color), ..part(text) }
}

fn clickable(mut p: Part, hover: &'static str, click: Click) -> Part {
    p.hover = Some(hover);
    p.click = Some(click);
    p
}

/// Feedback without the prefix (squaremap's `sendMessage` of a MiniMessage).
fn feedback(parts: Vec<Part>) -> Line {
    Line { prefixed: false, parts }
}

fn plain(text: impl Into<String>) -> Line {
    feedback(vec![part(text)])
}

fn msg(text: impl Into<String>, color: u32) -> Line {
    feedback(vec![colored(text, color)])
}

/// An error, with the prefix.
fn error(parts: Vec<Part>) -> Line {
    Line { prefixed: true, parts }
}

/// squaremap's `invalid-argument`: a red label, then the reason in grey.
fn invalid_argument(reason: impl Into<String>) -> Line {
    error(vec![colored("Invalid command argument", RED), colored(":", WHITE), part(" "), colored(reason, GRAY)])
}

/// squaremap's `invalid-syntax` with the usage (after the label).
fn invalid_syntax(usage: &str) -> Line {
    error(vec![
        colored("Invalid command syntax. Correct command syntax is", RED),
        colored(":", WHITE),
        part(" "),
        colored(format!("/{} {usage}", main_label()), GRAY),
    ])
}

fn no_permission() -> Line {
    error(vec![colored(
        "I'm sorry, but you do not have permission to perform this command.\nPlease contact the server administrators if you believe that this is in error.",
        RED,
    )])
}

fn no_such_world(input: &str) -> Line {
    invalid_argument(format!("No such world '{input}'"))
}

fn number_out_of_range(input: &str) -> Line {
    invalid_argument(format!("'{input}' is not a valid number in the range 1 to 2147483647"))
}

fn world_id(w: usize) -> &'static str {
    WORLDS[w].0
}

/// Why a render command was refused, worded as squaremap words it.
fn refused(e: RenderRefused, w: usize) -> Line {
    let id = world_id(w);
    match e {
        RenderRefused::InProgress => msg(format!("A render is already in progress on {id}"), RED),
        RenderRefused::NotInProgress => msg(format!("No renders running for {id}"), RED),
        RenderRefused::NotEnabled => error(vec![colored(format!("Map is not enabled for world '{id}'"), RED)]),
        RenderRefused::NotRunning => msg("The map is not running", RED),
    }
}

/// `resetmap` waits for `confirm` from the same sender this long (squaremap: 15 seconds).
const CONFIRM_FOR: Duration = Duration::from_secs(15);
static PENDING: Mutex<Vec<(Option<u128>, String, Instant)>> = Mutex::new(Vec::new());

/// squaremap's `/squaremap help` entries: the syntax and the description.
const HELP: [(&str, &str); 16] = [
    ("help [query]", "Get help for squaremap commands"),
    ("confirm", "Confirm a pending command"),
    ("fullrender [world]", "Starts a full render for the specified world"),
    ("radiusrender <world> <radius> [center]", "Starts a radius render"),
    ("cancelrender [world]", "Cancels a render for the specified world"),
    ("pauserender [world]", "Pauses all renders for the specified world"),
    ("resetmap <world>", "Resets the map of a specified world"),
    ("progresslogging", "Prints the current settings for render progress logging"),
    ("progresslogging toggle", "Toggles render progress logging on or off"),
    ("progresslogging rate <seconds>", "Sets the interval in seconds for logging render progress"),
    ("hide [player]", "Hide a player from the map view"),
    ("hide <player>", "Hide a player from the map view"),
    ("show [player]", "Show a player on the map view"),
    ("show <player>", "Show a player on the map view"),
    ("reload", "Reloads the plugin"),
    ("link", "Get a link to the map at your current location"),
];

/// A level by its identifier (`MapWorldParser`): `minecraft:the_nether`, or `the_nether` in the
/// `minecraft` namespace. Other spellings (`nether`) name no level.
pub fn parse_world(s: &str) -> Option<usize> {
    let id = if s.contains(':') { s.to_owned() } else { format!("minecraft:{s}") };
    runtime::world_index(&id)
}

/// Runs a map command line (without its slash); `online` lists the players as (UUID, name)
/// for `hide`/`show`. Returns the lines to send back.
pub fn command(line: &str, sender: &Sender, online: &[(u128, String)]) -> Vec<Line> {
    let line = line.trim().trim_start_matches('/');
    let words: Vec<&str> = line.split_whitespace().collect();
    let sub = words.get(1).map(|s| s.to_ascii_lowercase()).unwrap_or_default();
    let args = &words[words.len().min(2)..];
    if node_of(Some(sub.as_str())).is_some() && sender.permission < OP_LEVEL {
        return vec![no_permission()];
    }
    match sub.as_str() {
        "" | "help" => help(&args.join(" ")),
        "confirm" => confirm(sender, online),
        "fullrender" => match world_arg(args.first().copied(), sender) {
            Err(e) => vec![e],
            Ok(w) => match runtime::start_full_render(w) {
                Ok(()) if sender.player.is_some() => vec![plain(format!("Started full map render for {}", world_id(w)))],
                Ok(()) => vec![],
                Err(e) => vec![refused(e, w)],
            },
        },
        "radiusrender" => radius(args, sender),
        "cancelrender" => match world_arg(args.first().copied(), sender) {
            Err(e) => vec![e],
            Ok(w) => match runtime::cancel_render(w) {
                Ok(()) => vec![msg(format!("Render cancelled for {}", world_id(w)), GREEN)],
                Err(e) => vec![refused(e, w)],
            },
        },
        "pauserender" => match world_arg(args.first().copied(), sender) {
            Err(e) => vec![e],
            Ok(w) => match runtime::toggle_pause(w) {
                Some(true) => vec![msg(format!("Paused renders for {}", world_id(w)), GREEN)],
                Some(false) => vec![msg(format!("Unpaused renders for {}", world_id(w)), GREEN)],
                None => vec![refused(RenderRefused::NotRunning, w)],
            },
        },
        "resetmap" => reset_request(args, sender, line),
        "progresslogging" => progress(args),
        "hide" | "show" => hide_show(sub == "hide", args.first().copied(), sender, online),
        "reload" => match runtime::reload() {
            Ok(()) => vec![msg(format!("squaremap v{} reloaded", env!("CARGO_PKG_VERSION")), GREEN)],
            Err(e) => vec![msg(e, RED)],
        },
        "link" => vec![link(sender)],
        other => vec![msg(format!("Unknown command '{other}'. Try /{} help", main_label()), RED)],
    }
}

fn help(query: &str) -> Vec<Line> {
    let label = main_label();
    let q = query.trim();
    let needle = q.to_ascii_lowercase();
    let found: Vec<&(&str, &str)> = HELP
        .iter()
        .filter(|(u, d)| needle.is_empty() || u.contains(needle.as_str()) || d.to_ascii_lowercase().contains(needle.as_str()))
        .collect();
    if needle.is_empty() {
        let mut out = vec![feedback(vec![colored("squaremap command help", PRIMARY)])];
        out.extend(found.iter().map(|entry| help_entry(&label, entry)));
        return out;
    }
    if found.is_empty() {
        return vec![msg(format!("No results for query: {q}"), GRAY)];
    }
    let mut out = vec![msg(format!("Showing search results for query: {q}"), GRAY)];
    out.extend(found.iter().map(|entry| help_entry(&label, entry)));
    out
}

fn help_entry(label: &str, (usage, desc): &(&str, &str)) -> Line {
    let sub = usage.split_whitespace().next().unwrap_or_default();
    feedback(vec![
        clickable(
            colored(format!("/{label} {usage}"), WHITE),
            "Click to show help for this command",
            Click::Run(format!("/{label} help {sub}")),
        ),
        part(" - "),
        colored(*desc, GRAY),
    ])
}

/// The world a command names, or the sender's own world. Errors come as squaremap prints them.
fn world_arg(arg: Option<&str>, sender: &Sender) -> Result<usize, Line> {
    match (arg, sender.player.as_ref()) {
        (Some(a), _) => parse_world(a).ok_or_else(|| no_such_world(a)),
        (None, Some(p)) => runtime::world_index(&p.world)
            .ok_or_else(|| error(vec![colored(format!("Map is not enabled for world '{}'", p.world), RED)])),
        (None, None) => Err(error(vec![colored("You must specify the world when running this command from console", RED)])),
    }
}

/// A coordinate argument: a number, or `~`/`~n` relative to `base` (the sender's position, 0 for
/// the console). Numbers are floored to a block.
fn coord(s: &str, base: f64) -> Option<i32> {
    match s.strip_prefix('~') {
        Some(rest) => {
            let off = if rest.is_empty() { 0.0 } else { rest.parse::<f64>().ok()? };
            Some((base + off).floor() as i32)
        }
        None => s.parse::<f64>().ok().map(|v| v.floor() as i32),
    }
}

fn radius(args: &[&str], sender: &Sender) -> Vec<Line> {
    let (Some(w), Some(r)) = (args.first().copied(), args.get(1).copied()) else {
        return vec![invalid_syntax("radiusrender <world> <radius> [center]")];
    };
    let Some(w) = parse_world(w) else { return vec![no_such_world(w)] };
    let r = match r.parse::<i32>() {
        Ok(r) if r >= 1 => r,
        _ => return vec![number_out_of_range(r)],
    };
    let (base_x, base_z) = sender.player.as_ref().map_or((0.0, 0.0), |p| (p.x, p.z));
    let center = match (args.get(2).copied(), args.get(3).copied()) {
        (Some(x), Some(z)) => match (coord(x, base_x), coord(z, base_z)) {
            (Some(x), Some(z)) => [x, z],
            _ => return vec![invalid_argument(format!("'{x} {z}' is not a valid location. Required format is '<x> <z>'"))],
        },
        (None, None) => [0, 0],
        _ => return vec![invalid_syntax("radiusrender <world> <radius> [center]")],
    };
    match runtime::start_radius_render(w, center, r) {
        Ok(()) => vec![plain(format!("Started radius map render for {}", world_id(w)))],
        Err(e) => vec![refused(e, w)],
    }
}

/// `resetmap` asks for a confirmation first (squaremap's confirmation message, clickable).
fn reset_request(args: &[&str], sender: &Sender, line: &str) -> Vec<Line> {
    let Some(world) = args.first().copied() else {
        return vec![invalid_syntax("resetmap <world>")];
    };
    if parse_world(world).is_none() {
        return vec![no_such_world(world)];
    }
    let who = sender.player.as_ref().map(|p| p.uuid);
    let mut pending = PENDING.lock().unwrap();
    pending.retain(|(w, _, _)| *w != who);
    pending.push((who, line.to_owned(), Instant::now()));
    let label = main_label();
    vec![feedback(vec![clickable(
        colored(format!("Confirmation required. Confirm using /{label} confirm."), RED),
        "Click to confirm",
        Click::Run(format!("/{label} confirm")),
    )])]
}

fn confirm(sender: &Sender, online: &[(u128, String)]) -> Vec<Line> {
    let who = sender.player.as_ref().map(|p| p.uuid);
    let pending = {
        let mut p = PENDING.lock().unwrap();
        p.retain(|(_, _, at)| at.elapsed() < CONFIRM_FOR);
        p.iter().position(|(w, _, _)| *w == who).map(|i| p.remove(i).1)
    };
    let Some(line) = pending else {
        return vec![msg("You don't have any pending commands.", RED)];
    };
    // Only resetmap asks for a confirmation.
    let words: Vec<&str> = line.split_whitespace().collect();
    let Some(w) = words.get(2).and_then(|a| parse_world(a)) else {
        return command(&line, sender, online);
    };
    match runtime::reset_map(w) {
        Ok(()) => vec![msg(format!("Successfully reset map for {}", world_id(w)), GREEN)],
        Err(e) => vec![msg(e, RED)],
    }
}

fn progress(args: &[&str]) -> Vec<Line> {
    let label = main_label();
    match args.first().map(|s| s.to_ascii_lowercase()).as_deref() {
        None => {
            let (symbol, color) = if PROGRESS_LOGGING.load(Ordering::Relaxed) {
                ("\u{2714}", GREEN)
            } else {
                ("\u{2716}", RED)
            };
            vec![feedback(vec![
                part("Render progress logging enabled: "),
                clickable(colored(symbol, color), "Click to toggle", Click::Run(format!("/{label} progresslogging toggle"))),
                part(", interval: "),
                colored(PROGRESS_INTERVAL.load(Ordering::Relaxed).to_string(), GREEN),
                part(" seconds"),
            ])]
        }
        Some("toggle") => {
            let now = !PROGRESS_LOGGING.load(Ordering::Relaxed);
            PROGRESS_LOGGING.store(now, Ordering::Relaxed);
            runtime::restart_progress_logging();
            let text = if now {
                "Render progress logging has been enabled."
            } else {
                "Render progress logging has been disabled."
            };
            vec![feedback(vec![clickable(colored(text, GREEN), "Click to toggle", Click::Run(format!("/{label} progresslogging toggle")))])]
        }
        Some("rate") => match args.get(1).copied() {
            None => vec![invalid_syntax("progresslogging rate <seconds>")],
            Some(s) => match s.parse::<i32>() {
                Ok(s) if s >= 1 => {
                    PROGRESS_INTERVAL.store(s as u64, Ordering::Relaxed);
                    runtime::restart_progress_logging();
                    vec![feedback(vec![colored("Render progress logging interval has been set to ", GREEN), part(format!("{s} seconds"))])]
                }
                _ => vec![number_out_of_range(s)],
            },
        },
        Some(_) => vec![invalid_syntax("progresslogging")],
    }
}

fn hide_show(hide: bool, target: Option<&str>, sender: &Sender, online: &[(u128, String)]) -> Vec<Line> {
    if target.is_some() && !sender.others {
        return vec![no_permission()];
    }
    let found = match target {
        None => sender.player.as_ref().map(|p| (p.uuid, p.name.clone())),
        Some(name) => online.iter().find(|(_, n)| n.eq_ignore_ascii_case(name)).cloned(),
    };
    let Some((uuid, name)) = found else {
        return vec![match target {
            None => error(vec![colored("You must specify a target player when running this command from console", RED)]),
            Some(n) => invalid_argument(format!("No player named '{n}' is online")),
        }];
    };
    let hidden = crate::api::player_hidden(uuid);
    match (hide, hidden) {
        (true, true) => vec![msg(format!("{name} is already hidden from map"), RED)],
        (true, false) => {
            crate::api::hide_player(uuid, true);
            vec![msg(format!("{name} is now hidden from map"), GREEN)]
        }
        (false, false) => vec![msg(format!("{name} is not hidden from map"), RED)],
        (false, true) => {
            crate::api::show_player(uuid, true);
            vec![msg(format!("{name} is no longer hidden from map"), GREEN)]
        }
    }
}

/// `LinkCommand`: the web address, with `?uuid=` of a player sender.
fn link(sender: &Sender) -> Line {
    let config = runtime::config();
    let mut link = config.settings.web_address;
    // The stock address (`http://localhost:8080`) is only a placeholder: when the internal web server
    // listens elsewhere, the link points at it, so `/map link` is usable out of the box.
    let server = &config.settings.internal_webserver;
    if link == "http://localhost:8080" && server.enabled && server.port != 8080 {
        link = format!("http://{}:{}", server.bind, server.port);
    }
    if let Some(p) = &sender.player {
        link.push(if link.contains('?') { '&' } else { '?' });
        link.push_str(&format!("uuid={:032x}", p.uuid));
    }
    feedback(vec![
        part("Click to open map: "),
        Part { text: link.clone(), color: Some(GREEN), hover: None, click: Some(Click::Open(link)) },
    ])
}

/// The completions of the world argument (the first one) of the commands that take a world:
/// the level identifiers, and the short name of the `minecraft` ones once something is typed
/// (`MapWorldParser`'s suggestions). Empty for any other line.
pub fn world_completions(command: &str) -> Vec<String> {
    const WORLD_SUBS: [&str; 5] = ["fullrender", "radiusrender", "cancelrender", "pauserender", "resetmap"];
    let words: Vec<&str> = command.split(' ').collect();
    if words.len() != 3 || !is_ours(command) || !WORLD_SUBS.contains(&words[1].to_ascii_lowercase().as_str()) {
        return Vec::new();
    }
    let typed = words[2].to_ascii_lowercase();
    let mut out = Vec::new();
    for (id, _) in WORLDS {
        let short = id.strip_prefix("minecraft:").unwrap_or(id);
        let candidates: Vec<&str> = if typed.is_empty() { vec![id] } else { vec![short, id] };
        out.extend(
            candidates
                .into_iter()
                .filter(|c| c.to_ascii_lowercase().starts_with(typed.as_str()))
                .map(str::to_owned),
        );
    }
    out
}

/// A one-line status of each level's render, for operators (`/squaremap` has no status
/// command; used by the game side's log if wanted).
pub fn status_lines() -> Vec<String> {
    (0..WORLDS.len())
        .filter_map(|w| {
            runtime::render_status(w).map(|(mode, done, total, paused)| {
                let m = if mode == Mode::Full { "full" } else { "radius" };
                format!("{}: {m} render {done}/{total} chunks{}", WORLDS[w].0, if paused { " (paused)" } else { "" })
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(lines: &[Line]) -> Vec<String> {
        lines.iter().map(Line::text).collect()
    }

    fn console() -> Sender {
        Sender { player: None, permission: 4, others: true }
    }

    fn player(level: u8) -> Sender {
        Sender {
            player: Some(SenderPlayer { uuid: 7, name: "Alex".into(), world: "minecraft:the_nether".into(), x: 100.7, z: -3.2 }),
            permission: level,
            others: false,
        }
    }

    #[test]
    fn usages_cover_label_and_alias() {
        let u = usages();
        assert!(u.contains(&("squaremap".into(), 0)));
        assert!(u.contains(&("map confirm".into(), 0)));
        assert!(u.contains(&("map fullrender [world:text]".into(), OP_LEVEL)));
        assert!(is_ours("map hide") && is_ours("/SquareMap") && !is_ours("maps"));
    }

    #[test]
    fn permission_nodes_per_subcommand() {
        assert_eq!(permission_node("/map fullrender"), Some("squaremap.command.fullrender".into()));
        assert_eq!(permission_node("squaremap progresslogging rate 5"), Some("squaremap.command.progresslogging".into()));
        assert_eq!(permission_node("squaremap help hide"), Some("squaremap.command.help".into()));
        assert_eq!(permission_node("squaremap confirm"), None);
        assert_eq!(permission_node("squaremap"), None);
        assert_eq!(usage_permission("map hide [player:player]"), Some("squaremap.command.hide".into()));
        assert_eq!(usage_permission("squaremap"), Some(ROOT_NODE.into()));
        assert_eq!(usage_permission("squaremap confirm"), None);
        assert_eq!(others_node("squaremap show Steve"), Some("squaremap.command.show.others".into()));
        assert_eq!(others_node("squaremap hide"), Some("squaremap.command.hide.others".into()));
        assert_eq!(others_node("squaremap reload"), None);
    }

    #[test]
    fn worlds_and_coordinates() {
        assert_eq!(parse_world("minecraft:the_nether"), Some(2));
        assert_eq!(parse_world("the_nether"), Some(2));
        assert_eq!(parse_world("minecraft:overworld"), Some(0));
        assert_eq!(parse_world("minecraft:the_end"), Some(1));
        assert_eq!(parse_world("nether"), None);
        assert_eq!(parse_world("mars"), None);
        assert_eq!(coord("~", 100.7), Some(100));
        assert_eq!(coord("~-5", 0.5), Some(-5));
        assert_eq!(coord("-3.5", 0.0), Some(-4));
        assert_eq!(coord("~", 0.0), Some(0));
        assert_eq!(world_arg(None, &player(2)).ok(), Some(2));
        assert!(world_arg(None, &console()).is_err());
    }

    #[test]
    fn world_names_for_the_first_argument() {
        assert_eq!(world_completions("/squaremap fullrender "), ["minecraft:overworld", "minecraft:the_end", "minecraft:the_nether"]);
        assert_eq!(world_completions("map resetmap the"), ["the_end", "the_nether"]);
        assert!(world_completions("squaremap hide ").is_empty());
        assert!(world_completions("squaremap fullrender minecraft:the_end ").is_empty());
    }

    #[test]
    fn messages_without_a_running_map() {
        assert_eq!(
            texts(&command("squaremap fullrender", &player(0), &[])),
            ["I'm sorry, but you do not have permission to perform this command.\nPlease contact the server administrators if you believe that this is in error."]
        );
        assert_eq!(texts(&command("squaremap", &player(0), &[]))[0], "squaremap command help");
        assert_eq!(command("map help hide", &player(2), &[]).len(), 3);
        assert_eq!(texts(&command("squaremap fullrender", &console(), &[])), ["You must specify the world when running this command from console"]);
        assert_eq!(texts(&command("squaremap cancelrender nether", &console(), &[])), ["Invalid command argument: No such world 'nether'"]);
        assert_eq!(
            texts(&command("squaremap radiusrender minecraft:the_nether 0", &console(), &[])),
            ["Invalid command argument: '0' is not a valid number in the range 1 to 2147483647"]
        );
        assert_eq!(
            texts(&command("squaremap link", &player(2), &[])),
            ["Click to open map: http://localhost:8080?uuid=00000000000000000000000000000007"]
        );
        assert_eq!(texts(&command("squaremap confirm", &player(2), &[])), ["You don't have any pending commands."]);
    }

    #[test]
    fn resetmap_asks_then_confirm_runs_it() {
        assert_eq!(texts(&command("squaremap resetmap minecraft:the_end", &console(), &[])), ["Confirmation required. Confirm using /squaremap confirm."]);
        // Without the runtime the reset cannot run.
        assert_eq!(texts(&command("squaremap confirm", &console(), &[])), ["the map has not started"]);
    }

    #[test]
    fn hide_and_show_by_name() {
        let online = [(0xABCD_u128, "Steve".to_owned())];
        assert_eq!(texts(&command("squaremap hide steve", &console(), &online)), ["Steve is now hidden from map"]);
        assert_eq!(texts(&command("squaremap hide Steve", &console(), &online)), ["Steve is already hidden from map"]);
        assert_eq!(texts(&command("squaremap show Steve", &console(), &online)), ["Steve is no longer hidden from map"]);
        assert_eq!(texts(&command("squaremap show Steve", &console(), &online)), ["Steve is not hidden from map"]);
        assert_eq!(
            texts(&command("squaremap hide", &console(), &online)),
            ["You must specify a target player when running this command from console"]
        );
        assert_eq!(
            texts(&command("squaremap hide Nobody", &console(), &online)),
            ["Invalid command argument: No player named 'Nobody' is online"]
        );
        // Targeting someone else needs the `.others` node.
        assert_eq!(texts(&command("squaremap hide Steve", &player(2), &online))[0], "I'm sorry, but you do not have permission to perform this command.\nPlease contact the server administrators if you believe that this is in error.");
        assert_eq!(
            texts(&command("squaremap progresslogging rate 5", &console(), &[])),
            ["Render progress logging interval has been set to 5 seconds"]
        );
    }
}
