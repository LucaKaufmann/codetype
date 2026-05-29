//! Vim-style `:` command palette parser.
//!
//! Parses lines like `:open ~/foo` into a typed [`Command`]. The leading `:`
//! is optional, so callers can pass either the raw user input or the post-`:`
//! tail. Arguments are "rest of line" — internal whitespace is preserved —
//! so `:open path with spaces` is one path, not three arguments.

use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Open(String),
    File(String),
    Stats,
    Quit,
    Help,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ParseError {
    #[error("empty command")]
    Empty,
    #[error("unknown command: {0}")]
    Unknown(String),
    #[error("missing argument for :{0}")]
    MissingArg(&'static str),
    #[error("unexpected argument for :{0}")]
    UnexpectedArg(&'static str),
}

/// The command vocabulary, in display order.
const COMMANDS: &[&str] = &["open", "file", "stats", "quit", "help"];

/// Parse a command-palette line into a [`Command`].
///
/// Accepts input with or without a leading `:`. Surrounding whitespace is
/// trimmed; internal whitespace in arguments is preserved.
pub fn parse(input: &str) -> Result<Command, ParseError> {
    let body = input.trim().trim_start_matches(':').trim_start();
    if body.is_empty() {
        return Err(ParseError::Empty);
    }

    // Split into command name + the rest of the line. `split_once` returns
    // `(before, after)` at the first occurrence; we then trim the "after"
    // because the user almost certainly typed "open path" not "open  path".
    let (cmd, rest) = match body.split_once(' ') {
        Some((c, r)) => (c, r.trim()),
        None => (body, ""),
    };

    match cmd {
        "open" => arg_required("open", rest).map(|s| Command::Open(s.to_string())),
        "file" => arg_required("file", rest).map(|s| Command::File(s.to_string())),
        "stats" => no_arg("stats", rest).map(|()| Command::Stats),
        "quit" => no_arg("quit", rest).map(|()| Command::Quit),
        "help" => no_arg("help", rest).map(|()| Command::Help),
        other => Err(ParseError::Unknown(other.to_string())),
    }
}

/// Return the command names that match a partial input, in display order.
///
/// `complete(":")` and `complete("")` both return every command. If the
/// input already contains a space (i.e. the user has moved past the command
/// name into the argument), this function returns an empty `Vec` — argument
/// completion is the app layer's concern.
pub fn complete(input: &str) -> Vec<&'static str> {
    let body = input.trim_start().trim_start_matches(':');

    if body.contains(' ') {
        return Vec::new();
    }

    COMMANDS
        .iter()
        .copied()
        .filter(|cmd| cmd.starts_with(body))
        .collect()
}

// -- Palette suggestion metadata ---------------------------------------------

/// Display metadata for a command, surfaced in the palette suggestion list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommandInfo {
    /// Bare command name, e.g. "stats".
    pub name: &'static str,
    /// One-line description for the palette.
    pub description: &'static str,
    /// Argument shape shown after the name, e.g. "<path>". Empty for no-arg.
    pub arg: &'static str,
    /// True for commands that parse but aren't wired up yet (:file, :open).
    /// The palette marks these "(not yet)" instead of offering them as live.
    pub available: bool,
}

/// Metadata for every command, in display order.
///
/// INVARIANT: this must list the same names in the same order as [`COMMANDS`].
/// `command_info_matches_commands_order` (in tests) pins the two together so
/// they can't silently drift. `open`/`file` are `available: false` because the
/// app layer's `execute_command` still surfaces a "not yet implemented" notice
/// for them; `stats`/`quit`/`help` are wired up and therefore `true`.
const COMMAND_INFO: &[CommandInfo] = &[
    CommandInfo {
        name: "open",
        description: "open a different repo",
        arg: "<path>",
        available: false,
    },
    CommandInfo {
        name: "file",
        description: "jump to a specific file",
        arg: "<name>",
        available: false,
    },
    CommandInfo {
        name: "stats",
        description: "open the session stats view",
        arg: "",
        available: true,
    },
    CommandInfo {
        name: "quit",
        description: "quit codetype",
        arg: "",
        available: true,
    },
    CommandInfo {
        name: "help",
        description: "show keyboard help",
        arg: "",
        available: true,
    },
];

/// Metadata for the commands matching `input`, in display order. Mirrors
/// [`complete`] but returns full descriptions/arg-shapes for the palette.
/// Past the first space (argument territory) returns empty, like `complete`.
pub fn complete_info(input: &str) -> Vec<CommandInfo> {
    let names = complete(input);
    COMMAND_INFO
        .iter()
        .copied()
        .filter(|info| names.contains(&info.name))
        .collect()
}

/// Look up a single command's metadata by name (`None` if unknown).
pub fn info(name: &str) -> Option<CommandInfo> {
    COMMAND_INFO.iter().copied().find(|i| i.name == name)
}

// -- Tiny private helpers ----------------------------------------------------

fn arg_required<'a>(name: &'static str, rest: &'a str) -> Result<&'a str, ParseError> {
    if rest.is_empty() {
        Err(ParseError::MissingArg(name))
    } else {
        Ok(rest)
    }
}

fn no_arg(name: &'static str, rest: &str) -> Result<(), ParseError> {
    if rest.is_empty() {
        Ok(())
    } else {
        Err(ParseError::UnexpectedArg(name))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    // -- parse: empty / whitespace --

    #[test]
    fn empty_input_is_empty_error() {
        assert_eq!(parse(""), Err(ParseError::Empty));
    }

    #[test]
    fn whitespace_only_input_is_empty_error() {
        assert_eq!(parse("   "), Err(ParseError::Empty));
    }

    #[test]
    fn just_colon_is_empty_error() {
        assert_eq!(parse(":"), Err(ParseError::Empty));
    }

    #[test]
    fn surrounding_whitespace_is_trimmed() {
        assert_eq!(parse("  :quit  "), Ok(Command::Quit));
    }

    // -- parse: leading colon is optional --

    #[test]
    fn leading_colon_optional() {
        assert_eq!(parse(":quit"), Ok(Command::Quit));
        assert_eq!(parse("quit"), Ok(Command::Quit));
    }

    // -- parse: no-arg commands --

    #[test]
    fn parses_stats() {
        assert_eq!(parse(":stats"), Ok(Command::Stats));
    }

    #[test]
    fn parses_quit() {
        assert_eq!(parse(":quit"), Ok(Command::Quit));
    }

    #[test]
    fn parses_help() {
        assert_eq!(parse(":help"), Ok(Command::Help));
    }

    #[test]
    fn no_arg_command_with_extra_arg_errors() {
        assert_eq!(parse(":quit foo"), Err(ParseError::UnexpectedArg("quit")));
        assert_eq!(parse(":stats x"), Err(ParseError::UnexpectedArg("stats")));
        assert_eq!(parse(":help x"), Err(ParseError::UnexpectedArg("help")));
    }

    // -- parse: arg-required commands --

    #[test]
    fn parses_open_with_path() {
        assert_eq!(parse(":open ~/foo"), Ok(Command::Open("~/foo".to_string())));
    }

    #[test]
    fn parses_file_with_name() {
        assert_eq!(
            parse(":file Order.swift"),
            Ok(Command::File("Order.swift".to_string()))
        );
    }

    #[test]
    fn arg_with_internal_spaces_is_preserved() {
        // Rest-of-line semantics: internal whitespace stays.
        assert_eq!(
            parse(":open path with spaces"),
            Ok(Command::Open("path with spaces".to_string()))
        );
    }

    #[test]
    fn missing_arg_errors() {
        assert_eq!(parse(":open"), Err(ParseError::MissingArg("open")));
        assert_eq!(parse(":file"), Err(ParseError::MissingArg("file")));
        // Trailing whitespace alone doesn't count as an argument.
        assert_eq!(parse(":open   "), Err(ParseError::MissingArg("open")));
    }

    // -- parse: unknown commands --

    #[test]
    fn unknown_command_errors() {
        assert_eq!(parse(":nope"), Err(ParseError::Unknown("nope".to_string())));
    }

    // -- complete --

    #[test]
    fn complete_empty_returns_all_commands() {
        assert_eq!(complete(""), vec!["open", "file", "stats", "quit", "help"]);
    }

    #[test]
    fn complete_just_colon_returns_all_commands() {
        assert_eq!(complete(":"), vec!["open", "file", "stats", "quit", "help"]);
    }

    #[test]
    fn complete_prefix_filters() {
        assert_eq!(complete(":f"), vec!["file"]);
        assert_eq!(complete(":h"), vec!["help"]);
        assert_eq!(complete(":s"), vec!["stats"]);
    }

    #[test]
    fn complete_full_command_returns_itself() {
        assert_eq!(complete(":quit"), vec!["quit"]);
    }

    #[test]
    fn complete_no_match_returns_empty() {
        assert_eq!(complete(":xyz"), Vec::<&'static str>::new());
    }

    #[test]
    fn complete_past_first_space_returns_empty() {
        // Once the user has typed an argument, command completion is done.
        assert_eq!(complete(":file "), Vec::<&'static str>::new());
        assert_eq!(complete(":open ~/"), Vec::<&'static str>::new());
    }

    // -- error Display --

    #[test]
    fn error_messages_are_user_facing() {
        // The error variants are rendered for users via the `:`-line.
        // Sanity-check the messages.
        assert_eq!(ParseError::Empty.to_string(), "empty command");
        assert_eq!(
            ParseError::Unknown("foo".to_string()).to_string(),
            "unknown command: foo"
        );
        assert_eq!(
            ParseError::MissingArg("open").to_string(),
            "missing argument for :open"
        );
        assert_eq!(
            ParseError::UnexpectedArg("quit").to_string(),
            "unexpected argument for :quit"
        );
    }

    // -- complete_info / info metadata --

    #[test]
    fn complete_info_empty_returns_all_in_order() {
        let names: Vec<&str> = complete_info("").iter().map(|i| i.name).collect();
        assert_eq!(names, vec!["open", "file", "stats", "quit", "help"]);
    }

    #[test]
    fn complete_info_just_colon_returns_all() {
        let names: Vec<&str> = complete_info(":").iter().map(|i| i.name).collect();
        assert_eq!(names, vec!["open", "file", "stats", "quit", "help"]);
    }

    #[test]
    fn complete_info_prefix_filters() {
        let infos = complete_info(":s");
        assert_eq!(infos.len(), 1);
        assert_eq!(infos[0].name, "stats");
        assert!(infos[0].available);
        assert_eq!(infos[0].arg, "");
    }

    #[test]
    fn complete_info_past_space_is_empty() {
        assert_eq!(complete_info(":open ~/"), Vec::<CommandInfo>::new());
    }

    #[test]
    fn complete_info_marks_unavailable() {
        assert_eq!(info("open").unwrap().available, false);
        assert_eq!(info("file").unwrap().available, false);
        assert_eq!(info("stats").unwrap().available, true);
        assert_eq!(info("quit").unwrap().available, true);
        assert_eq!(info("help").unwrap().available, true);
    }

    #[test]
    fn info_lookup_round_trips() {
        let stats = info("stats").expect("stats is a known command");
        assert_eq!(stats.name, "stats");
        assert!(stats.available);
        assert_eq!(info("nope"), None);
    }

    #[test]
    fn command_info_covers_every_command_with_nonempty_descriptions() {
        // Every command has metadata, and no description is blank.
        assert_eq!(COMMAND_INFO.len(), COMMANDS.len());
        for cmd in COMMANDS {
            let i = info(cmd).expect("every COMMANDS entry has metadata");
            assert!(!i.description.is_empty(), "{cmd} has an empty description");
        }
    }

    #[test]
    fn command_info_matches_commands_order() {
        // The guard that the two tables never drift: same names, same order.
        let info_names: Vec<&str> = COMMAND_INFO.iter().map(|i| i.name).collect();
        assert_eq!(info_names, COMMANDS.to_vec());
    }

    #[test]
    fn command_info_arg_shape_matches_parse() {
        // Ties metadata to parser truth: arg-shaped commands need an argument,
        // no-arg commands parse bare.
        for i in COMMAND_INFO {
            let bare = format!(":{}", i.name);
            if i.arg.is_empty() {
                assert!(
                    parse(&bare).is_ok(),
                    "{} has no arg but failed to parse bare",
                    i.name
                );
            } else {
                assert_eq!(
                    parse(&bare),
                    Err(ParseError::MissingArg(i.name)),
                    "{} has an arg shape but parsed bare",
                    i.name
                );
            }
        }
    }
}
