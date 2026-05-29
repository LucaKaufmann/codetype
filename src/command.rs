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

impl Command {
    /// The bare command name (matches the `name` in [`COMMAND_INFO`]). Lets
    /// callers look a parsed command up in its metadata — e.g. to check
    /// [`CommandInfo::available`].
    pub fn name(&self) -> &'static str {
        match self {
            Command::Open(_) => "open",
            Command::File(_) => "file",
            Command::Stats => "stats",
            Command::Quit => "quit",
            Command::Help => "help",
        }
    }
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

/// Parse a command-palette line into a [`Command`].
///
/// Accepts input with or without a leading `:`. Surrounding whitespace is
/// trimmed; internal whitespace in arguments is preserved. Whether a command
/// takes an argument is derived from its [`COMMAND_INFO`] entry (the single
/// source of truth), so the parser and the palette's arg hints can't disagree.
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

    let info = COMMAND_INFO
        .iter()
        .find(|i| i.name == cmd)
        .ok_or_else(|| ParseError::Unknown(cmd.to_string()))?;

    // Validate the argument against the metadata's `arg` shape (non-empty =>
    // an argument is required).
    let arg = if info.arg.is_empty() {
        if !rest.is_empty() {
            return Err(ParseError::UnexpectedArg(info.name));
        }
        None
    } else if rest.is_empty() {
        return Err(ParseError::MissingArg(info.name));
    } else {
        Some(rest.to_string())
    };

    // The one inherently per-variant step: build the typed `Command`. `cmd`
    // matched a `COMMAND_INFO` entry above, so the catch-all is unreachable.
    Ok(match info.name {
        "open" => Command::Open(arg.expect("open requires an arg per its metadata")),
        "file" => Command::File(arg.expect("file requires an arg per its metadata")),
        "stats" => Command::Stats,
        "quit" => Command::Quit,
        "help" => Command::Help,
        other => unreachable!("COMMAND_INFO entry {other:?} has no Command constructor"),
    })
}

/// Return the command names that match a partial input, in display order.
/// Thin wrapper over [`complete_info`] for callers that only need names.
pub fn complete(input: &str) -> Vec<&'static str> {
    complete_info(input).into_iter().map(|i| i.name).collect()
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

/// Metadata for every command, in display order. **This is the single source
/// of truth for the command vocabulary**: `parse` derives names + arg-shapes
/// from it, `complete`/`complete_info` filter it, and `app::execute_command`
/// reads `available` to decide what's wired up. `open`/`file` are
/// `available: false` (not implemented yet) and the palette marks them
/// "(not yet)"; `stats`/`quit`/`help` are `true`.
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

/// Metadata for the commands whose name matches `input`'s prefix, in display
/// order — the primitive the palette renders. `complete_info(":")` and
/// `complete_info("")` return every command; once the input contains a space
/// (the user has moved into argument territory) it returns empty.
pub fn complete_info(input: &str) -> Vec<CommandInfo> {
    let body = input.trim_start().trim_start_matches(':');
    if body.contains(' ') {
        return Vec::new();
    }
    COMMAND_INFO
        .iter()
        .copied()
        .filter(|info| info.name.starts_with(body))
        .collect()
}

/// Look up a single command's metadata by name (`None` if unknown).
pub fn info(name: &str) -> Option<CommandInfo> {
    COMMAND_INFO.iter().copied().find(|i| i.name == name)
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
    fn command_info_descriptions_are_nonempty() {
        // The single source of truth: every entry has a usable description.
        for i in COMMAND_INFO {
            assert!(!i.description.is_empty(), "{} has an empty description", i.name);
        }
    }

    #[test]
    fn every_command_info_name_parses_and_round_trips() {
        // Couples the metadata table to the `Command` enum: each entry must
        // name a real, parseable command whose `name()` maps back to it. Adding
        // a COMMAND_INFO entry without a matching Command constructor fails here.
        for i in COMMAND_INFO {
            let input = if i.arg.is_empty() {
                format!(":{}", i.name)
            } else {
                format!(":{} x", i.name)
            };
            let cmd = parse(&input).expect("COMMAND_INFO entry should parse");
            assert_eq!(cmd.name(), i.name);
        }
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
