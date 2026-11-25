//! MiniMessage for the language files (`locale/lang-*.yml`): a message from the file keeps its colour, click and hover
//! tags. `commands::tr` hands such a message out behind [`MARK`] with the placeholder values escaped ([`escape`]);
//! [`expand`] turns the part that carries it into styled parts. Supported: named and `#RRGGBB` colours (`<red>`,
//! `<color:gold>`), `<click:run_command|open_url:...>`, `<hover:show_text:...>`, `<reset>`, `<newline>` and the closing
//! tags; decorations are accepted and dropped (the text model of a command line has none); other tags stay as text.
use crate::horizon_squaremap::commands::{Click, Part};

/// First character of a part whose text still holds MiniMessage tags.
pub const MARK: char = '\u{1}';
/// Stands for a `<` that came in through a placeholder value, so it is not read as a tag.
const LT: char = '\u{2}';

/// A placeholder value, safe to put between tags.
pub fn escape(value: &str) -> String {
    value.replace('<', &LT.to_string())
}

fn restore(text: &str) -> String {
    text.replace(LT, "<")
}

/// The end (index of the closing `>`) of the tag that opens at `start` (a `<`), with nested tags and quotes in its
/// arguments; None for a `<` that opens nothing.
pub fn tag_end(text: &str, start: usize) -> Option<usize> {
    let mut depth = 0usize;
    let mut quote: Option<char> = None;
    for (i, c) in text[start..].char_indices() {
        match (c, quote) {
            ('\'' | '"', None) if i > 0 => quote = Some(c),
            (q, Some(open)) if q == open => quote = None,
            ('<', None) => depth += 1,
            ('>', None) => {
                depth -= 1;
                if depth == 0 {
                    return Some(start + i);
                }
            }
            _ => {}
        }
    }
    None
}

/// `template` with `<name>` placeholders replaced by their escaped values (also inside tag arguments, as in
/// `<click:open_url:'<link>'>`); every other tag stays.
pub fn fill_rich(template: &str, vars: &[(&str, &str)]) -> String {
    let mut out = String::new();
    let mut at = 0;
    while let Some(rel) = template[at..].find('<') {
        let open = at + rel;
        out.push_str(&template[at..open]);
        match tag_end(template, open) {
            Some(end) => {
                let inner = &template[open + 1..end];
                match vars.iter().find(|(name, _)| *name == inner) {
                    Some((_, value)) => out.push_str(&escape(value)),
                    None => {
                        out.push('<');
                        out.push_str(&fill_rich(inner, vars));
                        out.push('>');
                    }
                }
                at = end + 1;
            }
            None => {
                out.push('<');
                at = open + 1;
            }
        }
    }
    out.push_str(&template[at..]);
    out
}

fn named(name: &str) -> Option<u32> {
    Some(match name {
        "black" => 0x000000,
        "dark_blue" => 0x0000AA,
        "dark_green" => 0x00AA00,
        "dark_aqua" => 0x00AAAA,
        "dark_red" => 0xAA0000,
        "dark_purple" => 0xAA00AA,
        "gold" => 0xFFAA00,
        "gray" | "grey" => 0xAAAAAA,
        "dark_gray" | "dark_grey" => 0x555555,
        "blue" => 0x5555FF,
        "green" => 0x55FF55,
        "aqua" => 0x55FFFF,
        "red" => 0xFF5555,
        "light_purple" => 0xFF55FF,
        "yellow" => 0xFFFF55,
        "white" => 0xFFFFFF,
        _ => return None,
    })
}

fn colour(spec: &str) -> Option<u32> {
    match spec.strip_prefix('#') {
        Some(hex) if hex.len() == 6 => u32::from_str_radix(hex, 16).ok(),
        Some(_) => None,
        None => named(spec),
    }
}

const DECORATIONS: [&str; 12] = [
    "bold", "b", "italic", "i", "em", "underlined", "u", "strikethrough", "st", "obfuscated", "obf", "font",
];

/// Splits a tag's inside at the colons outside quotes; quotes around an argument go.
fn split_args(inner: &str) -> Vec<String> {
    let (mut args, mut cur, mut quote) = (Vec::new(), String::new(), None);
    for c in inner.chars() {
        match (c, quote) {
            ('\'' | '"', None) => quote = Some(c),
            (q, Some(open)) if q == open => quote = None,
            (':', None) => args.push(std::mem::take(&mut cur)),
            _ => cur.push(c),
        }
    }
    args.push(cur);
    args
}

/// The text of a hover argument without tags.
fn plain_text(text: &str) -> String {
    let mut out = String::new();
    let mut at = 0;
    while let Some(rel) = text[at..].find('<') {
        let open = at + rel;
        out.push_str(&text[at..open]);
        match tag_end(text, open) {
            Some(end) => at = end + 1,
            None => {
                out.push('<');
                at = open + 1;
            }
        }
    }
    out.push_str(&text[at..]);
    out
}

/// A hover text from the language file, without its tags.
fn clean(text: &str) -> String {
    match text.strip_prefix(MARK) {
        Some(body) => restore(&plain_text(body)),
        None => text.to_owned(),
    }
}

#[derive(Default)]
struct Frame {
    name: String,
    color: Option<u32>,
    click: Option<Click>,
    hover: Option<String>,
}

/// The styled parts of `part` when its text is a MiniMessage message ([`MARK`]); a part without the mark as is. The
/// part's own colour, click and hover are what the tags start from.
pub fn expand(part: &Part) -> Vec<Part> {
    let part = &Part { hover: part.hover.as_deref().map(clean), ..part.clone() };
    let Some(body) = part.text.strip_prefix(MARK) else { return vec![part.clone()] };
    let mut out: Vec<Part> = Vec::new();
    let mut stack: Vec<Frame> = Vec::new();
    let mut buf = String::new();
    let flush = |buf: &mut String, stack: &[Frame], out: &mut Vec<Part>| {
        if buf.is_empty() {
            return;
        }
        out.push(Part {
            text: restore(&std::mem::take(buf)),
            color: stack.iter().rev().find_map(|f| f.color).or(part.color),
            click: stack.iter().rev().find_map(|f| f.click.clone()).or_else(|| part.click.clone()),
            hover: stack.iter().rev().find_map(|f| f.hover.clone()).or_else(|| part.hover.clone()),
        });
    };
    let mut at = 0;
    while let Some(rel) = body[at..].find('<') {
        let open = at + rel;
        buf.push_str(&body[at..open]);
        let Some(end) = tag_end(body, open) else {
            buf.push('<');
            at = open + 1;
            continue;
        };
        let inner = &body[open + 1..end];
        at = end + 1;
        if let Some(closing) = inner.strip_prefix('/') {
            let name = closing.split(':').next().unwrap_or("").to_ascii_lowercase();
            if let Some(i) = stack.iter().rposition(|f| f.name == name) {
                flush(&mut buf, &stack, &mut out);
                stack.truncate(i);
            } else {
                buf.push_str(&body[open..at]); // a closing tag nothing opened is text
            }
            continue;
        }
        let args = split_args(&restore(inner));
        let name = args[0].to_ascii_lowercase();
        let frame = match name.as_str() {
            "reset" => {
                flush(&mut buf, &stack, &mut out);
                stack.clear();
                continue;
            }
            "newline" | "br" => {
                buf.push('\n');
                continue;
            }
            "color" | "colour" | "c" => args.get(1).and_then(|s| colour(&s.to_ascii_lowercase())).map(|c| Frame { name: name.clone(), color: Some(c), ..Frame::default() }),
            "click" if args.len() >= 3 => {
                let value = args[2..].join(":");
                match args[1].as_str() {
                    "run_command" => Some(Frame { name: name.clone(), click: Some(Click::Run(value)), ..Frame::default() }),
                    "open_url" => Some(Frame { name: name.clone(), click: Some(Click::Open(value)), ..Frame::default() }),
                    _ => Some(Frame { name: name.clone(), ..Frame::default() }),
                }
            }
            "hover" if args.len() >= 3 && args[1] == "show_text" => {
                Some(Frame { name: name.clone(), hover: Some(plain_text(&args[2..].join(":"))), ..Frame::default() })
            }
            n if DECORATIONS.contains(&n) => Some(Frame { name: name.clone(), ..Frame::default() }),
            n => colour(n).map(|c| Frame { name: name.clone(), color: Some(c), ..Frame::default() }),
        };
        match frame {
            Some(f) => {
                flush(&mut buf, &stack, &mut out);
                stack.push(f);
            }
            None => buf.push_str(&body[open..at]), // an unknown tag is text
        }
    }
    buf.push_str(&body[at..]);
    flush(&mut buf, &stack, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base(color: Option<u32>) -> Part {
        Part { text: String::new(), color, hover: None, click: None }
    }

    fn rich(template: &str, vars: &[(&str, &str)], color: Option<u32>) -> Vec<Part> {
        let mut p = base(color);
        p.text = format!("{MARK}{}", fill_rich(template, vars));
        expand(&p)
    }

    #[test]
    fn colours_and_closing_tags() {
        let parts = rich("<red>Bad: </red>fine <#112233>x", &[], Some(0xAAAAAA));
        let seen: Vec<(String, Option<u32>)> = parts.iter().map(|p| (p.text.clone(), p.color)).collect();
        assert_eq!(
            seen,
            [("Bad: ".into(), Some(0xFF5555)), ("fine ".into(), Some(0xAAAAAA)), ("x".into(), Some(0x112233))]
        );
    }

    #[test]
    fn placeholders_fill_text_and_tag_arguments_and_stay_literal() {
        let parts = rich(
            "Click to open map: <green><click:open_url:'<link>'><link>",
            &[("link", "http://127.0.0.1:8080/?world=a<b")],
            None,
        );
        assert_eq!(parts[0].text, "Click to open map: ");
        assert_eq!(parts[1].text, "http://127.0.0.1:8080/?world=a<b");
        assert_eq!(parts[1].color, Some(0x55FF55));
        assert_eq!(parts[1].click, Some(Click::Open("http://127.0.0.1:8080/?world=a<b".into())));
    }

    #[test]
    fn hover_run_command_and_unknown_tags() {
        let parts = rich("<click:run_command:/map help><hover:show_text:'<gray>Help'>go</hover></click> <foo>", &[], None);
        assert_eq!(parts[0].text, "go");
        assert_eq!(parts[0].click, Some(Click::Run("/map help".into())));
        assert_eq!(parts[0].hover.as_deref(), Some("Help"));
        assert_eq!(parts[1].text, " <foo>");
        assert_eq!(parts[1].click, None);
    }

    #[test]
    fn a_plain_part_is_left_alone() {
        let mut p = base(Some(1));
        p.text = "No <tags> here".into();
        assert_eq!(expand(&p), vec![p]);
    }
}
