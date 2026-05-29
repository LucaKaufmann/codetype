//! Round-robin selector across the Repo Tour's eligible Exercise pool.
//!
//! Maintains per-file visit counts and declaration indices. `next()` returns
//! the next Exercise from the file with the lowest visit count; ties break
//! on the file's position in the internal `Vec`. In production, that
//! position is randomized at construction so each fresh tour starts on a
//! different file. For deterministic tests, use [`RepoTour::new_in_order`].

use std::path::PathBuf;

use rand::seq::SliceRandom;

use crate::exercise::Exercise;

#[derive(Debug)]
pub struct RepoTour {
    // `Vec` instead of `BTreeMap` so we can shuffle. With BTreeMap the
    // iteration order was always alphabetical, which made the very first
    // exercise the same on every launch.
    files: Vec<(PathBuf, FileState)>,
}

#[derive(Debug)]
struct FileState {
    // Invariant: never empty. The constructor filters out empty vecs, so
    // the modulo arithmetic in `next()` cannot divide by zero.
    exercises: Vec<Exercise>,
    next_index: usize,
    visited: usize,
}

impl RepoTour {
    /// Build a tour from a mapping of file path → Exercises. The file order
    /// is randomized so each fresh tour starts on a different file. Within
    /// the tour, round-robin still visits every file once per cycle, so
    /// you don't see the same file twice before seeing all the others.
    pub fn new(per_file: impl IntoIterator<Item = (PathBuf, Vec<Exercise>)>) -> Self {
        let mut tour = Self::new_in_order(per_file);
        tour.files.shuffle(&mut rand::rng());
        tour
    }

    /// Build a tour preserving the iteration order of the input. Useful for
    /// tests that need a deterministic sequence; production code should
    /// prefer [`Self::new`] for the randomized starting file.
    pub fn new_in_order(per_file: impl IntoIterator<Item = (PathBuf, Vec<Exercise>)>) -> Self {
        let files: Vec<_> = per_file
            .into_iter()
            .filter(|(_, exs)| !exs.is_empty())
            .map(|(path, exercises)| {
                (
                    path,
                    FileState {
                        exercises,
                        next_index: 0,
                        visited: 0,
                    },
                )
            })
            .collect();
        Self { files }
    }

    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    /// Return the next Exercise according to the round-robin policy.
    /// `None` only if the pool is empty; otherwise the tour cycles forever.
    // Named `next` deliberately as the tour's pull API; it isn't an `Iterator`
    // (it never ends), so the `Iterator::next` confusion lint doesn't apply.
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> Option<Exercise> {
        // Pick the file with the lowest `visited` count. `min_by_key`
        // returns the first such file in `self.files` iteration order;
        // since we shuffle on construction, that's a random tiebreak.
        let idx = self
            .files
            .iter()
            .enumerate()
            .min_by_key(|(_, (_, state))| state.visited)
            .map(|(i, _)| i)?;
        let (_, state) = &mut self.files[idx];
        let exercise = state.exercises[state.next_index].clone();
        state.next_index = (state.next_index + 1) % state.exercises.len();
        state.visited += 1;
        Some(exercise)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exercise::IndentUnit;
    use pretty_assertions::assert_eq;

    fn make(text: &str) -> Exercise {
        Exercise {
            source_path: PathBuf::from("ignored.swift"),
            byte_range: 0..text.len(),
            text: text.to_string(),
            indent_unit: IndentUnit::Spaces(4),
        }
    }

    fn take_texts(tour: &mut RepoTour, n: usize) -> Vec<String> {
        (0..n).map(|_| tour.next().unwrap().text).collect()
    }

    #[test]
    fn empty_pool_returns_none() {
        let mut tour = RepoTour::new_in_order([]);
        assert!(tour.is_empty());
        assert!(tour.next().is_none());
    }

    #[test]
    fn files_with_no_exercises_are_filtered_out() {
        let mut tour = RepoTour::new_in_order(vec![(PathBuf::from("empty.swift"), Vec::new())]);
        assert!(tour.is_empty());
        assert!(tour.next().is_none());
    }

    #[test]
    fn single_exercise_keeps_returning_itself() {
        let mut tour = RepoTour::new_in_order(vec![(PathBuf::from("A.swift"), vec![make("a0")])]);
        assert_eq!(take_texts(&mut tour, 3), vec!["a0", "a0", "a0"]);
    }

    #[test]
    fn single_file_walks_declarations_in_order_then_wraps() {
        let mut tour = RepoTour::new_in_order(vec![(
            PathBuf::from("A.swift"),
            vec![make("a0"), make("a1"), make("a2")],
        )]);
        assert_eq!(take_texts(&mut tour, 5), vec!["a0", "a1", "a2", "a0", "a1"]);
    }

    #[test]
    fn two_files_alternate() {
        let mut tour = RepoTour::new_in_order(vec![
            (PathBuf::from("A.swift"), vec![make("a0")]),
            (PathBuf::from("B.swift"), vec![make("b0")]),
        ]);
        assert_eq!(take_texts(&mut tour, 4), vec!["a0", "b0", "a0", "b0"]);
    }

    #[test]
    fn uneven_file_sizes_no_starvation() {
        let mut tour = RepoTour::new_in_order(vec![
            (PathBuf::from("A.swift"), vec![make("a0")]),
            (
                PathBuf::from("B.swift"),
                vec![make("b0"), make("b1"), make("b2")],
            ),
        ]);
        assert_eq!(
            take_texts(&mut tour, 6),
            vec!["a0", "b0", "a0", "b1", "a0", "b2"]
        );
    }

    #[test]
    fn three_files_distribute_evenly() {
        let mut tour = RepoTour::new_in_order(vec![
            (PathBuf::from("A.swift"), vec![make("a0")]),
            (PathBuf::from("B.swift"), vec![make("b0")]),
            (PathBuf::from("C.swift"), vec![make("c0")]),
        ]);
        assert_eq!(
            take_texts(&mut tour, 9),
            vec!["a0", "b0", "c0", "a0", "b0", "c0", "a0", "b0", "c0"]
        );
    }

    #[test]
    fn randomized_new_starts_on_varied_files() {
        // Statistical test: with 5 files and 50 fresh tours, we should see
        // at least 2 distinct starting files. Probability of all 50 picking
        // the same file is (1/5)^49 ≈ 1.4e-35 — astronomically unlikely.
        let files: Vec<_> = (0..5)
            .map(|i| {
                (
                    PathBuf::from(format!("f{i}.swift")),
                    vec![make(&format!("e{i}"))],
                )
            })
            .collect();
        let mut seen_starts = std::collections::HashSet::new();
        for _ in 0..50 {
            let mut tour = RepoTour::new(files.clone());
            seen_starts.insert(tour.next().unwrap().text);
        }
        assert!(
            seen_starts.len() >= 2,
            "expected the random shuffle to pick varied starting files; saw only {:?}",
            seen_starts
        );
    }

    #[test]
    fn randomized_new_still_visits_every_file_in_one_cycle() {
        // The shuffle randomizes the starting file but the round-robin
        // policy must still visit every file at least once before any
        // file gets visited twice.
        let files: Vec<_> = (0..4)
            .map(|i| {
                (
                    PathBuf::from(format!("f{i}.swift")),
                    vec![make(&format!("e{i}"))],
                )
            })
            .collect();
        let mut tour = RepoTour::new(files);
        let first_cycle: std::collections::HashSet<String> =
            (0..4).map(|_| tour.next().unwrap().text).collect();
        assert_eq!(first_cycle.len(), 4, "expected each file visited once");
    }
}
