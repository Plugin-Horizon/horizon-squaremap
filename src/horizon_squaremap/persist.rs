//! `/squaremap progresslogging toggle|rate` write their value back to the config (`Config.toggleProgressLogging`,
//! `Config.setLoggingInterval` save `config.yml`). The files are edited as text, so their comments and layout stay.

/// Sets `key` inside the TOML table `[section]` (`enabled = false`), when both are there.
pub fn set_toml(text: &str, section: &str, key: &str, value: &str) -> Option<String> {
    let header = format!("[{section}]");
    let mut inside = false;
    let mut done = false;
    let mut out = Vec::new();
    for line in text.lines() {
        let t = line.trim();
        if t.starts_with('[') {
            inside = t == header;
        } else if inside && !done {
            if let Some((k, _)) = t.split_once('=') {
                if k.trim() == key {
                    let indent = &line[..line.len() - line.trim_start().len()];
                    out.push(format!("{indent}{key} = {value}"));
                    done = true;
                    continue;
                }
            }
        }
        out.push(line.to_owned());
    }
    done.then(|| join(out, text))
}

/// Sets the scalar at the nested YAML `path` (`["settings", "render-progress-logging", "enabled"]`), when it is there.
pub fn set_yaml(text: &str, path: &[&str], value: &str) -> Option<String> {
    // The indents of the keys matched so far.
    let mut matched: Vec<usize> = Vec::new();
    let mut out = Vec::new();
    let mut done = false;
    for line in text.lines() {
        let trimmed = line.trim_start();
        let indent = line.len() - trimmed.len();
        if done || trimmed.is_empty() || trimmed.starts_with('#') {
            out.push(line.to_owned());
            continue;
        }
        while matched.last().is_some_and(|&i| indent <= i) {
            matched.pop();
        }
        let depth = matched.len();
        if depth < path.len() {
            if let Some(rest) = trimmed.strip_prefix(path[depth]).and_then(|r| r.strip_prefix(':')) {
                if depth + 1 == path.len() {
                    let comment = rest.find(" #").map(|i| &rest[i..]).unwrap_or("");
                    out.push(format!("{}{}: {value}{comment}", " ".repeat(indent), path[depth]));
                    done = true;
                    continue;
                }
                matched.push(indent);
            }
        }
        out.push(line.to_owned());
    }
    done.then(|| join(out, text))
}

fn join(lines: Vec<String>, original: &str) -> String {
    let mut s = lines.join("\n");
    if original.ends_with('\n') {
        s.push('\n');
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toml_value_is_replaced_in_its_section_only() {
        let text = "[other]\nenabled = true\n[settings.render-progress-logging]\n# note\nenabled = true\ninterval-seconds = 1\n";
        let out = set_toml(text, "settings.render-progress-logging", "enabled", "false").unwrap();
        assert_eq!(out, "[other]\nenabled = true\n[settings.render-progress-logging]\n# note\nenabled = false\ninterval-seconds = 1\n");
        let out = set_toml(&out, "settings.render-progress-logging", "interval-seconds", "5").unwrap();
        assert!(out.ends_with("interval-seconds = 5\n"));
        assert!(set_toml(text, "settings.missing", "enabled", "false").is_none());
    }

    #[test]
    fn yaml_value_is_replaced_by_path() {
        let text = "settings:\n  debug-mode: false\n  render-progress-logging:\n    enabled: true # log it\n    interval-seconds: 1\nworld-settings:\n  default:\n    enabled: true\n";
        let out = set_yaml(text, &["settings", "render-progress-logging", "enabled"], "false").unwrap();
        assert_eq!(
            out,
            "settings:\n  debug-mode: false\n  render-progress-logging:\n    enabled: false # log it\n    interval-seconds: 1\nworld-settings:\n  default:\n    enabled: true\n"
        );
        let out = set_yaml(&out, &["settings", "render-progress-logging", "interval-seconds"], "9").unwrap();
        assert!(out.contains("    interval-seconds: 9\n"));
        assert!(set_yaml(text, &["settings", "nope", "enabled"], "1").is_none());
    }
}
