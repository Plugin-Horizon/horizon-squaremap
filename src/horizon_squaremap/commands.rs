//! `/squaremap` (and its aliases, `/map` by default): squaremap's commands with their permission
//! nodes, English messages, colours, hovers and clicks (`Messages`; MIT, Copyright (c) 2021-2024
//! Jason Penilla and Contributors). The game side hands over the line and who sent it, and sends
//! the returned lines back (`glue.rs`).
use crate::horizon_squaremap::render::{Mode, PROGRESS_INTERVAL, PROGRESS_LOGGING};
use crate::horizon_squaremap::runtime::{self, RenderRefused};
use std::collections::HashMap;
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
    /// Which way the sender faces, in degrees (0 facing south), for `^` local coordinates.
    pub yaw: f64,
}

/// The operator level the map's commands need: squaremap's nodes are unregistered Bukkit nodes, which
/// default to operators of any level (1 and up). `confirm` and the bare label have no node, so everyone
/// may run them.
pub const OP_LEVEL: u8 = 1;

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
const DARK_GRAY: u32 = 0x555555;

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
    pub hover: Option<String>,
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
        self.expanded().iter().map(|p| p.text.as_str()).collect()
    }

    /// The parts with the language file's MiniMessage tags turned into colours, clicks and hovers.
    pub fn expanded(&self) -> Vec<Part> {
        self.parts.iter().flat_map(crate::horizon_squaremap::mini::expand).collect()
    }
}

fn part(text: impl Into<String>) -> Part {
    Part { text: text.into(), color: None, hover: None, click: None }
}

fn colored(text: impl Into<String>, color: u32) -> Part {
    Part { color: Some(color), ..part(text) }
}

fn clickable(mut p: Part, hover: impl Into<String>, click: Click) -> Part {
    p.hover = Some(hover.into());
    p.click = Some(click);
    p
}

/// The language file's messages, by key (`locale/lang-*.yml`, nested keys joined with dots).
static LANG: Mutex<Option<HashMap<String, String>>> = Mutex::new(None);

/// Reads a language file (squaremap's `locale/lang-en.yml` form). Keys it lacks keep the English text. A message
/// is MiniMessage there: its colour, click and hover tags are followed (`mini`), the message keeping squaremap's colour
/// where the file sets none. A file that cannot be read leaves the previous language.
pub fn set_language(text: &str) -> Result<(), String> {
    let table = crate::horizon_squaremap::yaml::parse(text)?;
    *LANG.lock().unwrap() = Some(crate::horizon_squaremap::yaml::flatten(&table));
    Ok(())
}

/// Back to the built-in English text.
pub fn clear_language() {
    *LANG.lock().unwrap() = None;
}

/// A message by its key in the language file: the file's text (behind `mini::MARK`, its tags still in it, for
/// `Line::expanded`), else `default`, with `<name>` placeholders from `vars` filled in and every other tag dropped.
fn tr(key: &str, default: &str, vars: &[(&str, &str)]) -> String {
    tr_from(LANG.lock().unwrap().as_ref(), key, default, vars)
}

fn tr_from(lang: Option<&HashMap<String, String>>, key: &str, default: &str, vars: &[(&str, &str)]) -> String {
    match lang.and_then(|m| m.get(key)) {
        Some(text) => format!("{}{}", crate::horizon_squaremap::mini::MARK, crate::horizon_squaremap::mini::fill_rich(text, vars)),
        None => fill(default, vars),
    }
}

fn fill(template: &str, vars: &[(&str, &str)]) -> String {
    let mut out = String::new();
    let mut rest = template;
    while let Some(open) = rest.find('<') {
        out.push_str(&rest[..open]);
        let Some(len) = rest[open..].find('>') else {
            out.push_str(&rest[open..]);
            return out;
        };
        let name = &rest[open + 1..open + len];
        match vars.iter().find(|(v, _)| *v == name) {
            Some((_, value)) => out.push_str(value),
            None => {} // a colour, click, hover or closing tag
        }
        rest = &rest[open + len + 1..];
    }
    out.push_str(rest);
    out
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
    let mut parts = vec![colored("Invalid command syntax. Correct command syntax is", RED), colored(":", WHITE), part(" ")];
    parts.extend(highlight_special(&format!("/{} {usage}", main_label()), GRAY));
    error(parts)
}

/// `Components.highlightSpecialCharacters`: every character that is not a word character, a space or a hyphen in white,
/// the runs between in `color`.
fn highlight_special(text: &str, color: u32) -> Vec<Part> {
    let mut out: Vec<Part> = Vec::new();
    let mut run = String::new();
    for c in text.chars() {
        if c.is_alphanumeric() || c == '_' || c == '-' || c.is_whitespace() {
            run.push(c);
        } else {
            if !run.is_empty() {
                out.push(colored(std::mem::take(&mut run), color));
            }
            out.push(colored(c.to_string(), WHITE));
        }
    }
    if !run.is_empty() {
        out.push(colored(run, color));
    }
    out
}

/// The subcommands of the bare label, as cloud lists them when the label alone is typed.
const ROOT_SYNTAX: &str = "<cancelrender|confirm|fullrender|help|hide|link|pauserender|progresslogging|radiusrender|reload|resetmap|show>";

/// The permission node that shows the failure's detail (ExceptionHandler.commandExecution).
pub const STACKTRACE_NODE: &str = "squaremap.command-exception-stacktrace";

/// squaremap's `command.message.exception.command-execution`: an unexpected failure while a command ran. `detail` is
/// put on hover for senders holding `squaremap.command-exception-stacktrace`.
pub fn command_failed(detail: Option<&str>) -> Line {
    let text = tr("command.message.exception.command-execution", "An internal error occurred while attempting to perform this command.", &[]);
    let mut p = colored(text, RED);
    p.hover = detail.map(str::to_string);
    error(vec![p])
}

/// squaremap's `command.message.exception.no-permission`, as the player sees it.
pub fn no_permission() -> Line {
    error(vec![colored(
        tr(
            "command.message.exception.no-permission",
            "I'm sorry, but you do not have permission to perform this command.\nPlease contact the server administrators if you believe that this is in error.",
            &[],
        ),
        RED,
    )])
}

fn no_such_world(input: &str) -> Line {
    invalid_argument(tr("command.message.no-such-world", "No such world '<world>'", &[("world", input)]))
}

fn number_out_of_range(input: &str) -> Line {
    invalid_argument(tr(
        "command.message.parser-exception.number",
        "'<input>' is not a valid number in the range <min> to <max>",
        &[("input", input), ("min", "1"), ("max", "2147483647")],
    ))
}

/// The bad `<x> <z>` of `radiusrender`'s centre.
fn bad_location(input: &str) -> Line {
    invalid_argument(tr(
        "command.message.parser-exception.location.invalid_format",
        "'<input>' is not a valid location. Required format is '<format>'",
        // `fill` drops unknown `<...>` tags, so the literal `<x> <z>` comes in as a value.
        &[("input", input), ("format", "<x> <z>")],
    ))
}

fn mixed_local_absolute() -> Line {
    invalid_argument(tr(
        "command.message.parser-exception.location.mixed_local_absolute",
        "Cannot mix local and absolute coordinates. (either all coordinates use '^' or none do)",
        &[],
    ))
}

/// The level's identifier (`minecraft:the_nether`, `mod:castle`), "" for an index the map does not know.
fn world_id(w: usize) -> String {
    runtime::world_name(w)
}

/// Why a render command was refused, worded as squaremap words it.
fn refused(e: RenderRefused, w: usize) -> Line {
    let name = world_id(w);
    let id = name.as_str();
    match e {
        RenderRefused::InProgress => msg(tr("render-in-progress", "A render is already in progress on <world>", &[("world", id)]), RED),
        RenderRefused::NotInProgress => msg(tr("render-not-in-progress", "No renders running for <world>", &[("world", id)]), RED),
        RenderRefused::NotEnabled => error(vec![colored(
            tr("command.message.map-not-enabled-for-world", "Map is not enabled for world '<world>'", &[("world", id)]),
            RED,
        )]),
        RenderRefused::NotRunning => msg("The map is not running", RED),
    }
}

/// `resetmap` waits for `confirm` from the same sender this long (squaremap: 15 seconds).
const CONFIRM_FOR: Duration = Duration::from_secs(15);
static PENDING: Mutex<Vec<(Option<u128>, String, Instant)>> = Mutex::new(Vec::new());

/// One argument of a help entry: its name, whether it is optional, the language key of its description and the
/// English text (empty: none).
struct HelpArg {
    name: &'static str,
    optional: bool,
    key: &'static str,
    description: &'static str,
}

/// One command in squaremap's `/squaremap help`: the syntax after the label, the description's language key and text, and its
/// arguments. Sorted by syntax, as cloud's help index lists them.
struct HelpEntry {
    syntax: &'static str,
    key: &'static str,
    description: &'static str,
    args: &'static [HelpArg],
}

const OPTIONAL_WORLD: HelpArg = HelpArg {
    name: "world",
    optional: true,
    key: "command.argument.optional-world",
    description: "Defaults to the players current world if not provided",
};
const PLAYER_ARG: HelpArg = HelpArg {
    name: "player",
    optional: false,
    key: "command.argument.optional-player",
    description: "Defaults to the executing player if unspecified (console must specify a player)",
};
const PLAIN_WORLD: HelpArg = HelpArg { name: "world", optional: false, key: "", description: "" };

const HELP: [HelpEntry; 16] = [
    HelpEntry {
        syntax: "cancelrender [world]",
        key: "command.description.cancel-render",
        description: "Cancels a render for the specified world",
        args: &[OPTIONAL_WORLD],
    },
    HelpEntry { syntax: "confirm", key: "command.description.confirm", description: "Confirm a pending command", args: &[] },
    HelpEntry {
        syntax: "fullrender [world]",
        key: "command.description.full-render",
        description: "Starts a full render for the specified world",
        args: &[OPTIONAL_WORLD],
    },
    HelpEntry {
        syntax: "help [query]",
        key: "command.description.help",
        description: "Get help for squaremap commands",
        args: &[HelpArg { name: "query", optional: true, key: "command.argument.help-query", description: "Help Query" }],
    },
    HelpEntry { syntax: "hide", key: "command.description.hide", description: "Hide a player from the map view", args: &[] },
    HelpEntry { syntax: "hide <player>", key: "command.description.hide", description: "Hide a player from the map view", args: &[PLAYER_ARG] },
    HelpEntry {
        syntax: "link",
        key: "command.description.link",
        description: "Get a link to the map at your current location",
        args: &[],
    },
    HelpEntry {
        syntax: "pauserender [world]",
        key: "command.description.pause-render",
        description: "Pauses all renders for the specified world",
        args: &[OPTIONAL_WORLD],
    },
    HelpEntry {
        syntax: "progresslogging",
        key: "command.description.progresslogging.status",
        description: "Prints the current settings for render progress logging",
        args: &[],
    },
    HelpEntry {
        syntax: "progresslogging rate <seconds>",
        key: "command.description.progresslogging.rate",
        description: "Sets the interval in seconds for logging render progress",
        args: &[HelpArg { name: "seconds", optional: false, key: "", description: "" }],
    },
    HelpEntry {
        syntax: "progresslogging toggle",
        key: "command.description.progresslogging.toggle",
        description: "Toggles render progress logging on or off",
        args: &[],
    },
    HelpEntry {
        syntax: "radiusrender <world> <radius> [center]",
        key: "command.description.radius-render",
        description: "Starts a radius render",
        args: &[
            PLAIN_WORLD,
            HelpArg { name: "radius", optional: false, key: "", description: "" },
            HelpArg {
                name: "center",
                optional: true,
                key: "command.argument.optional-center",
                description: "Defaults to (<white>0<gray>,</gray> 0</white>) if unspecified",
            },
        ],
    },
    HelpEntry { syntax: "reload", key: "command.description.reload", description: "Reloads the plugin", args: &[] },
    HelpEntry {
        syntax: "resetmap <world>",
        key: "command.description.resetmap",
        description: "Resets the map of a specified world",
        args: &[PLAIN_WORLD],
    },
    HelpEntry { syntax: "show", key: "command.description.show", description: "Show a player on the map view", args: &[] },
    HelpEntry { syntax: "show <player>", key: "command.description.show", description: "Show a player on the map view", args: &[PLAYER_ARG] },
];

/// An open level by its identifier (`LevelParser`): `minecraft:the_nether`, `the_nether` in the `minecraft`
/// namespace, or a mod's `mod:castle`. Other spellings (`nether`) name no level, and a level that is not loaded
/// names none either.
pub fn parse_world(s: &str) -> Option<usize> {
    let id = if s.contains(':') { s.to_owned() } else { format!("minecraft:{s}") };
    runtime::open_world_index(&id)
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
        // The label alone has no handler: cloud answers with the subcommands it could be followed by.
        "" => vec![invalid_syntax(ROOT_SYNTAX)],
        "help" => help(&args.join(" ")),
        "confirm" => confirm(sender, online),
        "fullrender" => match world_arg(args.first().copied(), sender) {
            Err(e) => vec![e],
            Ok(w) => match runtime::start_full_render(w) {
                Ok(()) if sender.player.is_some() => {
                    vec![plain(tr("log.started-full-render", "Started full map render for <world>", &[("world", world_id(w).as_str())]))]
                }
                Ok(()) => vec![],
                Err(e) => vec![refused(e, w)],
            },
        },
        "radiusrender" => radius(args, sender),
        "cancelrender" => match world_arg(args.first().copied(), sender) {
            Err(e) => vec![e],
            Ok(w) => match runtime::cancel_render(w) {
                Ok(()) => vec![msg(tr("cancelled-render", "Render cancelled for <world>", &[("world", world_id(w).as_str())]), GREEN)],
                Err(e) => vec![refused(e, w)],
            },
        },
        "pauserender" => match world_arg(args.first().copied(), sender) {
            Err(e) => vec![e],
            Ok(w) => match runtime::toggle_pause(w) {
                Ok(true) => vec![msg(tr("paused-renders", "Paused renders for <world>", &[("world", world_id(w).as_str())]), GREEN)],
                Ok(false) => vec![msg(tr("unpaused-renders", "Unpaused renders for <world>", &[("world", world_id(w).as_str())]), GREEN)],
                Err(e) => vec![refused(e, w)],
            },
        },
        "resetmap" => reset_request(args, sender, line),
        "progresslogging" => progress(args),
        "hide" | "show" => hide_show(sub == "hide", args.first().copied(), sender, online),
        "reload" => match runtime::reload() {
            Ok(()) => vec![msg(
                tr("plugin-reloaded", "<name> v<version> reloaded", &[("name", "squaremap"), ("version", env!("CARGO_PKG_VERSION"))]),
                GREEN,
            )],
            Err(e) => vec![msg(e, RED)],
        },
        "link" => vec![link(sender)],
        other => vec![msg(format!("Unknown command '{other}'. Try /{} help", main_label()), RED)],
    }
}

/// Commands to a page of the help index (`MinecraftHelp`'s default).
const HELP_PER_PAGE: usize = 6;

/// A caption of the help (`command.message.help.<key>`), English by default.
fn caption(key: &str, default: &str) -> String {
    tr(&format!("command.message.help.{key}"), default, &[])
}

/// The title bar of the help: the title in the primary colour between dashes.
fn help_title() -> Line {
    let bar = "-".repeat(12);
    feedback(vec![
        colored(format!("{bar} "), DARK_GRAY),
        colored(caption("help_title", "squaremap command help"), PRIMARY),
        colored(format!(" {bar}"), DARK_GRAY),
    ])
}

/// `/squaremap help [query]` (cloud's `MinecraftHelp`): a query that is exactly one command's syntax shows it with its
/// description and arguments; any other query lists the commands whose syntax starts with it, six to a page (a number
/// ending the query picks the page); no query lists them all.
fn help(query: &str) -> Vec<Line> {
    let label = main_label();
    let mut words: Vec<&str> = query.split_whitespace().collect();
    let page = match words.last().and_then(|w| w.parse::<usize>().ok()) {
        Some(n) => {
            words.pop();
            n
        }
        None => 1,
    };
    let q = words.join(" ");
    let needle = q.to_ascii_lowercase();
    // Exactly one command (no longer syntax continues it): shown in full.
    let longer = format!("{needle} ");
    let has_more = HELP.iter().any(|e| e.syntax.to_ascii_lowercase().starts_with(&longer));
    if let Some(entry) = HELP.iter().find(|e| !needle.is_empty() && e.syntax.eq_ignore_ascii_case(&needle)) {
        if !has_more {
            return help_verbose(&label, entry);
        }
    }
    let found: Vec<&HelpEntry> = HELP.iter().filter(|e| e.syntax.to_ascii_lowercase().starts_with(&needle)).collect();
    let mut out = vec![help_title()];
    if !needle.is_empty() {
        if found.is_empty() {
            out.push(msg(format!("{}: {q}", caption("no_results_for_query", "No results for query")), GRAY));
            return out;
        }
        out.push(msg(format!("{}: {q}", caption("showing_results_for_query", "Showing search results for query")), GRAY));
    }
    let pages = found.len().div_ceil(HELP_PER_PAGE).max(1);
    if page < 1 || page > pages {
        out.push(msg(
            tr(
                "command.message.help.page_out_of_range",
                "Error: Page <page> is not in range. Must be in range [1, <max_pages>]",
                &[("page", &page.to_string()), ("max_pages", &pages.to_string())],
            ),
            RED,
        ));
        return out;
    }
    out.push(feedback(vec![colored(format!("{}:", caption("available_commands", "Available Commands")), PRIMARY)]));
    for entry in found.iter().skip((page - 1) * HELP_PER_PAGE).take(HELP_PER_PAGE) {
        out.push(help_entry(&label, entry));
    }
    if pages > 1 {
        let base = if q.is_empty() { format!("/{label} help") } else { format!("/{label} help {q}") };
        let mut footer = Vec::new();
        if page > 1 {
            footer.push(clickable(
                colored("\u{2190}", PRIMARY),
                caption("click_for_previous_page", "Click for previous page"),
                Click::Run(format!("{base} {}", page - 1)),
            ));
        } else {
            footer.push(colored("\u{2190}", DARK_GRAY));
        }
        footer.push(colored(format!(" {page}/{pages} "), WHITE));
        if page < pages {
            footer.push(clickable(
                colored("\u{2192}", PRIMARY),
                caption("click_for_next_page", "Click for next page"),
                Click::Run(format!("{base} {}", page + 1)),
            ));
        } else {
            footer.push(colored("\u{2192}", DARK_GRAY));
        }
        out.push(feedback(footer));
    }
    out
}

fn help_entry(label: &str, entry: &HelpEntry) -> Line {
    feedback(vec![clickable(
        colored(format!("/{label} {}", entry.syntax), WHITE),
        caption("click_to_show_help", "Click to show help for this command"),
        Click::Run(format!("/{label} help {}", entry.syntax)),
    )])
}

/// One command's help: its syntax, description and arguments.
fn help_verbose(label: &str, entry: &HelpEntry) -> Vec<Line> {
    let mut out = vec![help_title()];
    let mut command = vec![colored(format!("{}: ", caption("command", "Command")), PRIMARY)];
    command.extend(highlight_special(&format!("/{label} {}", entry.syntax), WHITE));
    out.push(feedback(command));
    out.push(feedback(vec![
        colored(format!("{}: ", caption("description", "Description")), PRIMARY),
        colored(fill_description(&tr(entry.key, entry.description, &[])), GRAY),
    ]));
    if !entry.args.is_empty() {
        out.push(feedback(vec![colored(format!("{}:", caption("arguments", "Arguments")), PRIMARY)]));
        for arg in entry.args {
            let shown = if arg.optional { format!("[{}]", arg.name) } else { format!("<{}>", arg.name) };
            let description = if arg.key.is_empty() {
                caption("no_description", "No description")
            } else {
                fill_description(&tr(arg.key, arg.description, &[]))
            };
            let mut parts = vec![part("  "), colored(shown, WHITE), colored(format!(" - {description}"), GRAY)];
            if arg.optional {
                parts.push(colored(format!(" ({})", caption("optional", "Optional")), DARK_GRAY));
            }
            out.push(feedback(parts));
        }
    }
    out
}

/// A description's MiniMessage with its tags dropped.
fn fill_description(text: &str) -> String {
    fill(text, &[])
}

/// The world a command names, or the sender's own world. Errors come as squaremap prints them.
fn world_arg(arg: Option<&str>, sender: &Sender) -> Result<usize, Line> {
    match (arg, sender.player.as_ref()) {
        (Some(a), _) => parse_world(a).ok_or_else(|| no_such_world(a)),
        (None, Some(p)) => runtime::open_world_index(&p.world)
            .ok_or_else(|| {
                error(vec![colored(
                    tr("command.message.map-not-enabled-for-world", "Map is not enabled for world '<world>'", &[("world", &p.world)]),
                    RED,
                )])
            }),
        (None, None) => Err(error(vec![colored(
            tr("command.message.console-must-specify-world", "You must specify the world when running this command from console", &[]),
            RED,
        )])),
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

/// The part of a `^left` / `^forward` local coordinate after the caret: a number, empty for 0.
fn local_part(s: &str) -> Option<f64> {
    let rest = s.strip_prefix('^')?;
    if rest.is_empty() { Some(0.0) } else { rest.parse::<f64>().ok() }
}

/// The centre of `radiusrender` (`Location2DParser`): `<x> <z>` of numbers or `~` offsets from the sender, or both with
/// `^`: `^left ^forward` from the sender along their facing (`yaw` in degrees, 0 facing south). Mixing the two forms
/// is refused, as is anything that is not a number.
fn center(x: &str, z: &str, base: (f64, f64), yaw: f64) -> Result<[i32; 2], Line> {
    let local = (x.starts_with('^'), z.starts_with('^'));
    if local.0 != local.1 {
        return Err(mixed_local_absolute());
    }
    if local.0 {
        let (Some(left), Some(forward)) = (local_part(x), local_part(z)) else {
            return Err(bad_location(&format!("{x} {z}")));
        };
        let (sin, cos) = yaw.to_radians().sin_cos();
        // Facing south (yaw 0) forward is +z and left is +x; turning right (yaw +90, west) swings both round.
        let (fx, fz) = (-sin, cos);
        let (lx, lz) = (cos, sin);
        return Ok([(base.0 + left * lx + forward * fx).floor() as i32, (base.1 + left * lz + forward * fz).floor() as i32]);
    }
    match (coord(x, base.0), coord(z, base.1)) {
        (Some(x), Some(z)) => Ok([x, z]),
        _ => Err(bad_location(&format!("{x} {z}"))),
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
    let (base, yaw) = sender.player.as_ref().map_or(((0.0, 0.0), 0.0), |p| ((p.x, p.z), p.yaw));
    let center = match (args.get(2).copied(), args.get(3).copied()) {
        (Some(x), Some(z)) => match center(x, z, base, yaw) {
            Ok(c) => c,
            Err(line) => return vec![line],
        },
        (None, None) => [0, 0],
        _ => return vec![invalid_syntax("radiusrender <world> <radius> [center]")],
    };
    match runtime::start_radius_render(w, center, r) {
        Ok(()) => vec![plain(tr("log.started-radius-render", "Started radius map render for <world>", &[("world", world_id(w).as_str())]))],
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
        colored(tr("command.message.confirmation-required", "Confirmation required. Confirm using /<command> confirm.", &[("command", &label)]), RED),
        tr("click-to-confirm", "Click to confirm", &[]),
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
        return vec![msg(tr("command.message.no-pending-commands", "You don't have any pending commands.", &[]), RED)];
    };
    // Only resetmap asks for a confirmation.
    let words: Vec<&str> = line.split_whitespace().collect();
    let Some(w) = words.get(2).and_then(|a| parse_world(a)) else {
        return command(&line, sender, online);
    };
    match runtime::reset_map(w) {
        Ok(()) => vec![msg(tr("successfully-reset-map", "Successfully reset map for <world>", &[("world", world_id(w).as_str())]), GREEN)],
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
                clickable(colored(symbol, color), tr("click-to-toggle", "Click to toggle", &[]), Click::Run(format!("/{label} progresslogging toggle"))),
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
                tr("command.message.progresslogging.enabled", "Render progress logging has been enabled.", &[])
            } else {
                tr("command.message.progresslogging.disabled", "Render progress logging has been disabled.", &[])
            };
            vec![feedback(vec![clickable(
                colored(text, GREEN),
                tr("click-to-toggle", "Click to toggle", &[]),
                Click::Run(format!("/{label} progresslogging toggle")),
            )])]
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
            None => error(vec![colored(
                tr("command.message.console-must-specify-player", "You must specify a target player when running this command from console", &[]),
                RED,
            )]),
            Some(n) => invalid_argument(format!("No player named '{n}' is online")),
        }];
    };
    let hidden = crate::horizon_squaremap::api::player_hidden(uuid);
    match (hide, hidden) {
        (true, true) => vec![msg(tr("command.hide.already-hidden", "<player> is already hidden from map", &[("player", &name)]), RED)],
        (true, false) => {
            crate::horizon_squaremap::api::hide_player(uuid, true);
            vec![msg(tr("command.hide.hidden", "<player> is now hidden from map", &[("player", &name)]), GREEN)]
        }
        (false, false) => vec![msg(tr("command.show.not-hidden", "<player> is not hidden from map", &[("player", &name)]), RED)],
        (false, true) => {
            crate::horizon_squaremap::api::show_player(uuid, true);
            vec![msg(tr("command.show.shown", "<player> is no longer hidden from map", &[("player", &name)]), GREEN)]
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

/// The completions of the world argument (the first one) of the commands that take a world: the open levels'
/// identifiers, and the short name of the `minecraft` ones once something is typed (`MapWorldParser`'s
/// suggestions, which list the enabled ones). Empty for any other line.
pub fn world_completions(command: &str) -> Vec<String> {
    const WORLD_SUBS: [&str; 5] = ["fullrender", "radiusrender", "cancelrender", "pauserender", "resetmap"];
    let words: Vec<&str> = command.split(' ').collect();
    if words.len() != 3 || !is_ours(command) || !WORLD_SUBS.contains(&words[1].to_ascii_lowercase().as_str()) {
        return Vec::new();
    }
    let typed = words[2].to_ascii_lowercase();
    // `resetmap` names any open level; the others only those whose map is enabled (`MapWorldParser`).
    let resetmap = words[1].eq_ignore_ascii_case("resetmap");
    let mut out = Vec::new();
    for (_, id) in runtime::open_levels() {
        if !resetmap && !runtime::config().world(&id).map.enabled {
            continue;
        }
        let short = id.strip_prefix("minecraft:").unwrap_or(id.as_str());
        // A level outside `minecraft:` has no short name: its one candidate is listed once.
        let candidates: Vec<&str> = if typed.is_empty() || short == id.as_str() { vec![id.as_str()] } else { vec![short, id.as_str()] };
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
    runtime::open_levels()
        .into_iter()
        .filter_map(|(w, id)| {
            runtime::render_status(w).map(|(mode, done, total, paused)| {
                let m = if mode == Mode::Full { "full" } else { "radius" };
                format!("{id}: {m} render {done}/{total} chunks{}", if paused { " (paused)" } else { "" })
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    #[test]
    fn command_failed_hover_only_with_detail() {
        let plain = command_failed(None);
        assert!(plain.parts[0].hover.is_none());
        assert!(plain.prefixed);
        let with = command_failed(Some("boom"));
        assert_eq!(with.parts[0].hover.as_deref(), Some("boom"));
        assert_eq!(STACKTRACE_NODE, "squaremap.command-exception-stacktrace");
    }

    use super::*;

    fn texts(lines: &[Line]) -> Vec<String> {
        lines.iter().map(Line::text).collect()
    }

    fn console() -> Sender {
        Sender { player: None, permission: 4, others: true }
    }

    fn player(level: u8) -> Sender {
        Sender {
            player: Some(SenderPlayer { uuid: 7, name: "Alex".into(), world: "minecraft:the_nether".into(), x: 100.7, z: -3.2, yaw: 0.0 }),
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

    /// Opens the levels the tests use: the vanilla three and a mod's level. The registry is process-wide, so the
    /// tests that read it hold [`runtime::REGISTRY_TESTS`] and open this same list.
    fn open_test_levels() -> std::sync::MutexGuard<'static, ()> {
        let guard = runtime::REGISTRY_TESTS.lock().unwrap_or_else(|e| e.into_inner());
        runtime::set_open_levels(&["minecraft:overworld", "minecraft:the_end", "minecraft:the_nether", "mod:castle"].map(String::from));
        guard
    }

    #[test]
    fn worlds_and_coordinates() {
        let _levels = open_test_levels();
        let nether = parse_world("minecraft:the_nether");
        assert!(nether.is_some());
        assert_eq!(parse_world("the_nether"), nether);
        assert_eq!(nether.map(runtime::world_name).as_deref(), Some("minecraft:the_nether"));
        assert_eq!(parse_world("mod:castle").map(runtime::world_name).as_deref(), Some("mod:castle"));
        assert_eq!(parse_world("castle"), None);
        assert_eq!(parse_world("nether"), None);
        assert_eq!(parse_world("mars"), None);
        assert_eq!(coord("~", 100.7), Some(100));
        assert_eq!(coord("~-5", 0.5), Some(-5));
        assert_eq!(coord("-3.5", 0.0), Some(-4));
        assert_eq!(coord("~", 0.0), Some(0));
        assert_eq!(world_arg(None, &player(2)).ok(), nether);
        assert!(world_arg(None, &console()).is_err());
    }

    #[test]
    fn world_names_for_the_first_argument() {
        let _levels = open_test_levels();
        assert_eq!(
            world_completions("/squaremap fullrender "),
            ["minecraft:overworld", "minecraft:the_end", "minecraft:the_nether", "mod:castle"]
        );
        assert_eq!(world_completions("map resetmap the"), ["the_end", "the_nether"]);
        assert_eq!(world_completions("map resetmap mod"), ["mod:castle"]);
        assert!(world_completions("squaremap hide ").is_empty());
        assert!(world_completions("squaremap fullrender minecraft:the_end ").is_empty());
    }

    #[test]
    fn messages_without_a_running_map() {
        let _levels = open_test_levels();
        assert_eq!(
            texts(&command("squaremap fullrender", &player(0), &[])),
            ["I'm sorry, but you do not have permission to perform this command.\nPlease contact the server administrators if you believe that this is in error."]
        );
        // The label alone has no handler: the answer lists what could follow it.
        assert_eq!(
            texts(&command("squaremap", &player(0), &[])),
            ["Invalid command syntax. Correct command syntax is: /squaremap <cancelrender|confirm|fullrender|help|hide|link|pauserender|progresslogging|radiusrender|reload|resetmap|show>"]
        );
        // `help link` is exactly one command: its title, syntax and description (no arguments); `help hide` is two.
        assert_eq!(command("map help link", &player(2), &[]).len(), 3);
        assert_eq!(command("map help hide", &player(2), &[]).len(), 5);
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
    fn help_lists_six_to_a_page_and_shows_one_command_in_full() {
        let first = texts(&help(""));
        assert_eq!(first.len(), 9, "{first:?}");
        assert_eq!(first[0], "------------ squaremap command help ------------");
        assert_eq!(first[1], "Available Commands:");
        assert_eq!(first[2], "/squaremap cancelrender [world]");
        assert_eq!(first[7], "/squaremap hide <player>");
        assert_eq!(first[8], "\u{2190} 1/3 \u{2192}");
        let last = texts(&help("3"));
        assert_eq!(last[2], "/squaremap reload");
        assert_eq!(last[last.len() - 1], "\u{2190} 3/3 \u{2192}");
        assert!(texts(&help("9"))[1].starts_with("Error: Page 9 is not in range"));
        let found = texts(&help("progresslogging"));
        assert_eq!(found[1], "Showing search results for query: progresslogging");
        assert_eq!(found.len(), 6);
        assert_eq!(texts(&help("nothing"))[1], "No results for query: nothing");
        let one = texts(&help("hide <player>"));
        assert_eq!(one[1], "Command: /squaremap hide <player>");
        assert_eq!(one[2], "Description: Hide a player from the map view");
        assert_eq!(one[3], "Arguments:");
        assert_eq!(one[4], "  <player> - Defaults to the executing player if unspecified (console must specify a player)");
        let center = texts(&help("radiusrender <world> <radius> [center]"));
        assert_eq!(center[6], "  [center] - Defaults to (0, 0) if unspecified (Optional)");
        assert_eq!(center[4], "  <world> - No description");
    }

    #[test]
    fn language_file_replaces_words_and_fills_placeholders() {
        assert_eq!(fill("<red>No renders running for <world></red>", &[("world", "minecraft:the_end")]), "No renders running for minecraft:the_end");
        assert_eq!(fill("a < b", &[]), "a < b");
        let file = "render-not-in-progress: '<red>Kein Render fuer <world>'\ncommand:\n  message:\n    no-pending-commands: Nichts ausstehend\n";
        let lang = crate::horizon_squaremap::yaml::flatten(&crate::horizon_squaremap::yaml::parse(file).unwrap());
        assert_eq!(tr_from(Some(&lang), "render-not-in-progress", "No renders running for <world>", &[("world", "x")]), "\u{1}<red>Kein Render fuer x");
        assert_eq!(tr_from(Some(&lang), "command.message.no-pending-commands", "You don't have any pending commands.", &[]), "\u{1}Nichts ausstehend");
        // The line the player gets: the file's colour wins over the command's default, the tags are gone.
        let line = msg(tr_from(Some(&lang), "render-not-in-progress", "No renders running for <world>", &[("world", "a<b")]), GREEN);
        assert_eq!(line.text(), "Kein Render fuer a<b");
        assert_eq!(line.expanded()[0].color, Some(RED));
        let plain_line = msg(tr_from(Some(&lang), "command.message.no-pending-commands", "x", &[]), GREEN);
        assert_eq!(plain_line.expanded()[0].color, Some(GREEN));
        // A key the file lacks keeps the English text.
        assert_eq!(tr_from(Some(&lang), "cancelled-render", "Render cancelled for <world>", &[("world", "x")]), "Render cancelled for x");
        assert_eq!(tr_from(None, "cancelled-render", "Render cancelled for <world>", &[("world", "y")]), "Render cancelled for y");
    }

    #[test]
    fn center_reads_numbers_relative_and_local_coordinates() {
        assert_eq!(center("10", "-3.5", (0.0, 0.0), 0.0).ok(), Some([10, -4]));
        assert_eq!(center("~", "~5", (100.7, 20.2), 0.0).ok(), Some([100, 25]));
        // Facing south (yaw 0): forward is +z, left is +x.
        assert_eq!(center("^", "^10", (0.5, 0.5), 0.0).ok(), Some([0, 10]));
        assert_eq!(center("^3", "^", (0.5, 0.5), 0.0).ok(), Some([3, 0]));
        // Facing west (yaw 90): forward is -x, left is +z.
        assert_eq!(center("^", "^10", (0.5, 0.5), 90.0).ok(), Some([-10, 0]));
        assert_eq!(center("^5", "^", (0.5, 0.5), 90.0).ok(), Some([0, 5]));
        assert_eq!(center("^1", "2", (0.0, 0.0), 0.0).err().map(|l| l.text()), Some("Invalid command argument: Cannot mix local and absolute coordinates. (either all coordinates use '^' or none do)".into()));
        assert_eq!(center("a", "2", (0.0, 0.0), 0.0).err().map(|l| l.text()), Some("Invalid command argument: 'a 2' is not a valid location. Required format is '<x> <z>'".into()));
    }

    #[test]
    fn resetmap_asks_then_confirm_runs_it() {
        let _levels = open_test_levels();
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
