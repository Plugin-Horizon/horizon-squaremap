# Gaps against squaremap (source, 2026-11-08)

Reference: `github.com/jpenilla/squaremap` at commit `2e158c2` (shallow clone, MIT). Compared:
`common/.../command/Commands.java` and `command/commands/*.java`, `command/ExceptionHandler.java`,
`command/argument/parser/*.java`, `config/Messages.java` (the English texts are the defaults there),
`config/Config.java`, `config/WorldConfig.java`, `config/RenderConfig.java`.

## Fixed in this pass

- Permission nodes: `help` has `squaremap.command.help` (operator level by default, as on Paper); `confirm`
  has none, so anyone may run it. `hide`/`show` with a target need `squaremap.command.hide.others` /
  `show.others` (`glue.rs` `perform_command` -> `Sender.others`, `commands.rs` `hide_show`).
- Root gate: the host gates a root command by the first usage that has a node (`native_server.rs`
  `native_command_nodes`). The bare label now carries `squaremap.command` at level 0 (`ROOT_NODE`), which
  stays open unless a permissions mod denies it, so a denied `squaremap.command.<sub>` no longer hides the
  whole root. This node is not in squaremap.
- A permissions mod that denies a node now refuses an operator too (`perform_command` used to fall back to
  the operator level).
- Messages: texts, colours, hovers and clicks follow `Messages.java`. The `[squaremap]` prefix goes on
  errors only (exception handler), not on feedback.
- Confirmation: 15 seconds (was 30), clickable message, `main-command-label` in the text.
- `radiusrender`/`progresslogging rate`: numbers are integers from 1 to 2147483647; `~` in the centre
  counts from 0 at the console.
- World arguments: identifiers only (`minecraft:the_nether`, or `the_nether`); `nether`, `world`, `end`
  name no level, as in squaremap's `MapWorldParser`.
- Tab completion: the world argument of `fullrender`, `radiusrender`, `cancelrender`, `pauserender` and
  `resetmap` is `word?`, and `tab_complete` (`tab_complete.rs`, `glue.rs` `tab_complete`) suggests the
  level names. `link` is in the help list.

## Still different

- Help layout: squaremap prints `/squaremap help` through cloud's `MinecraftHelp` (not in the repo). The
  port's layout (a title, one clickable line per entry, the search lines) is a guess. Entries are not
  filtered by permission, and the query gets no suggestions. Argument descriptions are not shown.
- Invalid syntax: cloud's "correct syntax" string is not known. `progresslogging <unknown>` and missing
  arguments print the port's own usage string (`commands.rs` `invalid_syntax`).
- Unknown subcommand text (`commands.rs` `command`, last arm) is not squaremap's; squaremap's cloud
  reports it through vanilla. Permission denial text is squaremap's, but a host that hides a node-gated
  command from the tree may let vanilla answer instead (not verified).
- Player argument: squaremap's single player selector (`@p`, `@a[...]`) is not supported; the host's
  `player` kind gives names. The unknown-name text (`No player named '<name>' is online`) is a guess.
- Centre coordinates: `^` (local) coordinates are not parsed.
- Non-integer numbers are refused by the host before the port sees them (vanilla's text).
- Per-world map enable: squaremap's `map.enabled = false` (and datapack levels) gives "Map is not enabled"
  / "No such world"; the port knows only the three vanilla levels in `WORLDS` and does not read
  `map.enabled` for commands.
- `progresslogging toggle|rate` change the running server only; squaremap writes them to its config. The
  port does not write its config back.
- `reload` prints the port's version (`CARGO_PKG_VERSION`), squaremap prints its own.
- `hide`/`show` keep the port's `.others` check on the host's player list; squaremap's selector targets
  (above) are missing.
- Hover and click texts are only where squaremap has them. The prefix gradient is per letter, which can
  differ slightly from Adventure's gradient.
- Config keys not read: `settings.debug-mode`, `settings.update-checker` (no network check),
  `settings.language-file`, `settings.internal-webserver.flush-json-immediately` (default `false`, not
  implemented), `config-version`, `image-saving` workers and the `render.*` worker/chunk/region/cache
  limits (see README "Not ported").
- Deliberate differences: `internal-webserver.bind` is `127.0.0.1` (squaremap: `0.0.0.0`), and
  `settings.web-directory.path` defaults to `squaremap/web` in the world folder (squaremap: `web` in its
  data folder).
- Config defaults checked against `Config.java`/`WorldConfig.java`: every key the source reads at a literal
  path is in the port's TOML with the same default, except the four keys above and the two deliberate
  ones.

## Hooks

Horizon has `NativeCommand::permission`, `native_has_permission`, `native_feedback` (with `Text` colours
and click events), and the `tab_complete` hook (in `native-hooks.json`). No hook gap found.
