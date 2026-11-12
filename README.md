> **Frozen branch (`horizon-plain`).** This is the version written for horizon-rust/horizon's native hooks / horizon-mixins. We stopped working on it on 2026-11-12: horizon-rust moved all mod API work to the fork [arismoko/horizon](https://github.com/arismoko/horizon) (see "Mod-related pull requests" in its CONTRIBUTING.md) and closed mod pull requests there. This branch is experimental and unsupported. The maintained version, for arismoko/horizon only, is on the default branch.

# horizon-squaremap

> **Unofficial Rust port of squaremap for the Horizon server (https://github.com/horizon-rust/horizon). Not affiliated with or endorsed by the squaremap authors.**

A web map for [Horizon](https://github.com/horizon-rust/horizon) dedicated servers: a
rebuild of [squaremap](https://github.com/jpenilla/squaremap) as a Horizon native mod
(a horizon-mixins bundle). It renders the overworld, the nether and the end into
squaremap's tile format, publishes player positions, the spawn and the world border, and
serves squaremap's own web frontend from an internal web server.

Not affiliated with squaremap or its authors.

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
see [`mixins/squaremap/core/squaremap-server.toml`](mixins/squaremap/core/squaremap-server.toml).
`world-settings.default` applies to every world, `world-settings."minecraft:the_nether"`
overrides single keys of it. Changes are picked up when the file is saved, except the web
server's `bind`/`port` and the web folder, which are read when the world opens.

The web folder defaults to `squaremap/web` inside the world folder.

## Not ported

- The update checker and language files (English messages are built in, with squaremap's
  colours, hovers and clicks; the help layout and some wording are guesses, see GAPS.md).
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

The bundle is `mixins/squaremap/`: a crate (`core/`, the renderer, tiles, JSON and web
server, std + `png`, `serde`, `serde_json`, `toml` from Horizon's lock file), a support
module (`glue.rs`, the game side) and eleven HEAD hooks on named hook points
(`register_configs`, `config_loading`, `config_reloading`, `world_lifecycle`,
`server_tick`, `chunk_load`, `chunk_unload`, `block_changed`, `register_commands`,
`perform_command`, `perform_console_command`). The crate also uses `flate2` (in Horizon's
lock file) to read region files.

Since 0.3.0 (unbuilt) the glue needs the `hooks-all` helpers: loaded chunks of every level
(the one in place and open levels nobody is in) are drawn live from `native_chunk_snapshot`,
block colours come from `native_block_map_color` (the vanilla `MapColor` table in
`core/src/color.rs` is only a fallback), biome colours from `native_biome_tint`, the border
marker from `native_world_border(dimension)`, players from `native_server_players` with
`native_effects`, `native_player_equipment`, `native_attribute` and `native_max_players`.
Region files are only read for chunks that are not loaded.

```sh
# Horizon source at the commit in horizon-mixin.toml (replace PIN with it)
horizon-mixins apply --workspace horizon --mods mods --out stage   # mods/squaremap = this bundle
cd stage && cargo build --profile fast -p horizon-client --no-default-features
cargo test -p horizon-squaremap                                     # unit tests of the crate
```

`tools/gen_assets.py` regenerates `core/src/assets.rs` after the frontend in `core/web/`
changes. `tools/compare_tiles.py REF PORT` compares two tile folders pixel by pixel.

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

## Tested (0.1.0, Horizon `69fab597a`)

On a throwaway dedicated server with a fresh world (forceloaded 256 overworld and 64 nether
chunks), against squaremap 1.4.3 on Paper 26.3 with the same config:

- **Tiles against squaremap, same world data** (the Horizon world copied to Paper and
  rendered there with `/squaremap fullrender`): 99.79 % of 130,304 overworld pixels and
  99.83 % of 36,864 nether pixels identical at zoom 3, mean channel difference 0.02 and
  0.13; both drew exactly the same pixels. The rest: biome blending at the edge of the
  generated area and near biome borders, and nether lava that flowed on Paper after loading.
- **Same seed, each server generating its own world**: 98.20 % identical; the rest are
  trees that Horizon's world generation placed differently.
- New chunks (spawn area and forceloaded ones) are rendered; a `/fill` of 16x16 blocks is
  re-rendered within one background interval; nether renders below the roof.
- `markers.json` follows `/worldborder set` and `/setworldspawn`; `players.json` lists a
  joined player with health and armour points and drops them while invisible, in
  spectator mode or wearing a carved pumpkin.
- HTTP: frontend, JSON, tiles (empty 200 for a missing tile), 404s, `..` refused; bound to
  `127.0.0.1` only. An unmodded engine of the same commit opens no web port.

Unit tests (`cargo test -p horizon-squaremap`): colours, config merging, tables, the scanner
on synthetic levels (shading, water depth, glass, ceilings, limits), tile zoom writing,
JSON shapes and the web server.

## Untested: 0.2.0 features (written without a build)

Everything added after 0.1.0 has unit tests in the crate only (NBT and region file
reading with synthetic chunks, plans, checkpoints, spiral order, a render job over a
synthetic region file, visibility shapes, iterate-up, compression, commands without a
running map, the API's JSON). The game side (`glue.rs`, the four new hooks) has not been
compiled. To prove first, on a throwaway server:

- the build itself; `/squaremap fullrender` on a world made before the mod (tiles appear,
  compare against 0.1.0's live tiles with `tools/compare_tiles.py`), stop the server
  mid-render and see it resume; `cancelrender`, `pauserender`, `radiusrender ~ ~`;
- the region-file reader through the renderer (an opt-in test,
  `SQUAREMAP_REGION=<r.x.z.mca> cargo test -- --ignored probe`, decoded four region files a
  Horizon dedicated server wrote, overworld and nether, 445 full chunks: every column's
  height from the blocks matched the saved `WORLD_SURFACE` heightmap; colours and biomes
  were not compared);
- `hide`/`show` (players.json), `resetmap` + `confirm`, `reload` (web server rebinds),
  a `circle` and a `world-border` limit, `iterate-up` in the nether;
- that the guest body's armour attribute counts worn armour.

## Untested (0.1.0)

- The frontend in a browser (only its HTTP requests were checked).
- The end; worlds with data-pack biomes; long uptimes and many players; server-thread cost
  under load (it is a few reference-count clones per chunk, measured nowhere).
- The web server disabled (JSON written to disk then). Config reload was tried for one key
  (`ui.coordinates`): `settings.json` changed within seconds of saving the file.
- A world folder whose tiles were made by squaremap itself (same format, not tried).

## Contributing

Pull requests and improvements are always welcome! Open an issue or a PR -- direct pushes to main are disabled, every change goes through review.
