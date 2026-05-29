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
}
