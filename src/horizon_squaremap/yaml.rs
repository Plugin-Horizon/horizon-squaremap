//! The YAML subset squaremap's `config.yml`, `advanced.yml` and `locale/lang-*.yml` use, read into a TOML table so
//! the config structs read it as they read the TOML file: nested block maps, block lists of scalars or maps, flow
//! lists of scalars (`[a, b]`), quoted and plain scalars, comments. Anchors, multi-line scalars, tags and flow maps
//! are not read.
use std::collections::HashMap;

type Lines = Vec<(usize, String)>;

/// The text with comments and blank lines gone, each line with its indent.
fn lines(text: &str) -> Lines {
    text.lines()
        .filter_map(|raw| {
            let body = strip_comment(raw.trim_end());
            let trimmed = body.trim_start();
            (!trimmed.is_empty() && trimmed != "---").then(|| (body.len() - trimmed.len(), trimmed.to_owned()))
        })
        .collect()
}

/// Cuts a ` #` comment that is not inside quotes (a line that starts with `#` is all comment).
fn strip_comment(line: &str) -> &str {
    let (mut single, mut double) = (false, false);
    let mut prev = ' ';
    for (i, c) in line.char_indices() {
        match c {
            '\'' if !double => single = !single,
            '"' if !single && prev != '\\' => double = !double,
            '#' if !single && !double && (i == 0 || prev == ' ' || prev == '\t') => return &line[..i],
            _ => {}
        }
        prev = c;
    }
    line
}

/// Splits `key: value` at the first colon that is outside quotes and followed by a space or the end.
fn split_key(line: &str) -> Option<(String, String)> {
    let (mut single, mut double) = (false, false);
    let chars: Vec<(usize, char)> = line.char_indices().collect();
    for (n, &(i, c)) in chars.iter().enumerate() {
        match c {
            '\'' if !double => single = !single,
            '"' if !single => double = !double,
            ':' if !single && !double && chars.get(n + 1).is_none_or(|&(_, next)| next == ' ') => {
                return Some((unquote(line[..i].trim()), line[i + 1..].trim().to_owned()));
            }
            _ => {}
        }
    }
    None
}

fn unquote(s: &str) -> String {
    let b = s.as_bytes();
    if b.len() >= 2 && ((b[0] == b'"' && b[b.len() - 1] == b'"') || (b[0] == b'\'' && b[b.len() - 1] == b'\'')) {
        let inner = &s[1..s.len() - 1];
        if b[0] == b'\'' {
            return inner.replace("''", "'");
        }
        return inner.replace("\\\"", "\"").replace("\\n", "\n").replace("\\\\", "\\");
    }
    s.to_owned()
}

fn scalar(s: &str) -> toml::Value {
    let s = s.trim();
    if s.starts_with('"') || s.starts_with('\'') {
        return toml::Value::String(unquote(s));
    }
    if s.starts_with('[') && s.ends_with(']') {
        let inner = s[1..s.len() - 1].trim();
        let mut items = Vec::new();
        if !inner.is_empty() {
            let (mut start, mut single, mut double) = (0, false, false);
            for (i, c) in inner.char_indices() {
                match c {
                    '\'' if !double => single = !single,
                    '"' if !single => double = !double,
                    ',' if !single && !double => {
                        items.push(scalar(&inner[start..i]));
                        start = i + 1;
                    }
                    _ => {}
                }
            }
            items.push(scalar(&inner[start..]));
        }
        return toml::Value::Array(items);
    }
    match s {
        "true" | "True" => toml::Value::Boolean(true),
        "false" | "False" => toml::Value::Boolean(false),
        _ => {
            if let Ok(i) = s.parse::<i64>() {
                toml::Value::Integer(i)
            } else if let Ok(f) = s.parse::<f64>() {
                if f.is_finite() { toml::Value::Float(f) } else { toml::Value::String(s.to_owned()) }
            } else {
                toml::Value::String(s.to_owned())
            }
        }
    }
}

fn is_item(line: &str) -> bool {
    line == "-" || line.starts_with("- ")
}

fn block(lines: &mut Lines, pos: &mut usize, indent: usize) -> Result<toml::Value, String> {
    if is_item(&lines[*pos].1) {
        let mut out = Vec::new();
        while *pos < lines.len() && lines[*pos].0 == indent && is_item(&lines[*pos].1) {
            let rest = lines[*pos].1[1..].trim_start().to_owned();
            if rest.is_empty() {
                *pos += 1;
                if *pos < lines.len() && lines[*pos].0 > indent {
                    let inner = lines[*pos].0;
                    out.push(block(lines, pos, inner)?);
                }
            } else if split_key(&rest).is_some() && !rest.starts_with('"') && !rest.starts_with('\'') {
                // `- key: value`: a map whose first entry sits on the dash's line.
                let inner = indent + (lines[*pos].1.len() - rest.len());
                lines[*pos] = (inner, rest);
                out.push(block(lines, pos, inner)?);
            } else {
                out.push(scalar(&rest));
                *pos += 1;
            }
        }
        return Ok(toml::Value::Array(out));
    }
    let mut table = toml::Table::new();
    while *pos < lines.len() && lines[*pos].0 == indent && !is_item(&lines[*pos].1) {
        let (key, value) = split_key(&lines[*pos].1).ok_or_else(|| format!("not a `key: value` line: {}", lines[*pos].1))?;
        *pos += 1;
        let parsed = if !value.is_empty() {
            scalar(&value)
        } else if *pos < lines.len() && (lines[*pos].0 > indent || (lines[*pos].0 == indent && is_item(&lines[*pos].1))) {
            let inner = lines[*pos].0;
            block(lines, pos, inner)?
        } else {
            toml::Value::Table(toml::Table::new())
        };
        table.insert(key, parsed);
    }
    Ok(toml::Value::Table(table))
}

/// Reads a YAML document (a map at its top) into a table.
pub fn parse(text: &str) -> Result<toml::Table, String> {
    let mut all = lines(text);
    if all.is_empty() {
        return Ok(toml::Table::new());
    }
    let (mut pos, indent) = (0, all[0].0);
    match block(&mut all, &mut pos, indent)? {
        toml::Value::Table(t) if pos >= all.len() => Ok(t),
        toml::Value::Table(_) => Err(format!("unexpected indentation at: {}", all[pos].1)),
        _ => Err("the document is not a map".into()),
    }
}

/// `a.b.c` -> text for every string value of a nested table (the language file's keys).
pub fn flatten(table: &toml::Table) -> HashMap<String, String> {
    fn walk(prefix: &str, table: &toml::Table, out: &mut HashMap<String, String>) {
        for (key, value) in table {
            let path = if prefix.is_empty() { key.clone() } else { format!("{prefix}.{key}") };
            match value {
                toml::Value::Table(t) => walk(&path, t, out),
                toml::Value::String(s) => {
                    out.insert(path, s.clone());
                }
                toml::Value::Integer(i) => {
                    out.insert(path, i.to_string());
                }
                toml::Value::Boolean(b) => {
                    out.insert(path, b.to_string());
                }
                _ => {}
            }
        }
    }
    let mut out = HashMap::new();
    walk("", table, &mut out);
    out
}

/// `over` merged onto `base`: a table inside a table merges, anything else is replaced.
pub fn overlay(base: &mut toml::Table, over: &toml::Table) {
    for (key, value) in over {
        match (base.get_mut(key), value) {
            (Some(toml::Value::Table(a)), toml::Value::Table(b)) => overlay(a, b),
            _ => {
                base.insert(key.clone(), value.clone());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_nested_maps_lists_and_scalars() {
        let t = parse(
            "# squaremap\nsettings:\n  web-address: 'http://example.org:8080' # comment\n  internal-webserver:\n    enabled: true\n    port: 8081\nworld-settings:\n  default:\n    invisible-blocks:\n    - minecraft:fern\n    - \"minecraft:bush\"\n    map:\n      zoom:\n        maximum: 3\n      visibility-limits: []\n  \"minecraft:the_nether\":\n    map:\n      iterate-up: true\nsettings-list: [a, \"b c\"]\n",
        )
        .unwrap();
        let s = t["settings"].as_table().unwrap();
        assert_eq!(s["web-address"].as_str(), Some("http://example.org:8080"));
        assert_eq!(s["internal-webserver"]["port"].as_integer(), Some(8081));
        assert_eq!(s["internal-webserver"]["enabled"].as_bool(), Some(true));
        let d = &t["world-settings"]["default"];
        assert_eq!(d["invisible-blocks"].as_array().unwrap().len(), 2);
        assert_eq!(d["invisible-blocks"][1].as_str(), Some("minecraft:bush"));
        assert_eq!(d["map"]["zoom"]["maximum"].as_integer(), Some(3));
        assert_eq!(t["world-settings"]["minecraft:the_nether"]["map"]["iterate-up"].as_bool(), Some(true));
        assert_eq!(t["settings-list"][1].as_str(), Some("b c"));
    }

    #[test]
    fn reads_a_list_of_maps_and_flattens_language_keys() {
        let t = parse("limits:\n  - type: circle\n    radius: 5000\n  - type: rectangle\n    radius: 10\nlang:\n  command:\n    prefix: \"<white>[squaremap]\"\n").unwrap();
        let l = t["limits"].as_array().unwrap();
        assert_eq!((l[0]["type"].as_str(), l[0]["radius"].as_integer()), (Some("circle"), Some(5000)));
        assert_eq!(l[1]["type"].as_str(), Some("rectangle"));
        assert_eq!(flatten(&t)["lang.command.prefix"], "<white>[squaremap]");
    }

    #[test]
    fn overlay_merges_tables_and_replaces_the_rest() {
        let mut base = parse("a:\n  x: 1\n  y: 2\nb: [1]\n").unwrap();
        overlay(&mut base, &parse("a:\n  y: 5\nb: [2, 3]\n").unwrap());
        assert_eq!((base["a"]["x"].as_integer(), base["a"]["y"].as_integer()), (Some(1), Some(5)));
        assert_eq!(base["b"].as_array().unwrap().len(), 2);
    }
}
