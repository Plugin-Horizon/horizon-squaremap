# hmod port: squaremap
Parity: before ~60% -> after ~79% now / ~79% with our PRs (118/150 items; method: audit counts of commands, nodes, config keys, message keys and behaviours, estimated, blocked-by-API items count as not done)
Round 4 (100 % brief, blind, no compile; hmod-format branch)
- Fixed: `map-invisibility-equipment` reads the item tag `minecraft:map_invisibility_equipment` through `registry_entry_in_tag` (the query veinminer-cr uses); the vanilla entry (carved pumpkin) answers only when the host gives none. `src/lib.rs` `is_map_invisibility_item`, `has_map_invisibility_equipment`. README Known gaps updated.
- Checked against the original this round, already matching (the audit file's bugs 5-7 and gap list were older than these commits): spawn marker, `invisible` hide rule, `show-armor`/`show-health` in `players.json`, the newline in the no-permission text, bare `/squaremap` answer, `link` text and uuid form, `hide`/`show` messages, `log.started-full-render` for the console.
- Checklist round 100 (ticked = matches the original as read; open = not done):
  - [x] permission nodes (13, Level 2 default as port; original unregistered = op default)
  - [x] commands `help`, `fullrender`, `radiusrender` (args), `cancelrender`, `pauserender`, `resetmap`+`confirm`, `progresslogging`, `hide`, `show`, `reload`, `link`
  - [x] messages: render-in-progress, render-not-in-progress, cancelled-render, successfully-reset-map, paused/unpaused, no-such-world, map-not-enabled, confirmation-required, no-pending-commands, progresslogging.*, hide/show, link format, plugin-reloaded, command.prefix, command.description.*, exception.no-permission, invalid-argument, invalid-syntax
  - [x] spawn marker, invisible rule, map-invisibility-equipment rule (this round), show-armor / show-health
  - [ ] invalid-sender-type and command-execution messages: not reachable (no original command declares a sender type; the port has no exception path)
  - [ ] `resetmap` for a level with the map disabled (original resets it; port refuses): needs the reset path to work without an enabled world
  - [ ] every level, not only the three vanilla dimensions (Horizon `Levels::list_levels()` now exists, so the list is there; the fixed `DIMENSIONS`/`WORLDS`/`SHAPES` tables and the runtime still need a level registry)
  - [ ] `radiusrender`/`hide`/`show` selector grammar and coordinate completion (engine-side selector parser, not on Horizon)
  - [ ] nickname getter for `use-display-names` (API-blocked: only `player_set_nickname` exists)
  - [ ] loaded-chunk snapshot for background redraw (API-blocked: no chunk block read on the host; see GAPS_UPSTREAM.md line for squaremap)
  - [ ] biome zoom / swamp noise, LZ4 region files, language-file colours and click tags, cloud-style help layout (known, see README)
Round 3 (blind, no compile)
- Fixed: background redraw of changed chunks (`src/horizon_squaremap/runtime.rs` `due_chunk`, `world_saved`, `Work::Disk`; `src/lib.rs` `world_saved` hook): a changed chunk that is still loaded waits for the next world save and is then drawn by the render thread from the region files; one that unloaded meanwhile is drawn from them at once. Before, the mark was consumed and the tile redrawn from the old save.
- Fixed: the bare label answers `Invalid command syntax. Correct command syntax is: /squaremap <cancelrender|confirm|...>` (cloud's answer), with the special characters in white; `invalid_syntax` highlights them for every command.
- Fixed: `/squaremap help` is paged (six to a page, page number ends the query), has a title bar, `Available Commands:`, search-result captions, `\u{2190} n/m \u{2192}` page links, and one command's `Command`/`Description`/`Arguments` view (entries sorted by syntax as cloud's index; `hide` and `show` list `<player>` as a second entry). The bar and arrows are written from memory of cloud-minecraft-extras: the layout is close, not byte-identical.
- Fixed: `radiusrender`'s centre reads `<x> <z>` as numbers, `~` offsets or `^left ^forward` local coordinates (along the sender's facing); mixing the forms and bad input give squaremap's location messages (`commands::center`).
- Added: `plugins/squaremap/config.yml` and `advanced.yml` are read (a small YAML reader, `yaml.rs`) and laid over `config/squaremap-server.toml`; `settings.language-file` and `plugins/squaremap/locale/<file>` replace the command messages by their original keys (`render-in-progress`, `command.message.*`, `command.hide.*`, `plugin-reloaded`, the help captions `command.message.help.*`, ...). MiniMessage colour/click tags in a language file are dropped: the wording and the `<placeholders>` follow the file, the colours stay squaremap's defaults. `/squaremap reload` reads the language file again.
- Left open: more than the three dimensions and `resetmap` on any level (`WORLDS`, `SHAPES` and the JSON tables are fixed to three levels; needs a level registry through the whole runtime); `hide`/`show` take Horizon's `player` argument, so the selector grammar and its error texts are the engine's, not squaremap's; custom colours and click tags of language files; region compression LZ4; biome zoom/swamp noise for saved chunks; nickname.
- Unsure: (1) cloud sorts a command node's children alphabetically (the `<...|...>` list). (2) `Part.hover` is now an owned `String`. (3) `WorldSavedEvent` fires after the chunks reached the region files. (4) `entity.rotation(host)` yields `[yaw, pitch]`.
Round 1
- Fixed:
  - `src/lib.rs` `NODES` + `node(name)` helper: typed Boolean nodes for every node the map checks.
  - `src/lib.rs` `permission_gather_nodes` (new hook): declares the 13 nodes at Level 2 (`commands::OP_LEVEL`).
  - `src/lib.rs` `register_commands` / `player_command` / `run_mod_command`: `permission_node(...)` (still a string in the core) is converted to a typed node at the call site; `has_permission` replaced by `permission_bool(...).unwrap_or(false)`, for the subcommand node and both `.others` nodes.
  - `src/lib.rs` `publish_players`: `nickname` has no Horizon main counterpart; `display_name: None` with a `HMOD-GAP` comment.
  - Imports added for the permission types.
  - `src/horizon_squaremap/commands.rs` is unchanged: its string-returning helpers and their tests stay as they were.
- Permission nodes: `squaremap.command.{help,cancelrender,fullrender,pauserender,radiusrender,resetmap,progresslogging,hide,show,reload,link}` and `squaremap.command.hide.others`, `squaremap.command.show.others` -> Level 1 (any operator, as Bukkit's default for an unregistered node; Level 2 in round 1), declared in `src/lib.rs` `permission_gather_nodes` from `NODES`.
- Gaps:
  - HMOD-GAP(nickname): Horizon main has `set_nickname` but no getter. The map's `display_name` is always the real name.
  - HMOD-GAP(chunk_snapshot): no chunk snapshot API; changed loaded chunks are not redrawn in the background.
  - HMOD-GAP(item_in_tag): no item tag query; the `map-invisibility-equipment` rule uses the vanilla tag's entry (carved pumpkin).
- Unsure:
  - `NODES` is a fixed list, not derived from `commands::usages()`, because `usages()` reads the runtime config (not loaded at gather time). If the label or the command set changes, `NODES` must follow.
  - The `.others` nodes had no declared default before. I chose Level 2 to match the subcommand nodes.
  - `usage_permission` (the root node `squaremap.command`) is not declared. Only the bare label uses it, and that path has no node in `register_commands`, so it is unused.
