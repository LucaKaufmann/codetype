//! Accumulates typing events into per-Exercise and per-Session statistics.
//!
//! The collector is a **pure fold**: it does not maintain in-progress
//! exercise state, does not own clocks, and does not stream events. Each
//! call to [`StatsCollector::finish_exercise`] takes the complete error
//! list from a finished Exercise (the engine already holds it), folds it
//! into the running session totals, and returns the per-Exercise stats
//! for the stats screen.
//!
//! This shape keeps the unit of work obvious — one finish per Exercise —
//! and makes tests trivially deterministic.

use std::collections::HashMap;
use std::time::Duration;

use crate::engine::ErrorEvent;

const TOP_MISTAKES_LIMIT: usize = 3;
const CHARS_PER_WORD: f64 = 5.0;

#[derive(Debug, Default)]
pub struct StatsCollector {
    session: SessionStats,
    // Cumulative across the whole Session. Keyed by the *expected* char —
    // the one the user was supposed to type — so the v2 heatmap surfaces
    // "you struggle with `=>`," not "you mistakenly typed `e`."
    error_counts: HashMap<char, usize>,
}

#[derive(Debug, Default, Clone)]
pub struct SessionStats {
    pub exercises_completed: usize,
    pub total_correct_chars: usize,
    pub total_errors: usize,
    pub total_time: Duration,
}

#[derive(Debug, Clone)]
pub struct ExerciseStats {
    pub wpm: f64,
    pub cpm: f64,
    /// Fraction in `[0.0, 1.0]`.
    pub accuracy: f64,
    pub elapsed: Duration,
    pub correct_chars: usize,
    pub error_count: usize,
    /// Up to [`TOP_MISTAKES_LIMIT`] most-mistyped *expected* characters in
    /// this Exercise, sorted by count descending. Ties broken by character
    /// ascending so the ordering is deterministic.
    pub top_mistakes: Vec<(char, usize)>,
}

impl StatsCollector {
    pub fn new() -> Self {
        Self::default()
    }

    /// Fold one finished Exercise into the running totals and return its
    /// per-Exercise stats for display.
    ///
    /// - `errors`: every wrong keystroke logged by the engine during this
    ///   Exercise. Includes keystrokes that were later corrected.
    /// - `correct_chars`: positions the user typed correctly that *count*
    ///   (auto-indented whitespace is already excluded by the engine —
    ///   see [`crate::engine::TypingEngine::correct_chars_typed`]).
    /// - `elapsed`: the wall-clock duration the Exercise took. The app
    ///   layer owns the timer.
    pub fn finish_exercise(
        &mut self,
        errors: &[ErrorEvent],
        correct_chars: usize,
        elapsed: Duration,
    ) -> ExerciseStats {
        let error_count = errors.len();
        let stats = ExerciseStats {
            wpm: wpm(correct_chars, elapsed),
            cpm: cpm(correct_chars, elapsed),
            accuracy: accuracy(correct_chars, error_count),
            elapsed,
            correct_chars,
            error_count,
            top_mistakes: top_mistakes(errors, TOP_MISTAKES_LIMIT),
        };

        // Fold into session totals.
        self.session.exercises_completed += 1;
        self.session.total_correct_chars += correct_chars;
        self.session.total_errors += error_count;
        self.session.total_time += elapsed;

        // Accumulate cumulative per-character error counts for the v2 heatmap.
        for e in errors {
            *self.error_counts.entry(e.expected).or_insert(0) += 1;
        }

        stats
    }

    pub fn session(&self) -> &SessionStats {
        &self.session
    }

    pub fn error_counts(&self) -> &HashMap<char, usize> {
        &self.error_counts
    }
}

// -- Pure helpers ------------------------------------------------------------
// Each of these is a small free function, easier to read and test in isolation
// than as inline arithmetic inside `finish_exercise`.

fn cpm(correct_chars: usize, elapsed: Duration) -> f64 {
    let minutes = elapsed.as_secs_f64() / 60.0;
    if minutes == 0.0 {
        0.0
    } else {
        correct_chars as f64 / minutes
    }
}

fn wpm(correct_chars: usize, elapsed: Duration) -> f64 {
    cpm(correct_chars, elapsed) / CHARS_PER_WORD
}

fn accuracy(correct_chars: usize, error_count: usize) -> f64 {
    let total = correct_chars + error_count;
    if total == 0 {
        // No keystrokes attempted — say 100%. This shouldn't happen in
        // practice (the engine requires reaching the end of the Exercise),
        // but the API is defensive.
        1.0
    } else {
        correct_chars as f64 / total as f64
    }
}

fn top_mistakes(errors: &[ErrorEvent], limit: usize) -> Vec<(char, usize)> {
    let mut counts: HashMap<char, usize> = HashMap::new();
    for e in errors {
        *counts.entry(e.expected).or_insert(0) += 1;
    }
    let mut sorted: Vec<(char, usize)> = counts.into_iter().collect();
    // Sort by count descending; on ties, by char ascending. The second
    // criterion makes the output deterministic regardless of HashMap
    // iteration order (which is randomized).
    sorted.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    sorted.truncate(limit);
    sorted
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn err(expected: char, typed: char) -> ErrorEvent {
        ErrorEvent {
            position: 0,
            typed,
            expected,
        }
    }

    fn secs(n: u64) -> Duration {
        Duration::from_secs(n)
    }

    // -- WPM / CPM --

    #[test]
    fn cpm_is_correct_chars_per_minute() {
        // 60 correct chars in 60 seconds = 60 cpm.
        assert_eq!(cpm(60, secs(60)), 60.0);
        // 300 correct chars in 60 seconds = 300 cpm.
        assert_eq!(cpm(300, secs(60)), 300.0);
    }

    #[test]
    fn wpm_is_cpm_divided_by_five() {
        // 60 cpm → 12 wpm (the "5 chars per word" convention).
        assert_eq!(wpm(60, secs(60)), 12.0);
        // 300 cpm → 60 wpm.
        assert_eq!(wpm(300, secs(60)), 60.0);
    }

    #[test]
    fn wpm_with_zero_duration_is_zero() {
        // Edge case: defensive return rather than panicking on div-by-zero.
        assert_eq!(wpm(100, Duration::ZERO), 0.0);
        assert_eq!(cpm(100, Duration::ZERO), 0.0);
    }

    // -- Accuracy --

    #[test]
    fn accuracy_with_no_errors_is_one() {
        assert_eq!(accuracy(50, 0), 1.0);
    }

    #[test]
    fn accuracy_with_mix_of_correct_and_errors() {
        // 40 correct, 10 errors → 40 / 50 = 0.8
        assert_eq!(accuracy(40, 10), 0.8);
    }

    #[test]
    fn accuracy_with_no_keystrokes_is_one() {
        // Edge case: zero attempts → no failures, count as 100%.
        assert_eq!(accuracy(0, 0), 1.0);
    }

    // -- Top mistakes --

    #[test]
    fn top_mistakes_empty_input_is_empty() {
        assert_eq!(top_mistakes(&[], 3), Vec::<(char, usize)>::new());
    }

    #[test]
    fn top_mistakes_sorted_by_count_descending() {
        let errors = vec![
            err('=', 'x'),
            err('=', 'x'),
            err('=', 'x'),
            err('{', 'x'),
            err('{', 'x'),
            err(';', 'x'),
        ];
        assert_eq!(top_mistakes(&errors, 3), vec![('=', 3), ('{', 2), (';', 1)]);
    }

    #[test]
    fn top_mistakes_ties_broken_alphabetically() {
        let errors = vec![
            err('z', 'x'),
            err('z', 'x'),
            err('a', 'x'),
            err('a', 'x'),
            err('m', 'x'),
            err('m', 'x'),
        ];
        // All three appear twice. Tiebreak by char ascending: a, m, z.
        assert_eq!(top_mistakes(&errors, 3), vec![('a', 2), ('m', 2), ('z', 2)]);
    }

    #[test]
    fn top_mistakes_limited_to_n() {
        let errors = vec![err('a', 'x'), err('b', 'x'), err('c', 'x'), err('d', 'x')];
        assert_eq!(top_mistakes(&errors, 2).len(), 2);
    }

    // -- finish_exercise / session folding --

    #[test]
    fn new_collector_has_empty_session() {
        let collector = StatsCollector::new();
        assert_eq!(collector.session().exercises_completed, 0);
        assert_eq!(collector.session().total_correct_chars, 0);
        assert_eq!(collector.session().total_errors, 0);
        assert_eq!(collector.session().total_time, Duration::ZERO);
        assert!(collector.error_counts().is_empty());
    }

    #[test]
    fn finish_exercise_returns_filled_stats() {
        let mut collector = StatsCollector::new();
        let errors = vec![err('=', 'x'), err('=', 'x'), err('{', 'x')];
        let stats = collector.finish_exercise(&errors, 100, secs(60));
        assert_eq!(stats.correct_chars, 100);
        assert_eq!(stats.error_count, 3);
        assert_eq!(stats.elapsed, secs(60));
        assert_eq!(stats.cpm, 100.0);
        assert_eq!(stats.wpm, 20.0);
        // 100 / (100 + 3) ≈ 0.9708
        assert!((stats.accuracy - 100.0 / 103.0).abs() < 1e-9);
        assert_eq!(stats.top_mistakes, vec![('=', 2), ('{', 1)]);
    }

    #[test]
    fn session_totals_accumulate_across_exercises() {
        let mut collector = StatsCollector::new();
        collector.finish_exercise(&[err('=', 'x')], 50, secs(30));
        collector.finish_exercise(&[err('{', 'x'), err('{', 'x')], 70, secs(45));
        let s = collector.session();
        assert_eq!(s.exercises_completed, 2);
        assert_eq!(s.total_correct_chars, 120);
        assert_eq!(s.total_errors, 3);
        assert_eq!(s.total_time, secs(75));
    }

    #[test]
    fn error_counts_accumulate_across_exercises() {
        let mut collector = StatsCollector::new();
        collector.finish_exercise(&[err('=', 'x'), err('{', 'x')], 10, secs(10));
        collector.finish_exercise(&[err('=', 'x'), err('=', 'x')], 10, secs(10));
        let counts = collector.error_counts();
        assert_eq!(counts.get(&'='), Some(&3));
        assert_eq!(counts.get(&'{'), Some(&1));
    }

    #[test]
    fn empty_exercise_produces_clean_stats() {
        // A pathological zero-length exercise (filtered out in practice,
        // but the API must not panic).
        let mut collector = StatsCollector::new();
        let stats = collector.finish_exercise(&[], 0, Duration::ZERO);
        assert_eq!(stats.correct_chars, 0);
        assert_eq!(stats.error_count, 0);
        assert_eq!(stats.wpm, 0.0);
        assert_eq!(stats.cpm, 0.0);
        assert_eq!(stats.accuracy, 1.0);
        assert!(stats.top_mistakes.is_empty());
    }
}
