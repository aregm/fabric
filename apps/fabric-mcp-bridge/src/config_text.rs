//! The configuration text for the three clients (W06 6, A24).
//!
//! One function makes the text from the bridge's path and the agent's id; nothing else of the
//! system is in it. Each string is escaped for its format: JSON through `serde_json`, TOML
//! basic strings with `\"`, `\\` and `\uXXXX`, the shell line with single quotes. Nothing here
//! writes into a client's file.

use std::fmt;

use fabric_desktop_contracts::scalar::Id;

/// The server's name in all three formats.
pub const SERVER_NAME: &str = "fabric-calendar";

/// The first line of a text made by a debug build.
pub const TEST_LINE: &str = "Test configuration from a debug build of fabric-mcp-bridge; it is not for an installed Fabric Calendar.";

/// The client a text is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Client {
    ClaudeDesktop,
    ClaudeCode,
    Codex,
}

impl Client {
    pub const ALL: [Client; 3] = [Client::ClaudeDesktop, Client::ClaudeCode, Client::Codex];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Client::ClaudeDesktop => "claude-desktop",
            Client::ClaudeCode => "claude-code",
            Client::Codex => "codex",
        }
    }

    #[must_use]
    pub fn from_name(name: &str) -> Option<Client> {
        Client::ALL.into_iter().find(|c| c.as_str() == name)
    }
}

/// The debug seams a practice run's text carries (W06 6, A23).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DebugPaths {
    pub socket: String,
    pub key_dir: String,
}

/// Which paths a text may name: a release build only a bridge under `/Applications/` or
/// `$HOME/Applications/`; a debug build any absolute path (W06 6).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PathRule {
    pub debug_build: bool,
    /// `HOME`, for `$HOME/Applications/`.
    pub home: Option<String>,
}

/// Why no text was made. The messages are fixed and never repeat a path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConfigError {
    /// A path is not absolute.
    NotAbsolute,
    /// A path holds a control character or a character that changes the direction of text.
    ForbiddenCharacter,
    /// A release build makes a text only for a bridge in an application folder.
    NotInstalled,
    /// A release build has no debug seams.
    DebugOnly,
}

impl ConfigError {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            ConfigError::NotAbsolute => "a path is not absolute",
            ConfigError::ForbiddenCharacter => {
                "a path holds a control character or a direction mark"
            }
            ConfigError::NotInstalled => {
                "the bridge is not in /Applications or the home folder's Applications"
            }
            ConfigError::DebugOnly => "--socket and --key-dir exist only in debug builds",
        }
    }
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::error::Error for ConfigError {}

/// Whether `c` changes the direction of text (the bidirectional marks, embeddings, overrides
/// and isolates), separates lines or paragraphs, or is invisible (zero-width characters, the
/// word joiner and the invisible operators, the byte-order mark).
fn is_direction_mark(c: char) -> bool {
    matches!(
        c,
        '\u{061C}'
            | '\u{200B}'..='\u{200F}'
            | '\u{2028}'
            | '\u{2029}'
            | '\u{202A}'..='\u{202E}'
            | '\u{2060}'..='\u{206F}'
            | '\u{FEFF}'
    )
}

fn check_text(path: &str) -> Result<(), ConfigError> {
    if path.chars().any(|c| c.is_control() || is_direction_mark(c)) {
        return Err(ConfigError::ForbiddenCharacter);
    }
    if !path.starts_with('/') {
        return Err(ConfigError::NotAbsolute);
    }
    Ok(())
}

/// A path strictly under `folder`, with no `..` on the way. Paths are macOS paths, read the
/// same way on every system.
fn is_under(path: &str, folder: &str) -> bool {
    path.strip_prefix(folder)
        .and_then(|rest| rest.strip_prefix('/'))
        .is_some_and(|rest| !rest.is_empty() && !rest.split('/').any(|part| part == ".."))
}

fn check_installed(path: &str, home: Option<&str>) -> Result<(), ConfigError> {
    let in_home = home
        .filter(|h| h.starts_with('/'))
        .is_some_and(|h| is_under(path, &format!("{}/Applications", h.trim_end_matches('/'))));
    let installed = is_under(path, "/Applications") || in_home;
    if !installed || path.contains("AppTranslocation") || is_under(path, "/Volumes") {
        return Err(ConfigError::NotInstalled);
    }
    Ok(())
}

/// A JSON string.
#[must_use]
pub fn json_string(s: &str) -> String {
    serde_json::Value::String(s.to_owned()).to_string()
}

/// A TOML basic string: `\"`, `\\`, and `\uXXXX` for every control character.
#[must_use]
pub fn toml_string(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if c.is_control() => out.push_str(&format!("\\u{:04X}", u32::from(c))),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// One shell word in single quotes; a single quote inside is written `'\''`.
#[must_use]
pub fn shell_word(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// The arguments the client passes to the bridge.
fn arguments(id: &Id, extras: Option<&DebugPaths>) -> Vec<String> {
    let mut args = vec!["--enrollment".to_owned(), id.as_str().to_owned()];
    if let Some(extras) = extras {
        args.extend([
            "--socket".to_owned(),
            extras.socket.clone(),
            "--key-dir".to_owned(),
            extras.key_dir.clone(),
        ]);
    }
    args
}

fn json_entry(bridge_path: &str, args: &[String], indent: &str) -> String {
    let list: Vec<String> = args.iter().map(|a| json_string(a)).collect();
    format!(
        "{indent}{}: {{\n{indent}  \"command\": {},\n{indent}  \"args\": [{}]\n{indent}}}",
        json_string(SERVER_NAME),
        json_string(bridge_path),
        list.join(", ")
    )
}

/// The configuration text for `client` (W06 6). `bridge_path` is the bridge that will run;
/// `debug_extras` are the debug seams of a practice run.
pub fn config_text(
    client: Client,
    bridge_path: &str,
    id: &Id,
    debug_extras: Option<&DebugPaths>,
    rule: &PathRule,
) -> Result<String, ConfigError> {
    check_text(bridge_path)?;
    if let Some(extras) = debug_extras {
        if !rule.debug_build {
            return Err(ConfigError::DebugOnly);
        }
        check_text(&extras.socket)?;
        check_text(&extras.key_dir)?;
    }
    if !rule.debug_build {
        check_installed(bridge_path, rule.home.as_deref())?;
    }
    let args = arguments(id, debug_extras);
    let mut text = String::new();
    if rule.debug_build {
        text.push_str(TEST_LINE);
        text.push('\n');
    }
    match client {
        Client::ClaudeDesktop => {
            text.push_str("Add the fabric-calendar entry below to mcpServers in ~/Library/Application Support/Claude/claude_desktop_config.json (Settings, Developer, Edit Config), beside the servers already there; then quit Claude Desktop and open it again.\n\n");
            text.push_str(&json_entry(bridge_path, &args, ""));
            text.push('\n');
        }
        Client::ClaudeCode => {
            text.push_str("Save this as .mcp.json in a folder of its own, used only for calendar work. Claude Code asks once before it uses a server from .mcp.json.\n");
            text.push_str("Do not add the entry for the user (--scope user), and do not put it into the Fabric repository or, to be safe, into a folder above it. A user-wide entry reaches every Claude Code session on this Mac, and so the sessions that build Fabric. Sessions that other programs start load a folder's .mcp.json without asking.\n\n");
            text.push_str("{\n  \"mcpServers\": {\n");
            text.push_str(&json_entry(bridge_path, &args, "    "));
            text.push_str("\n  }\n}\n\n");
            text.push_str("The same entry can be added from that folder with the line below. Leaving out --scope project keeps the entry out of the folder and private to you, for that folder only.\n\n");
            let mut line = format!(
                "claude mcp add --scope project {SERVER_NAME} -- {}",
                shell_word(bridge_path)
            );
            for (i, a) in args.iter().enumerate() {
                line.push(' ');
                // The flags and the id are plain words; the paths are quoted.
                if i % 2 == 0 || i == 1 {
                    line.push_str(a);
                } else {
                    line.push_str(&shell_word(a));
                }
            }
            text.push_str(&line);
            text.push('\n');
        }
        Client::Codex => {
            text.push_str("Add this to .codex/config.toml in a folder of its own, used only for calendar work; Codex reads it for a trusted project. ~/.codex/config.toml also works, but the ChatGPT desktop application, the Codex command line and the editor extension all share it, so the folder is the safer place.\n\n");
            let list: Vec<String> = args.iter().map(|a| toml_string(a)).collect();
            text.push_str(&format!(
                "[mcp_servers.{SERVER_NAME}]\ncommand = {}\nargs = [{}]\n",
                toml_string(bridge_path),
                list.join(", ")
            ));
        }
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    const ODD: &str = "/Applications/Fab ric \"q\" 'single' $HOME `tick` back\\slash.app/Contents/MacOS/fabric-mcp-bridge";

    fn debug() -> PathRule {
        PathRule {
            debug_build: true,
            home: Some("/Users/someone".to_owned()),
        }
    }

    fn release() -> PathRule {
        PathRule {
            debug_build: false,
            home: Some("/Users/someone".to_owned()),
        }
    }

    fn id() -> Id {
        Id::new("agent-0123456789abcdef").unwrap()
    }

    fn blocks(text: &str) -> Vec<&str> {
        text.split("\n\n").collect()
    }

    /// Reads a TOML basic string at the start of `s`; returns it and the rest.
    fn toml_read(s: &str) -> (String, &str) {
        let mut chars = s.char_indices();
        assert_eq!(chars.next().map(|c| c.1), Some('"'));
        let mut out = String::new();
        while let Some((i, c)) = chars.next() {
            match c {
                '"' => return (out, &s[i + 1..]),
                '\\' => match chars.next().unwrap().1 {
                    '"' => out.push('"'),
                    '\\' => out.push('\\'),
                    'u' => {
                        let hex: String = (0..4).map(|_| chars.next().unwrap().1).collect();
                        out.push(char::from_u32(u32::from_str_radix(&hex, 16).unwrap()).unwrap());
                    }
                    other => panic!("escape {other}"),
                },
                c => out.push(c),
            }
        }
        panic!("unterminated");
    }

    /// Splits a POSIX shell line of plain words, single quotes and backslash escapes.
    fn shell_split(line: &str) -> Vec<String> {
        let mut words = Vec::new();
        let mut word: Option<String> = None;
        let mut chars = line.chars();
        while let Some(c) = chars.next() {
            match c {
                ' ' => {
                    if let Some(w) = word.take() {
                        words.push(w);
                    }
                }
                '\'' => {
                    let w = word.get_or_insert_with(String::new);
                    for q in chars.by_ref() {
                        if q == '\'' {
                            break;
                        }
                        w.push(q);
                    }
                }
                '\\' => word
                    .get_or_insert_with(String::new)
                    .push(chars.next().unwrap()),
                c => word.get_or_insert_with(String::new).push(c),
            }
        }
        words.extend(word);
        words
    }

    fn command_and_args(client: Client, text: &str) -> (String, Vec<String>) {
        match client {
            Client::ClaudeDesktop => {
                let block = blocks(text)
                    .into_iter()
                    .find(|b| b.starts_with("\"fabric-calendar\""))
                    .unwrap();
                let v: Value = serde_json::from_str(&format!("{{{block}}}")).unwrap();
                let e = &v["fabric-calendar"];
                (
                    e["command"].as_str().unwrap().to_owned(),
                    e["args"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|a| a.as_str().unwrap().to_owned())
                        .collect(),
                )
            }
            Client::ClaudeCode => {
                let block = blocks(text)
                    .into_iter()
                    .find(|b| b.starts_with('{'))
                    .unwrap();
                let v: Value = serde_json::from_str(block).unwrap();
                let e = &v["mcpServers"]["fabric-calendar"];
                (
                    e["command"].as_str().unwrap().to_owned(),
                    e["args"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|a| a.as_str().unwrap().to_owned())
                        .collect(),
                )
            }
            Client::Codex => {
                let block = blocks(text)
                    .into_iter()
                    .find(|b| b.starts_with("[mcp_servers"))
                    .unwrap();
                let mut lines = block.lines();
                assert_eq!(lines.next(), Some("[mcp_servers.fabric-calendar]"));
                let command = lines.next().unwrap().strip_prefix("command = ").unwrap();
                let (command, rest) = toml_read(command);
                assert!(rest.is_empty());
                let mut list = lines.next().unwrap().strip_prefix("args = [").unwrap();
                let mut args = Vec::new();
                loop {
                    let (a, rest) = toml_read(list);
                    args.push(a);
                    if let Some(r) = rest.strip_prefix(", ") {
                        list = r;
                    } else {
                        assert_eq!(rest, "]");
                        break;
                    }
                }
                (command, args)
            }
        }
    }

    // c01: the three texts for a path with a space, a double quote, a single quote, $, a
    // backtick and a backslash parse back to the same path, the shell line included.
    #[test]
    fn c01_escaping() {
        let extras = DebugPaths {
            socket: "/tmp/a b/'s\"/run/agent.sock".to_owned(),
            key_dir: "/tmp/k $x `y` \\z".to_owned(),
        };
        for rule in [release(), debug()] {
            let extra = rule.debug_build.then_some(&extras);
            for client in Client::ALL {
                let text = config_text(client, ODD, &id(), extra, &rule).unwrap();
                let (command, args) = command_and_args(client, &text);
                assert_eq!(command, ODD, "{}", client.as_str());
                let mut expected = vec!["--enrollment".to_owned(), id().as_str().to_owned()];
                if let Some(e) = extra {
                    expected.extend([
                        "--socket".to_owned(),
                        e.socket.clone(),
                        "--key-dir".to_owned(),
                        e.key_dir.clone(),
                    ]);
                }
                assert_eq!(args, expected);
                if client == Client::ClaudeCode {
                    let line = text
                        .lines()
                        .find(|l| l.starts_with("claude mcp add"))
                        .unwrap();
                    let words = shell_split(line);
                    let mut want: Vec<String> = "claude mcp add --scope project fabric-calendar --"
                        .split(' ')
                        .map(str::to_owned)
                        .collect();
                    want.push(ODD.to_owned());
                    want.extend(expected.clone());
                    assert_eq!(words, want);
                }
            }
        }
    }

    // c02: a text holds the path and the id and nothing else; the debug extras appear only
    // when given; a path with a line feed, a tab or a direction mark is refused; a release
    // build refuses a path outside the two application folders.
    #[test]
    fn c02_contents_and_refusals() {
        let other_id = Id::new("agent-zz").unwrap();
        let p1 = "/Applications/Fabric Calendar.app/Contents/MacOS/fabric-mcp-bridge";
        let p2 = "/Applications/Other.app/Contents/MacOS/fabric-mcp-bridge";
        for client in Client::ALL {
            for rule in [release(), debug()] {
                let a = config_text(client, p1, &id(), None, &rule).unwrap();
                let b = config_text(client, p2, &other_id, None, &rule).unwrap();
                // With the path and the id taken out, the two texts are the same.
                assert_eq!(
                    a.replace(p1, "P").replace(id().as_str(), "I"),
                    b.replace(p2, "P").replace(other_id.as_str(), "I"),
                );
                assert!(a.contains(p1) && a.contains(id().as_str()));
                assert!(!a.contains("--socket") && !a.contains("--key-dir"));
                assert!(!a.contains("someone"), "HOME is not in the text");
                assert_eq!(a.starts_with(TEST_LINE), rule.debug_build);
            }
            let extras = DebugPaths {
                socket: "/s/run/agent.sock".to_owned(),
                key_dir: "/k".to_owned(),
            };
            let with = config_text(client, p1, &id(), Some(&extras), &debug()).unwrap();
            assert!(with.contains("--socket") && with.contains("/s/run/agent.sock"));
            assert!(with.contains("--key-dir"));
            assert_eq!(
                config_text(client, p1, &id(), Some(&extras), &release()),
                Err(ConfigError::DebugOnly)
            );
            for bad in [
                "/Applications/a\nb/fabric-mcp-bridge",
                "/Applications/a\tb/fabric-mcp-bridge",
                "/Applications/a\u{202E}b/fabric-mcp-bridge",
                "/Applications/a\u{200F}b/fabric-mcp-bridge",
                "/Applications/a\u{2028}b/fabric-mcp-bridge",
                "/Applications/a\u{2029}b/fabric-mcp-bridge",
                "/Applications/a\u{200B}b/fabric-mcp-bridge",
                "/Applications/a\u{200D}b/fabric-mcp-bridge",
                "/Applications/a\u{2060}b/fabric-mcp-bridge",
                "/Applications/a\u{2066}b/fabric-mcp-bridge",
                "/Applications/a\u{206F}b/fabric-mcp-bridge",
                "/Applications/a\u{FEFF}b/fabric-mcp-bridge",
            ] {
                for rule in [release(), debug()] {
                    assert_eq!(
                        config_text(client, bad, &id(), None, &rule),
                        Err(ConfigError::ForbiddenCharacter)
                    );
                }
            }
            let bad_extras = DebugPaths {
                socket: "/s/\n/agent.sock".to_owned(),
                key_dir: "/k".to_owned(),
            };
            assert_eq!(
                config_text(client, p1, &id(), Some(&bad_extras), &debug()),
                Err(ConfigError::ForbiddenCharacter)
            );
            for outside in [
                "/Users/someone/Downloads/fabric-mcp-bridge",
                "/Volumes/Fabric/Fabric Calendar.app/Contents/MacOS/fabric-mcp-bridge",
                "/private/var/folders/x/AppTranslocation/y/Fabric Calendar.app/Contents/MacOS/fabric-mcp-bridge",
                "/Applications/../tmp/fabric-mcp-bridge",
                "/Applications",
                "/ApplicationsX/fabric-mcp-bridge",
            ] {
                assert_eq!(
                    config_text(client, outside, &id(), None, &release()),
                    Err(ConfigError::NotInstalled),
                    "{outside}"
                );
                assert!(config_text(client, outside, &id(), None, &debug()).is_ok());
            }
            assert!(
                config_text(
                    client,
                    "/Users/someone/Applications/Fabric Calendar.app/Contents/MacOS/fabric-mcp-bridge",
                    &id(),
                    None,
                    &release()
                )
                .is_ok()
            );
            assert_eq!(
                config_text(client, "relative/fabric-mcp-bridge", &id(), None, &debug()),
                Err(ConfigError::NotAbsolute)
            );
        }
    }
}
