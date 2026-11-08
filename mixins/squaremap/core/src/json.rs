//! The JSON documents the frontend reads, in squaremap's formats (`WebSettingsPublisher`,
//! `MarkerDataPublisher`, `PlayerDataPublisher`; MIT, Copyright (c) 2021-2024 Jason Penilla).
use crate::config::{Config, Layer, WorldConfig};
use serde_json::{json, Value};

/// The three levels a server has, in the order squaremap lists them.
pub const WORLDS: [(&str, &str); 3] = [
    ("minecraft:overworld", "normal"),
    ("minecraft:the_end", "the_end"),
    ("minecraft:the_nether", "nether"),
];

/// `minecraft:overworld` -> `minecraft_overworld` (`WorldIdentifiers.webName`).
pub fn web_name(world: &str) -> String {
    world.replace(':', "_")
}

/// `/tiles/settings.json`.
pub fn settings(config: &Config) -> Value {
    let ui = &config.settings.ui;
    let worlds: Vec<Value> = WORLDS
        .iter()
        .filter_map(|(id, kind)| {
            let w = config.world(id);
            w.map.enabled.then(|| {
                json!({
                    "name": web_name(id),
                    "icon": w.map.icon,
                    "display_name": w.map.display_name.replace("{world}", id),
                    "type": kind,
                    "order": w.map.order,
                })
            })
        })
        .collect();
    json!({
        "worlds": worlds,
        "ui": {
            "sidebar": {
                "player_list_label": "Players ({cur}/{max})",
                "world_list_label": "Worlds",
                "pinned": ui.sidebar.pinned,
            },
            "coordinates": {"html": "Coordinates<br/>{x}, {z}", "enabled": ui.coordinates.enabled},
            "link": {"enabled": ui.link.enabled},
            "title": "squaremap - {world}",
        },
    })
}

/// `/tiles/<world>/settings.json`.
pub fn world_settings(w: &WorldConfig, spawn: [i32; 2]) -> Value {
    let t = &w.player_tracker;
    let n = &t.nameplate;
    json!({
        "player_tracker": {
            "default_hidden": t.default_hidden,
            "z_index": t.z_index,
            "update_interval": t.update_interval_seconds,
            "show_controls": t.show_controls,
            "nameplates": {
                "show_health": n.show_health,
                "heads_url": n.heads_url,
                "show_armor": n.show_armor,
                "show_heads": n.show_head,
                "enabled": n.enabled,
            },
            "label": "Players",
            "priority": t.layer_priority,
            "enabled": t.enabled,
        },
        "spawn": {"x": spawn[0], "z": spawn[1]},
        "marker_update_interval": w.map.markers.update_interval_seconds,
        "zoom": {"def": w.map.zoom.default, "max": w.map.zoom.maximum, "extra": w.map.zoom.extra},
        "tiles_update_interval": w.map.render.background.interval_seconds,
    })
}

fn layer(id: &str, name: &str, l: &Layer, markers: Value, timestamp: u64) -> Value {
    json!({
        "id": id,
        "name": name,
        "control": l.show_controls,
        "hide": l.default_hidden,
        "order": l.layer_priority,
        "z_index": l.z_index,
        "markers": markers,
        "timestamp": timestamp,
    })
}

/// `/tiles/<world>/markers.json`: the spawn icon and the world border.
/// `border` is `[min_x, min_z, max_x, max_z]`.
pub fn markers(w: &WorldConfig, spawn: Option<[i32; 2]>, border: [f64; 4], timestamp: u64) -> Value {
    let m = &w.map.markers;
    let mut layers = Vec::new();
    if m.spawn_icon.enabled {
        let icons: Vec<Value> = spawn
            .map(|[x, z]| {
                json!({
                    "type": "icon",
                    "point": {"x": x, "z": z},
                    "icon": "squaremap-spawn_icon",
                    "size": {"x": 16, "z": 16},
                    "anchor": {"x": 8, "z": 8},
                    "tooltip_anchor": {"x": 0, "z": -8},
                    "tooltip": "Spawn",
                })
            })
            .into_iter()
            .collect();
        layers.push(layer("squaremap-spawn_icon", "Spawn", &m.spawn_icon, json!(icons), timestamp));
    }
    if m.world_border.enabled {
        let [x0, z0, x1, z1] = border.map(|v| v.round() as i64);
        let points = json!([
            {"x": x0, "z": z0}, {"x": x1, "z": z0}, {"x": x1, "z": z1}, {"x": x0, "z": z1}, {"x": x0, "z": z0}
        ]);
        let line = json!([{"type": "polyline", "points": points, "color": "#ff0000", "tooltip": "World Border"}]);
        layers.push(layer("squaremap-worldborder", "World Border", &m.world_border, line, timestamp));
    }
    Value::Array(layers)
}

/// A player as the tracker shows them.
#[derive(Clone, Debug, PartialEq)]
pub struct Player {
    pub uuid: u128,
    pub name: String,
    pub world: String,
    pub position: [f64; 3],
    pub yaw: f32,
    pub health: f32,
    pub armor: Option<i32>,
    /// A nickname another mod gave them (shown with `use-display-names`).
    pub display_name: Option<String>,
}

/// Escapes text for the frontend, which puts `display_name` into the page as HTML.
pub fn html_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

/// `/tiles/players.json`. Players already filtered by the hide rules; `world` gives a
/// level's (`player-tracker.enabled`, `use-display-names`).
pub fn players(list: &[Player], max: usize, world: impl Fn(&str) -> (bool, bool)) -> Value {
    let players: Vec<Value> = list
        .iter()
        .map(|p| {
            let mut v = json!({
                "name": p.name,
                "uuid": format!("{:032x}", p.uuid),
                "world": web_name(&p.world),
            });
            let (tracker_enabled, display_names) = world(&p.world);
            if display_names {
                v["display_name"] = json!(html_escape(p.display_name.as_deref().unwrap_or(&p.name)));
            }
            if tracker_enabled {
                v["x"] = json!(p.position[0].floor() as i64);
                v["y"] = json!(p.position[1].floor() as i64);
                v["z"] = json!(p.position[2].floor() as i64);
                v["yaw"] = json!(p.yaw.round() as i64);
                v["health"] = json!(p.health as i32);
                if let Some(a) = p.armor {
                    v["armor"] = json!(a);
                }
            }
            v
        })
        .collect();
    json!({"players": players, "max": max})
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::DEFAULT_TOML;

    #[test]
    fn documents_have_squaremaps_shape() {
        let c = Config::parse(DEFAULT_TOML).unwrap();
        let s = settings(&c);
        assert_eq!(s["worlds"][0]["name"], "minecraft_overworld");
        assert_eq!(s["worlds"][0]["display_name"], "minecraft:overworld");
        assert_eq!(s["worlds"][2]["type"], "nether");
        let w = c.world("minecraft:overworld");
        let ws = world_settings(&w, [0, 0]);
        assert_eq!(ws["zoom"], json!({"def": 3, "max": 3, "extra": 2}));
        assert_eq!(ws["tiles_update_interval"], 15);
        let m = markers(&w, Some([5, -7]), [-15000.0, -15000.0, 15000.0, 15000.0], 1);
        assert_eq!(m[0]["markers"][0]["point"], json!({"x": 5, "z": -7}));
        assert_eq!(m[1]["markers"][0]["points"][2], json!({"x": 15000, "z": 15000}));
        let p = Player {
            uuid: 0x1234_5678_9abc_def0_1234_5678_9abc_def0,
            name: "Alex".into(),
            world: "minecraft:the_nether".into(),
            position: [1.5, 64.0, -0.5],
            yaw: 90.4,
            health: 19.5,
            armor: None,
            display_name: Some("<Al & ex>".into()),
        };
        let j = players(&[p.clone()], 20, |_| (true, false));
        assert!(j["players"][0].get("display_name").is_none());
        let d = players(&[p], 20, |_| (false, true));
        assert_eq!(d["players"][0]["display_name"], "&lt;Al &amp; ex&gt;");
        assert!(d["players"][0].get("x").is_none());
        assert_eq!(j["players"][0]["uuid"], "123456789abcdef0123456789abcdef0");
        assert_eq!(j["players"][0]["z"], -1);
        assert_eq!(j["players"][0]["health"], 19);
        assert!(j["players"][0].get("armor").is_none());
    }
}
