//! Command tree helpers shared by the mods' `register_commands` (a copy lives in each mod as `src/cmdkit.rs`).
//!
//! The client asks the server for completions only at a node of a registered argument type (`ask_server`); a plain `word`
//! or `text` node gets none, so the `tab_complete` code of a mod never ran. `register` adds a pass-through `word` and `text`
//! type under the mod's namespace, `ask` rewrites a usage to name them, and the mod's own `tab_complete` (or the type's
//! `suggest` when a mod gives it one) answers. `rebuild` turns a parsed invocation back into the typed words.

#![allow(dead_code)]

use mod_api::{
    CommandArgumentError, CommandArgumentKind, CommandArgumentType, CommandArgumentValue, CommandInvocation, CommandSender,
    CommandSuggestion, Registrar,
};

/// A word or the rest of the line, accepted as typed; the completions come from the mod's `tab_complete`. `names`: the word
/// is a player's name (online or not) and completes from the players online (`joined` and `left` keep the list).
pub struct Ask {
    pub greedy: bool,
    pub names: bool,
}

static ONLINE: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

/// A player joined (`player_logged_in`): the name completes in a `player` argument.
pub fn joined(name: &str) {
    let mut online = ONLINE.lock().unwrap_or_else(|e| e.into_inner());
    if !online.iter().any(|n| n == name) {
        online.push(name.to_owned());
    }
}

/// A player left (`player_logged_out`).
pub fn left(name: &str) {
    ONLINE.lock().unwrap_or_else(|e| e.into_inner()).retain(|n| n != name);
}

impl CommandArgumentType for Ask {
    fn kind(&self) -> CommandArgumentKind {
        if self.greedy { CommandArgumentKind::Greedy } else { CommandArgumentKind::Word }
    }

    fn parse(&self, input: &str) -> Result<String, CommandArgumentError> {
        Ok(input.to_owned())
    }

    fn suggest(&self, _: Option<&CommandSender>, prefix: &str) -> Vec<CommandSuggestion> {
        if !self.names {
            return Vec::new();
        }
        let prefix = prefix.to_ascii_lowercase();
        let online = ONLINE.lock().unwrap_or_else(|e| e.into_inner());
        online.iter().filter(|n| n.to_ascii_lowercase().starts_with(&prefix)).cloned().map(CommandSuggestion::new).collect()
    }
}

/// Registers `word`, `text` and `player` (kinds `<mod id>:word`, `<mod id>:text`, `<mod id>:player`).
pub fn register(registrar: &mut dyn Registrar) {
    registrar.register_command_argument("word", Ask { greedy: false, names: false });
    registrar.register_command_argument("text", Ask { greedy: true, names: false });
    registrar.register_command_argument("player", Ask { greedy: false, names: true });
}

/// The usage with its `word`, `text` and `player_name` arguments naming the mod's pass-through types (`player_name` as a
/// plain `word`: the mod's `tab_complete` answers).
pub fn ask(usage: &str, ns: &str) -> String {
    ask_with(usage, ns, false)
}

/// `ask`, a `player_name` argument as the `player` type that completes the online names itself.
pub fn ask_names(usage: &str, ns: &str) -> String {
    ask_with(usage, ns, true)
}

fn ask_with(usage: &str, ns: &str, names: bool) -> String {
    usage
        .split_whitespace()
        .map(|token| {
            let (open, close) = if token.starts_with('<') { ('<', '>') } else if token.starts_with('[') { ('[', ']') } else { return token.to_owned() };
            let Some(inner) = token.strip_prefix(open).and_then(|t| t.strip_suffix(close)) else { return token.to_owned() };
            match inner.split_once(':') {
                Some((name, "player_name")) if names => format!("{open}{name}:{ns}:player{close}"),
                Some((name, "word" | "player_name")) => format!("{open}{name}:{ns}:word{close}"),
                Some((name, "text")) => format!("{open}{name}:{ns}:text{close}"),
                _ => token.to_owned(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// An argument's value as the typed word, for the kinds a usage here takes.
pub fn value_text(value: &CommandArgumentValue) -> Option<String> {
    match value {
        CommandArgumentValue::String(s) => Some(s.clone()),
        CommandArgumentValue::Custom { value, .. } => Some(value.clone()),
        CommandArgumentValue::Integer(i) => Some(i.to_string()),
        CommandArgumentValue::Number(n) => Some(n.to_string()),
        CommandArgumentValue::Boolean(b) => Some(b.to_string()),
        _ => None,
    }
}

/// The typed words (root included, as typed) of an invocation: the usage with the same command words is walked and each
/// argument replaced by the value the invocation holds, in the usage's order; an omitted optional argument is skipped.
pub fn rebuild(usages: &[String], command: &CommandInvocation) -> String {
    let values: Vec<(String, String)> =
        command.arguments.iter().filter_map(|a| value_text(&a.value).map(|v| (a.name.clone(), v))).collect();
    rebuild_words(usages, &command.literals, &values)
}

fn is_argument(token: &str) -> bool {
    token.starts_with('<') || token.starts_with('[')
}

fn argument_name(token: &str) -> &str {
    token[1..].split(':').next().unwrap_or("")
}

/// `rebuild` on plain data: the invocation's command words and its arguments by name.
pub fn rebuild_words(usages: &[String], words: &[String], values: &[(String, String)]) -> String {
    let template = usages.iter().find(|usage| {
        let tokens: Vec<&str> = usage.split_whitespace().collect();
        let literals: Vec<&str> = tokens.iter().copied().filter(|t| !is_argument(t)).collect();
        literals.len() == words.len()
            && literals.iter().skip(1).zip(words.iter().skip(1)).all(|(a, b)| a.eq_ignore_ascii_case(b))
            && values.iter().all(|(name, _)| tokens.iter().any(|t| is_argument(t) && argument_name(t) == name))
    });
    let Some(usage) = template else {
        return words.iter().cloned().chain(values.iter().map(|(_, v)| v.clone())).collect::<Vec<_>>().join(" ");
    };
    let mut out: Vec<String> = Vec::new();
    let mut at = 0;
    for token in usage.split_whitespace() {
        if is_argument(token) {
            if let Some((_, v)) = values.iter().find(|(n, _)| n == argument_name(token)) {
                out.push(v.clone());
            }
        } else {
            out.push(words.get(at).cloned().unwrap_or_else(|| token.to_owned()));
            at += 1;
        }
    }
    out.join(" ")
}

/// The words of the usages in order, with the arguments' kinds, for a test that asserts the tree.
#[cfg(test)]
pub fn literal_paths(usages: &[String]) -> Vec<String> {
    usages
        .iter()
        .map(|u| u.split_whitespace().filter(|t| !is_argument(t)).collect::<Vec<_>>().join(" "))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ask_names_the_mods_types() {
        assert_eq!(
            ask("rg define <id:word> [owners:text] -w <p:player_name> [n:integer]", "worldguard"),
            "rg define <id:worldguard:word> [owners:worldguard:text] -w <p:worldguard:word> [n:integer]"
        );
    }

    #[test]
    fn rebuild_keeps_the_usage_order() {
        let usages = vec!["rg define <id:word> [owners:text]".to_owned(), "rg define -g <id:word>".to_owned()];
        let words = ["rg".to_owned(), "define".to_owned(), "-g".to_owned()];
        assert_eq!(rebuild_words(&usages, &words, &[("id".into(), "spawn".into())]), "rg define -g spawn");
        let words = ["region".to_owned(), "define".to_owned()];
        let values = [("id".to_owned(), "spawn".to_owned()), ("owners".to_owned(), "A B".to_owned())];
        assert_eq!(rebuild_words(&usages, &words, &values), "region define spawn A B");
    }
}
