# hmod port: squaremap
Round 1
- Fixed:
  - `src/lib.rs` `NODES` + `node(name)` helper: typed Boolean nodes for every node the map checks.
  - `src/lib.rs` `permission_gather_nodes` (new hook): declares the 13 nodes at Level 2 (`commands::OP_LEVEL`).
  - `src/lib.rs` `register_commands` / `player_command` / `run_mod_command`: `permission_node(...)` (still a string in the core) is converted to a typed node at the call site; `has_permission` replaced by `permission_bool(...).unwrap_or(false)`, for the subcommand node and both `.others` nodes.
  - `src/lib.rs` `publish_players`: `nickname` has no Horizon main counterpart; `display_name: None` with a `HMOD-GAP` comment.
  - Imports added for the permission types.
  - `src/horizon_squaremap/commands.rs` is unchanged: its string-returning helpers and their tests stay as they were.
- Permission nodes: `squaremap.command.{help,cancelrender,fullrender,pauserender,radiusrender,resetmap,progresslogging,hide,show,reload,link}` and `squaremap.command.hide.others`, `squaremap.command.show.others` -> Level 2, declared in `src/lib.rs` `permission_gather_nodes` from `NODES`.
- Gaps:
  - HMOD-GAP(nickname): Horizon main has `set_nickname` but no getter. The map's `display_name` is always the real name.
- Unsure:
  - `NODES` is a fixed list, not derived from `commands::usages()`, because `usages()` reads the runtime config (not loaded at gather time). If the label or the command set changes, `NODES` must follow.
  - The `.others` nodes had no declared default before. I chose Level 2 to match the subcommand nodes.
  - `usage_permission` (the root node `squaremap.command`) is not declared. Only the bare label uses it, and that path has no node in `register_commands`, so it is unused.
