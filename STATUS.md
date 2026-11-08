# Status (squaremap port, 2026-11-08)

- Commands: squaremap's command table (`help`, `confirm`, `fullrender`, `radiusrender`, `cancelrender`,
  `pauserender`, `resetmap`, `progresslogging` with `toggle`/`rate`, `hide`/`hide <player>`,
  `show`/`show <player>`, `reload`, `link`) is registered with typed usages, and the world argument is
  completed (`word?` + `tab_complete`).
- Permission nodes: `squaremap.command.<sub>` per subcommand (`help` included), operator level 2 by default;
  `confirm` has none; `hide`/`show` with a target check `.others`. The bare label carries
  `squaremap.command` (level 0) so the host's root gate stays open. See GAPS.md.
- Messages: squaremap's texts, colours, hovers and clicks (`Messages.java`); `[squaremap]` prefix on errors.
- Defaults: checked against the source's `Config.java`/`WorldConfig.java`; differences are in GAPS.md.
- Not built or run (no cargo by brief). Changed in this pass: `core/src/commands.rs`, `glue.rs`,
  `tab_complete.rs` (new), `horizon-mixin.toml`, `README.md`, `GAPS.md`, `NOTICE` (new).
