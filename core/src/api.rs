//! The map's API for other mods, after squaremap's (`Squaremap`, `MapWorld`,
//! `LayerProvider`, `SimpleLayerProvider`, `Marker`, `MarkerOptions`, `PlayerManager`;
//! MIT, Copyright (c) 2021-2024 Jason Penilla). Another native mod built into the same
//! server calls these functions (`horizon_squaremap::api::...`); everything is kept here,
//! so a mod may register before the map starts and its layers survive `/squaremap reload`.
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

/// A world coordinate on the map (squaremap's `Point`); serialized truncated to ints.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Point {
    pub x: f64,
    pub z: f64,
}

impl Point {
    pub fn new(x: f64, z: f64) -> Self {
        Self { x, z }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum FillRule {
    NonZero,
    #[default]
    EvenOdd,
}

/// `MarkerOptions`; colours are 0xRRGGBB.
#[derive(Clone, Debug, PartialEq)]
pub struct MarkerOptions {
    pub stroke: bool,
    pub stroke_color: u32,
    pub stroke_weight: i32,
    pub stroke_opacity: f64,
    pub fill: bool,
    pub fill_color: Option<u32>,
    pub fill_opacity: f64,
    pub fill_rule: FillRule,
    /// HTML shown on click.
    pub click_tooltip: Option<String>,
    /// HTML shown on hover.
    pub hover_tooltip: Option<String>,
}

impl Default for MarkerOptions {
    fn default() -> Self {
        Self {
            stroke: true,
            stroke_color: 0x0000FF,
            stroke_weight: 3,
            stroke_opacity: 1.0,
            fill: true,
            fill_color: None,
            fill_opacity: 0.2,
            fill_rule: FillRule::EvenOdd,
            click_tooltip: None,
            hover_tooltip: None,
        }
    }
}

/// The marker kinds of squaremap's API.
#[derive(Clone, Debug, PartialEq)]
pub enum Shape {
    /// An icon registered with [`register_icon`].
    Icon { point: Point, icon: String, size: (i32, i32), anchor: Point, tooltip_anchor: Point },
    /// One line, or several (a multi-polyline).
    Polyline(Vec<Vec<Point>>),
    Rectangle(Point, Point),
    Circle { center: Point, radius: f64 },
    Ellipse { center: Point, radius_x: f64, radius_z: f64 },
    /// The outline and its holes.
    Polygon { main: Vec<Point>, holes: Vec<Vec<Point>> },
    MultiPolygon(Vec<(Vec<Point>, Vec<Vec<Point>>)>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Marker {
    pub shape: Shape,
    pub options: MarkerOptions,
}

impl Marker {
    pub fn new(shape: Shape) -> Self {
        Self { shape, options: MarkerOptions::default() }
    }
    /// `Marker.icon` with squaremap's defaults: anchored at its centre, tooltip above it.
    pub fn icon(point: Point, icon: &str, size: i32) -> Self {
        let half = f64::from(size / 2);
        Self::new(Shape::Icon {
            point,
            icon: icon.to_owned(),
            size: (size, size),
            anchor: Point::new(half, half),
            tooltip_anchor: Point::new(0.0, -half),
        })
    }
}

/// `SimpleLayerProvider`'s settings.
#[derive(Clone, Debug, PartialEq)]
pub struct LayerOptions {
    pub label: String,
    pub show_controls: bool,
    pub default_hidden: bool,
    pub layer_priority: i32,
    pub z_index: i32,
}

impl LayerOptions {
    pub fn new(label: &str) -> Self {
        Self { label: label.to_owned(), show_controls: true, default_hidden: false, layer_priority: 99, z_index: 99 }
    }
}

struct Layer {
    options: LayerOptions,
    markers: BTreeMap<String, Marker>,
    changed: u64,
}

#[derive(Default)]
struct Api {
    /// (world, layer key) in registration order.
    layers: Vec<((String, String), Layer)>,
    icons: BTreeMap<String, Vec<u8>>,
    generation: u64,
    hidden: HashSet<u128>,
    persistent: HashSet<u128>,
    hidden_file: Option<PathBuf>,
}

static API: Mutex<Option<Api>> = Mutex::new(None);

fn with<T>(f: impl FnOnce(&mut Api) -> T) -> T {
    f(API.lock().unwrap().get_or_insert_with(Api::default))
}

fn now_millis() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

/// squaremap's `Key`: `[a-zA-Z0-9._-]+`.
fn check_key(key: &str) -> Result<(), String> {
    if !key.is_empty() && key.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-')) {
        Ok(())
    } else {
        Err(format!("invalid key '{key}': use a-z, A-Z, 0-9, '.', '_', '-'"))
    }
}

const BUILT_IN: [&str; 3] = ["squaremap-spawn_icon", "squaremap-worldborder", "squaremap-player-tracker"];

/// Adds a layer to a world's map (`minecraft:overworld`); a key may be used once per world.
pub fn register_layer(world: &str, key: &str, options: LayerOptions) -> Result<(), String> {
    check_key(key)?;
    if BUILT_IN.contains(&key) {
        return Err(format!("layer key '{key}' is squaremap's own"));
    }
    with(|a| {
        let id = (world.to_owned(), key.to_owned());
        if a.layers.iter().any(|(k, _)| *k == id) {
            return Err(format!("layer '{key}' is already registered for {world}"));
        }
        a.layers.push((id, Layer { options, markers: BTreeMap::new(), changed: now_millis() }));
        a.generation += 1;
        Ok(())
    })
}

pub fn unregister_layer(world: &str, key: &str) -> bool {
    with(|a| {
        let before = a.layers.len();
        a.layers.retain(|((w, k), _)| !(w == world && k == key));
        let removed = a.layers.len() != before;
        a.generation += u64::from(removed);
        removed
    })
}

fn layer_mut<T>(world: &str, key: &str, f: impl FnOnce(&mut Layer) -> T) -> Result<T, String> {
    with(|a| {
        let l = a
            .layers
            .iter_mut()
            .find(|((w, k), _)| w == world && k == key)
            .map(|(_, l)| l)
            .ok_or_else(|| format!("no layer '{key}' in {world}"))?;
        let out = f(l);
        l.changed = now_millis();
        a.generation += 1;
        Ok(out)
    })
}

/// Adds or replaces a marker of a layer.
pub fn set_marker(world: &str, layer: &str, key: &str, marker: Marker) -> Result<(), String> {
    check_key(key)?;
    layer_mut(world, layer, |l| {
        l.markers.insert(key.to_owned(), marker);
    })
}

pub fn remove_marker(world: &str, layer: &str, key: &str) -> bool {
    layer_mut(world, layer, |l| l.markers.remove(key).is_some()).unwrap_or(false)
}

pub fn clear_markers(world: &str, layer: &str) {
    let _ = layer_mut(world, layer, |l| l.markers.clear());
}

/// Registers a PNG as icon `key` (served as `images/icon/registered/<key>.png`).
pub fn register_icon(key: &str, png: Vec<u8>) -> Result<(), String> {
    check_key(key)?;
    crate::image::decode_rgb(&png).map_err(|e| format!("icon '{key}' is not a PNG: {e}"))?;
    let new = with(|a| a.icons.insert(key.to_owned(), png.clone()).is_none());
    if !new {
        return Err(format!("icon '{key}' is already registered"));
    }
    crate::runtime::write_icon(key, &png);
    Ok(())
}

pub fn unregister_icon(key: &str) -> bool {
    let removed = with(|a| a.icons.remove(key).is_some());
    if removed {
        crate::runtime::remove_icon(key);
    }
    removed
}

pub(crate) fn icons() -> Vec<(String, Vec<u8>)> {
    with(|a| a.icons.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
}

/// Changes whenever a layer or marker changes.
pub(crate) fn generation() -> u64 {
    with(|a| a.generation)
}

/// The world's API layers as `markers.json` entries (`MarkerDataPublisher`).
pub(crate) fn layers_json(world: &str) -> Vec<Value> {
    with(|a| {
        a.layers
            .iter()
            .filter(|((w, _), _)| w == world)
            .map(|((_, key), l)| {
                let o = &l.options;
                json!({
                    "id": key,
                    "name": o.label,
                    "control": o.show_controls,
                    "hide": o.default_hidden,
                    "order": o.layer_priority,
                    "z_index": o.z_index,
                    "markers": l.markers.values().map(marker_json).collect::<Vec<_>>(),
                    "timestamp": l.changed,
                })
            })
            .collect()
    })
}

fn point(p: Point) -> Value {
    json!({"x": p.x as i32, "z": p.z as i32})
}

fn points(list: &[Point]) -> Value {
    Value::Array(list.iter().map(|&p| point(p)).collect())
}

fn hex(c: u32) -> String {
    format!("#{:06x}", c & 0xFF_FFFF)
}

/// `MarkerJsonSerializer`: options differing from the defaults, then the shape.
pub fn marker_json(m: &Marker) -> Value {
    let mut out = Map::new();
    let (o, d) = (&m.options, MarkerOptions::default());
    if o.stroke != d.stroke {
        out.insert("stroke".into(), json!(o.stroke));
    }
    if o.stroke_color != d.stroke_color {
        out.insert("color".into(), json!(hex(o.stroke_color)));
    }
    if o.stroke_weight != d.stroke_weight {
        out.insert("weight".into(), json!(o.stroke_weight));
    }
    if o.stroke_opacity != d.stroke_opacity {
        out.insert("opacity".into(), json!(o.stroke_opacity));
    }
    if o.fill != d.fill {
        out.insert("fill".into(), json!(o.fill));
    }
    if let Some(c) = o.fill_color {
        out.insert("fillColor".into(), json!(hex(c)));
    }
    if o.fill_opacity != d.fill_opacity {
        out.insert("fillOpacity".into(), json!(o.fill_opacity));
    }
    if o.fill_rule != d.fill_rule {
        out.insert("fillRule".into(), json!("nonzero"));
    }
    if let Some(t) = &o.click_tooltip {
        out.insert("popup".into(), json!(t));
    }
    if let Some(t) = &o.hover_tooltip {
        out.insert("tooltip".into(), json!(t));
    }
    let rings = |main: &[Point], holes: &[Vec<Point>]| {
        let mut all = vec![points(main)];
        all.extend(holes.iter().map(|h| points(h)));
        Value::Array(all)
    };
    match &m.shape {
        Shape::Polyline(lines) => {
            out.insert("type".into(), json!("polyline"));
            let p = if lines.len() == 1 { points(&lines[0]) } else { Value::Array(lines.iter().map(|l| points(l)).collect()) };
            out.insert("points".into(), p);
        }
        Shape::Rectangle(a, b) => {
            out.insert("type".into(), json!("rectangle"));
            out.insert("points".into(), json!([point(*a), point(*b)]));
        }
        Shape::Circle { center, radius } => {
            out.insert("type".into(), json!("circle"));
            out.insert("center".into(), point(*center));
            out.insert("radius".into(), json!(radius));
        }
        Shape::Ellipse { center, radius_x, radius_z } => {
            out.insert("type".into(), json!("ellipse"));
            out.insert("center".into(), point(*center));
            out.insert("radiusX".into(), json!(radius_x));
            out.insert("radiusZ".into(), json!(radius_z));
        }
        Shape::Polygon { main, holes } => {
            out.insert("type".into(), json!("polygon"));
            out.insert("points".into(), rings(main, holes));
        }
        Shape::MultiPolygon(polys) => {
            out.insert("type".into(), json!("polygon"));
            out.insert("points".into(), Value::Array(polys.iter().map(|(m, h)| rings(m, h)).collect()));
        }
        Shape::Icon { point: p, icon, size, anchor, tooltip_anchor } => {
            out.insert("type".into(), json!("icon"));
            out.insert("point".into(), point(*p));
            out.insert("size".into(), json!({"x": size.0, "z": size.1}));
            out.insert("anchor".into(), point(*anchor));
            out.insert("tooltip_anchor".into(), point(*tooltip_anchor));
            out.insert("icon".into(), json!(icon));
        }
    }
    Value::Object(out)
}

// Hidden players (`PlayerManager`, `/squaremap hide|show`). squaremap keeps the persistent
// flag in the player's data; here it is a JSON list of UUIDs in the map's data folder.

fn uuid_string(u: u128) -> String {
    let h = format!("{u:032x}");
    format!("{}-{}-{}-{}-{}", &h[..8], &h[8..12], &h[12..16], &h[16..20], &h[20..])
}

fn parse_uuid(s: &str) -> Option<u128> {
    u128::from_str_radix(&s.replace('-', ""), 16).ok()
}

/// Reads the persistent list (on start).
pub(crate) fn load_hidden(file: PathBuf) {
    let list: Vec<String> = std::fs::read_to_string(&file).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    with(|a| {
        a.persistent = list.iter().filter_map(|s| parse_uuid(s)).collect();
        a.hidden_file = Some(file);
    });
}

fn save_hidden(a: &Api) {
    let Some(file) = &a.hidden_file else { return };
    let mut list: Vec<String> = a.persistent.iter().map(|&u| uuid_string(u)).collect();
    list.sort();
    let write = || -> std::io::Result<()> {
        if let Some(p) = file.parent() {
            std::fs::create_dir_all(p)?;
        }
        std::fs::write(file, serde_json::to_string_pretty(&list).unwrap_or_default())
    };
    if let Err(e) = write() {
        eprintln!("[squaremap] saving hidden players to {}: {e}", file.display());
    }
}

/// Hides a player from the map; `persistent` keeps it across restarts.
pub fn hide_player(uuid: u128, persistent: bool) {
    with(|a| {
        a.hidden.insert(uuid);
        if persistent && a.persistent.insert(uuid) {
            save_hidden(a);
        }
    });
}

pub fn show_player(uuid: u128, persistent: bool) {
    with(|a| {
        a.hidden.remove(&uuid);
        if persistent && a.persistent.remove(&uuid) {
            save_hidden(a);
        }
    });
}

pub fn player_hidden(uuid: u128) -> bool {
    with(|a| a.hidden.contains(&uuid) || a.persistent.contains(&uuid))
}

/// `/squaremap reload` drops the hides that weren't persistent, as squaremap's API says.
pub(crate) fn clear_temporary_hides() {
    with(|a| a.hidden.clear());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markers_serialize_like_squaremap() {
        let mut m = Marker::new(Shape::Polygon {
            main: vec![Point::new(0.9, 0.0), Point::new(10.0, -0.5), Point::new(10.0, 10.0)],
            holes: vec![vec![Point::new(2.0, 2.0)]],
        });
        m.options.fill_color = Some(0x00FF00);
        m.options.stroke_weight = 1;
        m.options.hover_tooltip = Some("<b>Home</b>".into());
        let j = marker_json(&m);
        assert_eq!(j["type"], "polygon");
        assert_eq!(j["points"][0][1], json!({"x": 10, "z": 0}));
        assert_eq!(j["points"][1][0], json!({"x": 2, "z": 2}));
        assert_eq!(j["fillColor"], "#00ff00");
        assert_eq!(j["weight"], 1);
        assert_eq!(j["tooltip"], "<b>Home</b>");
        assert!(j.get("color").is_none() && j.get("fillOpacity").is_none());
        let line = marker_json(&Marker::new(Shape::Polyline(vec![vec![Point::new(1.0, 2.0)]])));
        assert_eq!(line["points"], json!([{"x": 1, "z": 2}]));
        let icon = marker_json(&Marker::icon(Point::new(5.0, 6.0), "shop", 16));
        assert_eq!(icon["anchor"], json!({"x": 8, "z": 8}));
        assert_eq!(icon["tooltip_anchor"], json!({"x": 0, "z": -8}));
    }

    #[test]
    fn layers_registry_and_hidden_players() {
        let w = "test:layers";
        assert!(register_layer(w, "bad key", LayerOptions::new("x")).is_err());
        assert!(register_layer(w, "squaremap-worldborder", LayerOptions::new("x")).is_err());
        register_layer(w, "shops", LayerOptions::new("Shops")).unwrap();
        assert!(register_layer(w, "shops", LayerOptions::new("Shops")).is_err());
        let g = generation();
        set_marker(w, "shops", "a", Marker::new(Shape::Circle { center: Point::new(1.0, 1.0), radius: 5.0 })).unwrap();
        assert!(generation() > g);
        assert!(set_marker(w, "nope", "a", Marker::new(Shape::Rectangle(Point::new(0.0, 0.0), Point::new(1.0, 1.0)))).is_err());
        let l = layers_json(w);
        assert_eq!(l[0]["id"], "shops");
        assert_eq!(l[0]["order"], 99);
        assert_eq!(l[0]["markers"][0]["radius"], 5.0);
        assert!(remove_marker(w, "shops", "a"));
        assert!(unregister_layer(w, "shops") && layers_json(w).is_empty());
        let (p, t) = (0x1111_u128, 0x2222_u128);
        hide_player(p, true);
        hide_player(t, false);
        assert!(player_hidden(p) && player_hidden(t));
        clear_temporary_hides();
        assert!(player_hidden(p) && !player_hidden(t));
        show_player(p, true);
        assert!(!player_hidden(p));
        assert_eq!(uuid_string(0x0123456789abcdef0123456789abcdef), "01234567-89ab-cdef-0123-456789abcdef");
        assert_eq!(parse_uuid("01234567-89ab-cdef-0123-456789abcdef"), Some(0x0123456789abcdef0123456789abcdef));
    }
}
