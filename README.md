# horizon-squaremap

> **Unofficial Rust port of squaremap for the Horizon server (https://github.com/arismoko/horizon). Not affiliated with or endorsed by the squaremap authors.**

Not affiliated with squaremap or its authors.

> **Built for and working only with [arismoko/horizon](https://github.com/arismoko/horizon)** (the Horizon fork that carries the mod API). [horizon-rust/horizon](https://github.com/horizon-rust/horizon) is experimental for mods and not supported: the earlier version for its native hooks / horizon-mixins is frozen on the `horizon-plain` branch (work stopped on 2026-11-12).

## Which Horizon

This is a native trait mod: `horizon-mod.toml`, a `Cargo.toml` that depends on `horizon-mod-api`, `src/lib.rs` with the hooks, and the plugin logic in the `core/` crate (plain Rust, no engine needed for its tests).

- Pin: `[native] horizon = "54090ee36c4e2c7b43b66db864220c577b94b7e0"` in `horizon-mod.toml` (game `26.3`). That is a development build of arismoko/horizon `main` plus mod API additions that are not all merged upstream yet, so a stock `main` may lack some host methods this mod calls until they land.
- It compiled and loaded on that build and was smoke-tested there; it has not seen a long run with real players. Treat it as experimental.
- Feature descriptions below were written for the earlier horizon-rust version; where they name engine calls (`native_*`) or old file paths, read them as descriptions only. The Known gaps list is current.

## What it does

- **Tiles.** Chunks are rendered with squaremap's algorithm: map colours per block state,
  heightmap shading, biome tinting of grass, foliage, dry foliage and water with blending,
  clear water with depth, lava checkerboard, clear glass, invisible blocks, colour
  overrides, a max height, `iterate-up` for ceiling levels and visibility limits
  (rectangle, circle, polygon, world border). Each region is one
  512x512 PNG at the maximum zoom, and lower zooms are written into the same tile tree
  (`tiles/<world>/<zoom>/<x>_<z>.png`), so tiles squaremap rendered before stay valid.
- **What gets rendered.** Newly generated chunks (squaremap's `ChunkPopulateEvent`) and
  chunks whose blocks change (`Level.setBlock`), in batches every
  `render.background.interval-seconds`, at most `max-chunks-per-interval` per batch.
  A changed chunk that unloads before its turn is rendered when it loads again.
- **Full and radius renders** (`/squaremap fullrender`, `radiusrender`): the chunks already
  saved in the world's region files (`dimensions/<ns>/<level>/region/r.<x>.<z>.mca`) are
  read and drawn on a thread of their own, region by region in squaremap's spiral order;
  loaded chunks are drawn from memory instead. Progress is saved after each region in
  squaremap's `resume_render.json` format (`squaremap/data/<world>/` in the world folder)
  and a render the server stopped resumes on the next start. Progress lines as squaremap
  prints them (`render-progress-logging`).
- **Rendering off the server thread.** The server thread only clones the chunk and its
  eight neighbours (reference-counted, copy-on-write); a render thread scans them and
  writes the tiles. At most 256 snapshots wait at once.
- **Markers and players.** `markers.json` per world (spawn icon, world border polyline,
  layers other mods add), `players.json` (name, display name, position, heading, health,
  armour points), hiding spectators, invisible players, players wearing map-invisibility
  equipment and players hidden with `/squaremap hide` (kept in
  `squaremap/data/hidden-players.json`).
- **Commands.** `/squaremap` (alias `/map`, `settings.commands`): `help [query]`,
  `fullrender [world]`, `radiusrender <world> <radius> [x z]` (`~` for your position),
  `cancelrender [world]`, `pauserender [world]`, `resetmap <world>` + `confirm`,
  `progresslogging [toggle | rate <seconds>]`, `hide [player]`, `show [player]`, `reload`,
  `link`. Worlds are `overworld`, `the_nether`, `the_end` (or `minecraft:...`). Every
  command but `help` checks `squaremap.command.<sub>` (`native_has_permission`, which a
  permissions mod can grant); without one, operator level 2 is needed. From the server
  console too.
- **API for other mods** (`horizon_squaremap::api`): layers with icon, polyline, rectangle,
  circle, ellipse, polygon and multi-polygon markers and squaremap's marker options,
  registered icons, hiding players. Serialized as squaremap's `MarkerJsonSerializer` does.
- **Image compression** (`settings.image-quality.compress-images`): deflate level
  `9 * value`, as ImageIO's PNG writer does with squaremap's setting.
- **Web server.** `std::net`, GET/HEAD only, the JSON documents from memory, files from
  the web folder, binds `127.0.0.1` by default. The frontend is squaremap 1.4.3's,
  written into the web folder on start (`web-directory.auto-update`).

## Config

`config/squaremap-server.toml` (a world's `serverconfig/squaremap-server.toml` wins), written
with the defaults on first start. Keys follow squaremap's `config.yml` and `advanced.yml`;
see [`core/squaremap-server.toml`](core/squaremap-server.toml).
`world-settings.default` applies to every world, `world-settings."minecraft:the_nether"`
overrides single keys of it. Changes are picked up when the file is saved, except the web
server's `bind`/`port` and the web folder, which are read when the world opens.

The web folder defaults to `squaremap/web` inside the world folder.

## Not ported

- The update checker and language files (English messages are built in, with squaremap's
  colours, hovers and clicks; the help layout and some wording are guesses, see the Known gaps section).
- `progresslogging toggle|rate` change the running server only; squaremap writes them
  back to its config.
- Region files are read for chunks compressed with gzip, zlib or not at all (vanilla's
  default is zlib); LZ4 and custom compression are skipped, as are pre-1.18 chunks that
  need below-zero retrogen (squaremap has the server upgrade those).
- The API is Rust functions for mods built into the same server; there is no Java API.
  Its layers use fixed labels (no per-viewer supplier).
- Armour points are the player's `minecraft:armor` attribute (`native_attribute`, equipment
  modifiers included) where the player has a body; otherwise vanilla's armour materials by
  the worn items' names (`native_player_equipment`).
- Swamp grass of live chunks is sampled per column of the 3x3 snapshot from
  `native_biome_tint` (a blend reaching further reads the nearest column); chunks drawn from
  region files still use the game's grass noise directly, as the helpers can't be called
  off the server thread. Biome climate and grass modifiers come from the world's biome table
  (the tint doesn't carry them).
- Biomes in chunks outside the 3x3 snapshot (a blend reaching into an unloaded chunk) are
  read from the region files; a chunk never generated reads the nearest cell of the
  rendered chunk, where squaremap asks the biome source.
- Full/radius renders use one thread per world (squaremap's `workers` and other
  `full-render`/`radius-render` limits are not read).

## Build

Build it as a native trait mod with arismoko's launcher (see the "native mods" how-to in the arismoko/horizon docs):

```sh
horizon-launcher native-build <path to this folder> --server-output <dir> --version <tag> --trust-native-code
```

`--trust-native-code` is your consent to compile and run this code inside the server. The logic crate tests run without the engine: `(cd core && cargo test)`.

## Known gaps

What does not work yet on arismoko/horizon (each is marked `TODO(arismoko-api: ...)` in `src/lib.rs`):

- The web renderer does not start yet: arismoko has no block-state, biome or biome-zoom tables to render from, so commands answer and the config is read, but no tiles are drawn.
- Loaded chunks are not snapshotted; tiles come from the region files.
- No spawn marker; the invisibility and equipment hide rules for players are not applied.

## Licence

Derived from squaremap (https://github.com/jpenilla/squaremap, MIT). Notices of the original
authors are in [NOTICE](NOTICE) and [LICENSE](LICENSE).

MIT, see [LICENSE](LICENSE). squaremap is MIT licensed (Copyright (c) 2021-2024 Jason
Penilla, 2020-2021 William Blake Galbreath); its source was read and its rendering,
tile, marker and JSON logic ported (and, for 0.2.0, its commands, `RenderPlan`/`RenderJob`/
`RenderCheckpointStore`/`RenderProgressReporter`, `SpiralIterator`, the visibility shapes,
`ChunkScanner.iterateUp`, `MarkerJsonSerializer`, the API's marker types and options,
`AbstractPlayerManager`, `PlayerDataPublisher` and `Messages`, all at v1.4.3), so its notice is kept in `LICENSE` and in the files
that carry ported code. The web frontend in `core/web/` is squaremap's built frontend,
unchanged; it bundles Leaflet (BSD 2-Clause, notice in `LICENSE`). No clean room was
needed.
