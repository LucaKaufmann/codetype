//! The typing state machine.
//!
//! Pure logic: feed it keystrokes via [`TypingEngine::on_char`] /
//! [`TypingEngine::on_backspace`] / [`TypingEngine::on_enter`] and read its
//! state. No I/O, no rendering.
//!
//! Supports two modes:
//! - [`Mode::Strict`] — wrong keys do not advance the cursor (the default)
//! - [`Mode::Lenient`] — wrong keys advance the cursor and mark the
//!   position as errored; the user can backspace to fix (Monkeytype-style)

use crate::exercise::Exercise;

/// Per-position recording of what the user last typed at that position.
/// Used in both modes — in [`Mode::Lenient`] it drives `status_at` directly;
/// in [`Mode::Strict`] it's kept consistent but unused for display.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PositionStatus {
    Untyped,
    Correct,
    Wrong,
}

/// How the engine treats wrong keystrokes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    /// Wrong keys do not advance the cursor. The user must type the
    /// expected character to make progress. The default.
    #[default]
    Strict,
    /// Wrong keys advance the cursor anyway and mark the position as
    /// errored. The user can backspace and retype to fix. Matches the
    /// Monkeytype "advance through errors" feel.
    Lenient,
}

/// Per-character status of each position in the Exercise text.
///
/// This is purely a *display* concept — the engine itself does not store a
/// `Vec<CharStatus>`. It is computed on demand from the cursor and "stuck"
/// flag (see [`TypingEngine::status_at`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CharStatus {
    /// The cursor has not yet reached this position.
    Pending,
    /// The cursor has passed this position. (The position may still appear
    /// in the error log if a wrong key was pressed here before the user
    /// eventually typed the correct one.)
    Correct,
    /// The cursor is *currently* at this position and the most recent
    /// keystroke here was wrong.
    Errored,
}

/// A single wrong-key event recorded at the position the user was expected
/// to type `expected` but typed `typed`.
///
/// Error events are sticky: once logged they remain in the engine's log for
/// the life of the Exercise — even if the user backspaces and retypes the
/// position correctly. This is the data feed for per-character stats.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ErrorEvent {
    pub position: usize,
    pub typed: char,
    pub expected: char,
}

#[derive(Debug)]
pub struct TypingEngine {
    exercise: Exercise,
    chars: Vec<char>,
    is_auto_indent: Vec<bool>,
    cursor: usize,
    mode: Mode,
    // Per-position last-attempt status. In Strict mode this is maintained
    // but `status_at` doesn't consult it (it uses cursor + the stuck flag).
    // In Lenient mode it IS the source of truth for `status_at`.
    position_status: Vec<PositionStatus>,
    // Strict-mode "stuck on a wrong key" flag. In Lenient mode this stays
    // false because the cursor always advances.
    last_keystroke_wrong: bool,
    errors: Vec<ErrorEvent>,
}

impl TypingEngine {
    pub fn new(exercise: Exercise) -> Self {
        Self::new_with_mode(exercise, Mode::Strict)
    }

    pub fn new_with_mode(exercise: Exercise, mode: Mode) -> Self {
        let chars: Vec<char> = exercise.text.chars().collect();
        let is_auto_indent = compute_auto_indent_mask(&chars);
        let position_status = vec![PositionStatus::Untyped; chars.len()];
        Self {
            exercise,
            chars,
            is_auto_indent,
            cursor: 0,
            mode,
            position_status,
            last_keystroke_wrong: false,
            errors: Vec::new(),
        }
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    pub fn exercise(&self) -> &Exercise {
        &self.exercise
    }

    /// The text of the Exercise as a slice of chars, in position order.
    pub fn chars(&self) -> &[char] {
        &self.chars
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// All error events recorded so far, in the order they happened.
    pub fn errors(&self) -> &[ErrorEvent] {
        &self.errors
    }

    pub fn is_complete(&self) -> bool {
        self.cursor == self.chars.len()
    }

    /// Display status of the position at `i`. In Strict mode, derived from
    /// the cursor + stuck flag; in Lenient mode, from the per-position log.
    pub fn status_at(&self, i: usize) -> CharStatus {
        match self.mode {
            Mode::Strict => {
                if i < self.cursor {
                    CharStatus::Correct
                } else if i == self.cursor && self.last_keystroke_wrong {
                    CharStatus::Errored
                } else {
                    CharStatus::Pending
                }
            }
            Mode::Lenient => match self.position_status.get(i).copied() {
                Some(PositionStatus::Correct) => CharStatus::Correct,
                Some(PositionStatus::Wrong) => CharStatus::Errored,
                _ => CharStatus::Pending,
            },
        }
    }

    /// Number of characters the user has typed correctly that *count* — i.e.
    /// excludes positions that were skipped past as auto-indent. In Lenient
    /// mode also excludes positions left in a Wrong state.
    pub fn correct_chars_typed(&self) -> usize {
        (0..self.cursor)
            .filter(|&i| !self.is_auto_indent[i])
            .filter(|&i| match self.mode {
                Mode::Strict => true,
                Mode::Lenient => self.position_status[i] == PositionStatus::Correct,
            })
            .count()
    }

    pub fn on_char(&mut self, ch: char) {
        if self.is_complete() {
            return;
        }
        let expected = self.chars[self.cursor];
        if ch == expected {
            self.position_status[self.cursor] = PositionStatus::Correct;
            self.last_keystroke_wrong = false;
            self.cursor += 1;
        } else {
            self.errors.push(ErrorEvent {
                position: self.cursor,
                typed: ch,
                expected,
            });
            self.position_status[self.cursor] = PositionStatus::Wrong;
            match self.mode {
                Mode::Strict => {
                    self.last_keystroke_wrong = true;
                }
                Mode::Lenient => {
                    let was_newline = expected == '\n';
                    self.cursor += 1;
                    self.last_keystroke_wrong = false;
                    // If the user wrong-keyed at a newline position, still
                    // skip the auto-indent block that followed it — they
                    // shouldn't have to type leading whitespace they would
                    // never normally type.
                    if was_newline {
                        self.skip_auto_indent();
                    }
                }
            }
        }
    }

    pub fn on_backspace(&mut self) {
        if self.is_complete() {
            return;
        }
        // Strict mode: if we're stuck on a wrong key, backspace clears the
        // stuck flag but does NOT decrement past the previous (correctly
        // typed) character. This is the fix for the "backspace eats good
        // progress" feedback.
        if self.mode == Mode::Strict && self.last_keystroke_wrong {
            self.last_keystroke_wrong = false;
            self.position_status[self.cursor] = PositionStatus::Untyped;
            return;
        }
        self.last_keystroke_wrong = false;
        if self.cursor > 0 {
            self.cursor -= 1;
            self.position_status[self.cursor] = PositionStatus::Untyped;
        }
    }

    pub fn on_enter(&mut self) {
        if self.is_complete() {
            return;
        }
        let expected = self.chars[self.cursor];
        if expected == '\n' {
            self.position_status[self.cursor] = PositionStatus::Correct;
            self.last_keystroke_wrong = false;
            self.cursor += 1;
            self.skip_auto_indent();
        } else {
            self.errors.push(ErrorEvent {
                position: self.cursor,
                typed: '\n',
                expected,
            });
            self.position_status[self.cursor] = PositionStatus::Wrong;
            match self.mode {
                Mode::Strict => {
                    self.last_keystroke_wrong = true;
                }
                Mode::Lenient => {
                    self.cursor += 1;
                    self.last_keystroke_wrong = false;
                }
            }
        }
    }

    /// Advance the cursor through any contiguous run of auto-indent
    /// positions, marking each as `Correct` (the user "earned" them by
    /// reaching them, even though they didn't type them).
    fn skip_auto_indent(&mut self) {
        while self.cursor < self.chars.len() && self.is_auto_indent[self.cursor] {
            self.position_status[self.cursor] = PositionStatus::Correct;
            self.cursor += 1;
        }
    }
}

/// Compute the auto-indent mask for an Exercise's chars.
///
/// A position is auto-indent iff it is ' ' or '\t' AND its predecessor is
/// either '\n' or itself auto-indent. The first position is never auto-indent
/// (no predecessor to trigger the run).
fn compute_auto_indent_mask(chars: &[char]) -> Vec<bool> {
    let mut mask = vec![false; chars.len()];
    let mut in_indent_run = false;
    for (i, &ch) in chars.iter().enumerate() {
        if ch == '\n' {
            in_indent_run = true;
        } else if in_indent_run && (ch == ' ' || ch == '\t') {
            mask[i] = true;
        } else {
            in_indent_run = false;
        }
    }
    mask
}

// `#[cfg(test)]` means: only compile this module when running `cargo test`.
// It doesn't ship in the release binary. The `mod tests` is a nested private
// module — the convention for unit tests that live next to the code they
// test.
#[cfg(test)]
mod tests {
    // `use super::*` brings everything from the parent module (engine.rs)
    // into scope. Tests need access to the private `compute_auto_indent_mask`
    // function as well as the public API — `super::*` gives us both because
    // tests are siblings of the rest of the module's items.
    use super::*;
    use crate::exercise::IndentUnit;
    // Shadow the stdlib `assert_eq!` with pretty_assertions's version, which
    // prints colored diffs on failure. This line is a per-module override —
    // outside this module, `assert_eq!` is still the stdlib macro.
    use pretty_assertions::assert_eq;
    use std::path::PathBuf;

    fn make_exercise(text: &str) -> Exercise {
        Exercise {
            source_path: PathBuf::from("test.swift"),
            byte_range: 0..text.len(),
            text: text.to_string(),
            indent_unit: IndentUnit::Spaces(4),
        }
    }

    #[test]
    fn new_exercise_starts_at_cursor_zero() {
        let engine = TypingEngine::new(make_exercise("abc"));
        assert_eq!(engine.cursor(), 0);
        assert!(!engine.is_complete());
        assert!(engine.errors().is_empty());
        assert_eq!(engine.status_at(0), CharStatus::Pending);
    }

    #[test]
    fn correct_char_advances_cursor() {
        let mut engine = TypingEngine::new(make_exercise("abc"));
        engine.on_char('a');
        assert_eq!(engine.cursor(), 1);
        assert!(engine.errors().is_empty());
        assert_eq!(engine.status_at(0), CharStatus::Correct);
        assert_eq!(engine.status_at(1), CharStatus::Pending);
    }

    #[test]
    fn wrong_char_does_not_advance_cursor() {
        let mut engine = TypingEngine::new(make_exercise("abc"));
        engine.on_char('x');
        assert_eq!(engine.cursor(), 0);
        assert_eq!(engine.status_at(0), CharStatus::Errored);
    }

    #[test]
    fn wrong_char_logs_error_event() {
        let mut engine = TypingEngine::new(make_exercise("abc"));
        engine.on_char('x');
        assert_eq!(
            engine.errors(),
            &[ErrorEvent {
                position: 0,
                typed: 'x',
                expected: 'a',
            }]
        );
    }

    #[test]
    fn wrong_then_correct_advances_past() {
        let mut engine = TypingEngine::new(make_exercise("abc"));
        engine.on_char('x');
        engine.on_char('a');
        assert_eq!(engine.cursor(), 1);
        assert_eq!(engine.errors().len(), 1);
        // Display status at position 0 is Correct (the cursor has passed
        // it), even though the error log remembers the earlier mistake.
        assert_eq!(engine.status_at(0), CharStatus::Correct);
    }

    #[test]
    fn multiple_wrong_keystrokes_log_separately() {
        let mut engine = TypingEngine::new(make_exercise("abc"));
        engine.on_char('x');
        engine.on_char('y');
        engine.on_char('z');
        assert_eq!(engine.errors().len(), 3);
        assert_eq!(engine.cursor(), 0);
    }

    #[test]
    fn backspace_decrements_cursor() {
        let mut engine = TypingEngine::new(make_exercise("abc"));
        engine.on_char('a');
        engine.on_backspace();
        assert_eq!(engine.cursor(), 0);
        assert_eq!(engine.status_at(0), CharStatus::Pending);
    }

    #[test]
    fn backspace_at_zero_is_noop() {
        let mut engine = TypingEngine::new(make_exercise("abc"));
        engine.on_backspace();
        assert_eq!(engine.cursor(), 0);
    }

    #[test]
    fn backspace_does_not_erase_errors() {
        let mut engine = TypingEngine::new(make_exercise("abc"));
        engine.on_char('x');
        engine.on_char('a');
        engine.on_backspace();
        assert_eq!(engine.cursor(), 0);
        assert_eq!(engine.errors().len(), 1);
    }

    #[test]
    fn backspace_clears_stuck_state() {
        let mut engine = TypingEngine::new(make_exercise("abc"));
        engine.on_char('x');
        assert_eq!(engine.status_at(0), CharStatus::Errored);
        engine.on_backspace();
        assert_eq!(engine.status_at(0), CharStatus::Pending);
    }

    #[test]
    fn enter_on_newline_skips_auto_indent() {
        let mut engine = TypingEngine::new(make_exercise("a\n    b"));
        engine.on_char('a');
        engine.on_enter();
        // Cursor should land directly on 'b' (position 6), having skipped
        // past the '\n' (position 1) and four spaces (positions 2..6).
        assert_eq!(engine.cursor(), 6);
    }

    #[test]
    fn enter_on_non_newline_logs_error() {
        let mut engine = TypingEngine::new(make_exercise("abc"));
        engine.on_enter();
        assert_eq!(engine.cursor(), 0);
        assert_eq!(
            engine.errors(),
            &[ErrorEvent {
                position: 0,
                typed: '\n',
                expected: 'a',
            }]
        );
    }

    #[test]
    fn correct_chars_typed_excludes_auto_indent() {
        let mut engine = TypingEngine::new(make_exercise("a\n    b"));
        engine.on_char('a');
        engine.on_enter();
        engine.on_char('b');
        // The user passes through 7 positions: 'a', '\n', four spaces, 'b'.
        // The four spaces are auto-indent and excluded. 'a', '\n', 'b' count.
        assert_eq!(engine.correct_chars_typed(), 3);
    }

    #[test]
    fn is_complete_when_cursor_reaches_end() {
        let mut engine = TypingEngine::new(make_exercise("ab"));
        engine.on_char('a');
        engine.on_char('b');
        assert!(engine.is_complete());
    }

    #[test]
    fn completed_engine_ignores_further_input() {
        let mut engine = TypingEngine::new(make_exercise("a"));
        engine.on_char('a');
        assert!(engine.is_complete());
        engine.on_char('x');
        engine.on_enter();
        engine.on_backspace();
        assert!(engine.errors().is_empty());
        assert_eq!(engine.cursor(), 1);
    }

    // -- Strict-mode backspace improvement ----------------------------------

    #[test]
    fn strict_backspace_when_stuck_does_not_eat_previous_correct() {
        // Previously, backspace decremented unconditionally — typing `a`,
        // then a wrong key, then backspace would undo the `a`. New behavior:
        // backspace while stuck just clears the stuck flag without moving.
        let mut engine = TypingEngine::new(make_exercise("abc"));
        engine.on_char('a'); // correct, cursor → 1
        engine.on_char('x'); // wrong, cursor stays at 1, stuck
        engine.on_backspace();
        assert_eq!(engine.cursor(), 1);
        // The previously-correct char at position 0 still shows Correct.
        assert_eq!(engine.status_at(0), CharStatus::Correct);
        // The cursor position has been "reset" — no longer stuck.
        assert_eq!(engine.status_at(1), CharStatus::Pending);
    }

    // -- Lenient mode -------------------------------------------------------

    fn lenient(text: &str) -> TypingEngine {
        TypingEngine::new_with_mode(make_exercise(text), Mode::Lenient)
    }

    #[test]
    fn lenient_wrong_char_advances_cursor() {
        let mut engine = lenient("abc");
        engine.on_char('x'); // wrong
        assert_eq!(engine.cursor(), 1);
        assert_eq!(engine.status_at(0), CharStatus::Errored);
    }

    #[test]
    fn lenient_wrong_char_logs_error() {
        let mut engine = lenient("abc");
        engine.on_char('x');
        assert_eq!(
            engine.errors(),
            &[ErrorEvent {
                position: 0,
                typed: 'x',
                expected: 'a',
            }]
        );
    }

    #[test]
    fn lenient_can_complete_with_errors() {
        let mut engine = lenient("abc");
        engine.on_char('x'); // wrong
        engine.on_char('b'); // correct
        engine.on_char('c'); // correct
        assert!(engine.is_complete());
        // Position 0 stayed Wrong since user never went back to fix it.
        assert_eq!(engine.status_at(0), CharStatus::Errored);
        assert_eq!(engine.status_at(1), CharStatus::Correct);
        assert_eq!(engine.status_at(2), CharStatus::Correct);
    }

    #[test]
    fn lenient_correct_chars_typed_excludes_wrong_positions() {
        let mut engine = lenient("abc");
        engine.on_char('x'); // wrong @ pos 0
        engine.on_char('b'); // correct @ pos 1
        engine.on_char('c'); // correct @ pos 2
        // Two positions correctly typed; pos 0 is still Wrong.
        assert_eq!(engine.correct_chars_typed(), 2);
    }

    #[test]
    fn lenient_backspace_then_retype_fixes_position() {
        let mut engine = lenient("abc");
        engine.on_char('x'); // wrong @ pos 0, cursor → 1
        engine.on_backspace(); // cursor → 0, pos 0 reset
        assert_eq!(engine.cursor(), 0);
        assert_eq!(engine.status_at(0), CharStatus::Pending);
        engine.on_char('a'); // correct now
        assert_eq!(engine.status_at(0), CharStatus::Correct);
        // The error event from the original wrong attempt is still logged.
        assert_eq!(engine.errors().len(), 1);
    }

    #[test]
    fn lenient_wrong_at_newline_skips_auto_indent() {
        // Same shape as the strict test, but the user "presses" a letter
        // instead of Enter. In lenient mode the cursor still advances past
        // the newline AND past the auto-indent so the user isn't trapped
        // trying to type leading whitespace.
        let mut engine = lenient("a\n    b");
        engine.on_char('a'); // correct
        engine.on_char('x'); // wrong at the newline position
        assert_eq!(engine.cursor(), 6); // landed on 'b'
        assert_eq!(engine.status_at(1), CharStatus::Errored);
    }

    #[test]
    fn lenient_completed_engine_ignores_further_input() {
        let mut engine = lenient("a");
        engine.on_char('a');
        assert!(engine.is_complete());
        let errors_before = engine.errors().len();
        engine.on_char('x');
        engine.on_enter();
        engine.on_backspace();
        assert_eq!(engine.errors().len(), errors_before);
    }

    // -- The original auto-indent helper still has its own test ------------

    #[test]
    fn auto_indent_mask_basic() {
        let chars: Vec<char> = "a\n    b\n  c".chars().collect();
        let mask = compute_auto_indent_mask(&chars);
        // Positions: 'a'  '\n'  ' '   ' '   ' '   ' '  'b'  '\n'  ' '   ' '  'c'
        // Index:      0    1     2     3     4     5    6     7    8     9    10
        // Auto:       F    F     T     T     T     T    F     F    T     T    F
        assert_eq!(
            mask,
            vec![
                false, false, true, true, true, true, false, false, true, true, false
            ]
        );
    }
}
