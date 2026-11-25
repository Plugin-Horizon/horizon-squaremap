//! ChunkyBorder's squaremap integration (`SquaremapIntegration`, GPL-3.0, read for behaviour; the code is written
//! for this mod): the border of a world is drawn as a layer `chunky` with one outline marker, and squaremap's own
//! `squaremap-worldborder` layer is left out of that world's `markers.json` while it exists.
//!
//! The data comes from the chunky mod's service `chunky:api`: `subscribe squaremap` once, then the mod's own service
//! `squaremap:chunky_event` is called with `border_change\t<world>\t<shape>\t<x>\t<z>\t<rx>\t<rz>\t<wrap>` or
//! `border_change\t<world>\tnone` (the layout is in chunky's `api.rs`). A border that existed before the subscription
//! is read with `border_get <world>`. When the chunky mod is not there every call fails quietly and nothing is drawn.

use crate::horizon_squaremap::api::{self, LayerOptions, Marker, MarkerOptions, Point, Shape};

/// The layer key the original registers (`CHUNKY_KEY`) and the one it replaces.
pub const LAYER: &str = "chunky";
/// The marker key inside the layer.
const MARKER: &str = "border";
/// Points around an ellipse (`numPoints`).
const ELLIPSE_POINTS: usize = 360;

/// One border as the chunky service reports it.
#[derive(Clone, Debug, PartialEq)]
pub struct Border {
    pub world: String,
    pub shape: String,
    pub cx: f64,
    pub cz: f64,
    pub rx: f64,
    pub rz: f64,
}

/// `map-options` of ChunkyBorder's config.
#[derive(Clone, Debug, PartialEq)]
pub struct Options {
    pub enabled: bool,
    pub label: String,
    pub hide_by_default: bool,
    pub color: u32,
    pub weight: i32,
    pub priority: i32,
}

impl Default for Options {
    fn default() -> Self {
        Self { enabled: true, label: "World Border".into(), hide_by_default: false, color: 0xFF0000, weight: 3, priority: 0 }
    }
}

impl Options {
    /// Reads `config.yml` (`key: value`) or `config.toml` (`key = value`): the keys are the same, only their names
    /// matter, not the section. An unreadable value keeps the default.
    pub fn parse(text: &str) -> Options {
        let mut o = Options::default();
        for line in text.lines() {
            let line = line.trim();
            if line.starts_with('#') {
                continue;
            }
            let Some((key, value)) = line.split_once('=').or_else(|| line.split_once(':')) else { continue };
            let value = value.split(" #").next().unwrap_or("").trim().trim_matches(|c| c == '"' || c == '\'');
            match key.trim() {
                "squaremap" => o.enabled = value != "false",
                "label" => o.label = value.to_owned(),
                "hide-by-default" => o.hide_by_default = value == "true",
                "color" => o.color = u32::from_str_radix(value.trim_start_matches('#'), 16).unwrap_or(o.color) & 0xFF_FFFF,
                "weight" => o.weight = value.parse().unwrap_or(o.weight),
                "priority" => o.priority = value.parse().unwrap_or(o.priority),
                _ => {}
            }
        }
        o
    }
}

impl Options {
    /// The chunky mod's `map_options` reply (`bluemap dynmap pl3xmap squaremap label hide-by-default color weight
    /// priority`, tab-separated): chunky owns and validates the keys, so this is what the border is drawn with.
    pub fn from_service(reply: &str) -> Option<Options> {
        let f: Vec<&str> = reply.trim_end_matches(['\r', '\n']).split('\t').collect();
        if f.len() != 9 {
            return None;
        }
        Some(Options {
            enabled: f[3] == "true",
            label: f[4].to_owned(),
            hide_by_default: f[5] == "true",
            color: u32::from_str_radix(f[6], 16).ok()? & 0xFF_FFFF,
            weight: f[7].parse().ok()?,
            priority: f[8].parse().ok()?,
        })
    }
}

/// `<world> <shape> <x> <z> <rx> <rz> [<wrap>]` as tab-separated fields (the reply of `border_get`, and the tail of
/// a `border_change` event).
pub fn parse_border(fields: &[&str]) -> Option<Border> {
    let num = |i: usize| fields.get(i)?.parse::<f64>().ok();
    Some(Border {
        world: (*fields.first()?).to_owned(),
        shape: fields.get(1)?.to_ascii_lowercase(),
        cx: num(2)?,
        cz: num(3)?,
        rx: num(4)?,
        rz: num(5)?,
    })
}

/// A `border_change` event: the world, and its border or `None` when the border was removed. Other events: `None`
/// altogether.
pub fn parse_event(line: &str) -> Option<(String, Option<Border>)> {
    let mut parts = line.split('\t');
    if parts.next()? != "border_change" {
        return None;
    }
    let rest: Vec<&str> = parts.collect();
    let world = (*rest.first()?).to_owned();
    if rest.get(1) == Some(&"none") {
        return Some((world, None));
    }
    Some((world, Some(parse_border(&rest)?)))
}

/// The dimension id squaremap's layers are keyed by, for a chunky world name.
pub fn dimension_of(world: &str) -> Option<String> {
    Some(match world {
        "world" => "minecraft:overworld".to_owned(),
        "world_nether" => "minecraft:the_nether".to_owned(),
        "world_the_end" => "minecraft:the_end".to_owned(),
        w if w.contains(':') => w.to_owned(),
        _ => return None,
    })
}

/// The chunky world name of a dimension id, for `border_get`.
pub fn world_of(dimension: &str) -> String {
    match dimension {
        "minecraft:overworld" => "world".to_owned(),
        "minecraft:the_nether" => "world_nether".to_owned(),
        "minecraft:the_end" => "world_the_end".to_owned(),
        other => other.to_owned(),
    }
}

fn corners(ax: f64, az: f64, r: f64, degrees: &[f64]) -> Vec<(f64, f64)> {
    degrees.iter().map(|d| (ax + r * d.to_radians().cos(), az + r * d.to_radians().sin())).collect()
}

/// Where the line through `a1`, `a2` meets the line through `b1`, `b2`.
fn intersection(a1: (f64, f64), a2: (f64, f64), b1: (f64, f64), b2: (f64, f64)) -> Option<(f64, f64)> {
    let (ca, cb) = (a2.1 - a1.1, b2.1 - b1.1);
    let (da, db) = (a1.0 - a2.0, b1.0 - b2.0);
    let det = ca * db - cb * da;
    if det == 0.0 {
        return None;
    }
    let (c1, c2) = (ca * a1.0 + da * a1.1, cb * b1.0 + db * b1.1);
    Some(((db * c1 - da * c2) / det, (ca * c2 - cb * c1) / det))
}

/// The corners of a polygon shape in chunky's order; `None` for the circle, the ellipse and unknown names.
fn polygon(b: &Border) -> Option<Vec<(f64, f64)>> {
    let (ax, az, rx, rz) = (b.cx, b.cz, b.rx, b.rz);
    Some(match b.shape.as_str() {
        "square" => vec![(ax + rx, az - rx), (ax - rx, az - rx), (ax - rx, az + rx), (ax + rx, az + rx)],
        "rectangle" => vec![(ax + rx, az - rz), (ax - rx, az - rz), (ax - rx, az + rz), (ax + rx, az + rz)],
        "diamond" => vec![(ax, az + rx), (ax - rx, az), (ax, az - rx), (ax + rx, az)],
        "triangle" => vec![(ax + rx, az + rx), (ax - rx, az + rx), (ax, az - rx)],
        "pentagon" => corners(ax, az, rx, &[54.0, 126.0, 198.0, 270.0, 342.0]),
        "hexagon" => corners(ax, az, rx, &[60.0, 120.0, 180.0, 240.0, 300.0, 360.0]),
        "star" => {
            let p = corners(ax, az, rx, &[54.0, 126.0, 198.0, 270.0, 342.0]);
            let at = |a: usize, b: usize, c: usize, d: usize, fallback: usize| intersection(p[a], p[b], p[c], p[d]).unwrap_or(p[fallback]);
            let i = [at(0, 2, 1, 4, 0), at(0, 2, 1, 3, 1), at(1, 3, 2, 4, 2), at(2, 4, 0, 3, 3), at(0, 3, 1, 4, 4)];
            (0..5).flat_map(|k| [p[k], i[k]]).collect()
        }
        _ => return None,
    })
}

/// The outline of an ellipse (a circle has equal radii) as one closed line.
fn ellipse(cx: f64, cz: f64, rx: f64, rz: f64) -> Vec<Point> {
    let step = 2.0 * std::f64::consts::PI / ELLIPSE_POINTS as f64;
    let mut points: Vec<Point> =
        (0..ELLIPSE_POINTS).map(|i| Point::new(cx + (step * i as f64).sin() * rx, cz + (step * i as f64).cos() * rz)).collect();
    points.push(Point::new(cx, cz + rz));
    points
}

/// The outline marker of a border (`addShapeMarker`): a polyline back to its first point, stroked, not filled, with
/// the label as its click tooltip. `None` for a shape this does not know.
pub fn marker(b: &Border, o: &Options) -> Option<Marker> {
    let line: Vec<Point> = match b.shape.as_str() {
        "circle" => ellipse(b.cx, b.cz, b.rx, b.rx),
        "ellipse" => ellipse(b.cx, b.cz, b.rx, b.rz),
        _ => {
            let mut p: Vec<Point> = polygon(b)?.into_iter().map(|(x, z)| Point::new(x, z)).collect();
            p.push(*p.first()?);
            p
        }
    };
    let mut m = Marker::new(Shape::Polyline(vec![line]));
    m.options = MarkerOptions {
        stroke: true,
        stroke_color: o.color,
        stroke_weight: o.weight,
        fill: false,
        click_tooltip: Some(o.label.clone()),
        ..MarkerOptions::default()
    };
    Some(m)
}

/// The border of `dimension` changed (`Some`) or went away (`None`): the layer is made and filled, or removed.
pub fn apply(dimension: &str, border: Option<&Border>, o: &Options) {
    let drawn = border.filter(|_| o.enabled).and_then(|b| marker(b, o));
    let Some(m) = drawn else {
        api::unregister_layer(dimension, LAYER);
        return;
    };
    let mut options = LayerOptions::new(&o.label);
    options.default_hidden = o.hide_by_default;
    options.layer_priority = 1;
    options.z_index = o.priority;
    // Registered again with the current label and options; the marker is replaced.
    api::unregister_layer(dimension, LAYER);
    if api::register_layer(dimension, LAYER, options).is_ok() {
        let _ = api::set_marker(dimension, LAYER, MARKER, m);
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_chunky_reply_and_the_file_agree() {
        // chunky's `MapOptions::fields()` for this config text.
        let text = "map-options:\n  enable:\n    squaremap: false\n  label: \"Edge\"\n  hide-by-default: true\n  color: 00FF00\n  weight: 5\n  priority: 7\n";
        let reply = "true\ttrue\ttrue\tfalse\tEdge\ttrue\t00FF00\t5\t7";
        assert_eq!(Options::from_service(reply), Some(Options::parse(text)));
        assert_eq!(Options::from_service("true\ttrue"), None);
        assert_eq!(Options::from_service("true\ttrue\ttrue\ttrue\tL\tfalse\tzzzzzz\t3\t0"), None);
    }

    use super::*;
    use serde_json::Value;

    fn square() -> Border {
        parse_border(&["world", "square", "0", "0", "100", "100", "none"]).unwrap()
    }

    #[test]
    fn options_read_both_formats_and_default() {
        assert_eq!(Options::parse(""), Options::default());
        let o = Options::parse("map-options:\n  enable:\n    squaremap: false\n  label: \"Edge\"\n  hide-by-default: true\n  color: 00FF00 # green\n  weight: 5\n  priority: 7\n  visualizer-color: 20A0FF\n");
        assert_eq!(o, Options { enabled: false, label: "Edge".into(), hide_by_default: true, color: 0x00FF00, weight: 5, priority: 7 });
        let t = Options::parse("label = 'Wall'\nweight = x\n");
        assert_eq!((t.label.as_str(), t.weight), ("Wall", 3));
    }

    #[test]
    fn events_parse_changes_and_removals() {
        let (w, b) = parse_event("border_change\tworld\tcircle\t10\t-20\t500\t500\tnone").unwrap();
        assert_eq!(w, "world");
        let b = b.unwrap();
        assert_eq!((b.shape.as_str(), b.cx, b.cz, b.rx), ("circle", 10.0, -20.0, 500.0));
        assert_eq!(parse_event("border_change\tworld_nether\tnone"), Some(("world_nether".to_owned(), None)));
        assert_eq!(parse_event("generation_start\tworld"), None);
        assert_eq!(parse_event("border_change\tworld\tcircle\tx"), None);
    }

    #[test]
    fn dimensions_map_both_ways() {
        assert_eq!(dimension_of("world_the_end").as_deref(), Some("minecraft:the_end"));
        assert_eq!(dimension_of("mv:arena").as_deref(), Some("mv:arena"));
        assert_eq!(dimension_of("arena"), None);
        assert_eq!(world_of("minecraft:the_nether"), "world_nether");
    }

    #[test]
    fn a_square_is_a_closed_line_of_five_points() {
        let m = marker(&square(), &Options::default()).unwrap();
        let Shape::Polyline(lines) = &m.shape else { panic!() };
        assert_eq!(lines[0].len(), 5);
        assert_eq!(lines[0][0], Point::new(100.0, -100.0));
        assert_eq!(lines[0][4], lines[0][0]);
        assert!(!m.options.fill && m.options.stroke);
        assert_eq!((m.options.stroke_color, m.options.stroke_weight), (0xFF0000, 3));
        assert_eq!(m.options.click_tooltip.as_deref(), Some("World Border"));
    }

    #[test]
    fn a_circle_has_361_points_starting_south_and_ending_there() {
        let b = parse_border(&["world", "circle", "0", "0", "50", "50"]).unwrap();
        let Shape::Polyline(lines) = marker(&b, &Options::default()).unwrap().shape else { panic!() };
        assert_eq!(lines[0].len(), ELLIPSE_POINTS + 1);
        assert_eq!(lines[0][0], Point::new(0.0, 50.0));
        assert_eq!(lines[0][ELLIPSE_POINTS], Point::new(0.0, 50.0));
        // The ellipse uses both radii.
        let e = parse_border(&["world", "ellipse", "0", "0", "50", "20"]).unwrap();
        let Shape::Polyline(l) = marker(&e, &Options::default()).unwrap().shape else { panic!() };
        assert_eq!(l[0][0], Point::new(0.0, 20.0));
        assert!((l[0][90].x - 50.0).abs() < 1e-9);
    }

    #[test]
    fn star_and_unknown_shapes() {
        let s = parse_border(&["world", "star", "0", "0", "100", "100"]).unwrap();
        let Shape::Polyline(l) = marker(&s, &Options::default()).unwrap().shape else { panic!() };
        assert_eq!(l[0].len(), 11);
        assert!(marker(&parse_border(&["world", "blob", "0", "0", "1", "1"]).unwrap(), &Options::default()).is_none());
    }

    #[test]
    fn apply_draws_replaces_and_removes_the_layer() {
        let dim = "test:chunky_border_apply";
        let o = Options { color: 0x00FF00, label: "Edge".into(), hide_by_default: true, priority: 4, ..Options::default() };
        apply(dim, Some(&square()), &o);
        let layers = api::layers_json(dim);
        assert_eq!(layers.len(), 1);
        let l = &layers[0];
        assert_eq!((l["id"].as_str(), l["name"].as_str(), l["hide"].as_bool(), l["order"].as_i64(), l["z_index"].as_i64()),
            (Some("chunky"), Some("Edge"), Some(true), Some(1), Some(4)));
        let m: &Value = &l["markers"][0];
        assert_eq!(m["type"], "polyline");
        assert_eq!(m["color"], "#00ff00");
        assert_eq!(m["fill"], false);
        // A new border replaces the marker, not adds one.
        let big = parse_border(&["world", "square", "0", "0", "200", "200"]).unwrap();
        apply(dim, Some(&big), &o);
        assert_eq!(api::layers_json(dim)[0]["markers"].as_array().unwrap().len(), 1);
        assert!(api::has_layer(dim, LAYER));
        // Removed, or the integration switched off: gone.
        apply(dim, None, &o);
        assert!(api::layers_json(dim).is_empty() && !api::has_layer(dim, LAYER));
        apply(dim, Some(&square()), &Options { enabled: false, ..o });
        assert!(api::layers_json(dim).is_empty());
    }
}
