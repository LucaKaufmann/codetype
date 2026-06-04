//! Persistent, checked-in typing scores: the `CODETYPE.md` leaderboard.
//!
//! A session's stats normally die with the process. This module persists them
//! into a Markdown table at the root of the drilled repo so a team can commit
//! scores to git and compete. The design notes (intentionally not committed)
//! live in `docs/design/persistent-scores.md`; the short version:
//!
//! - **One file, `CODETYPE.md`** — renders as a real table on the GitHub repo
//!   page, which is the whole reason a team would adopt this.
//! - **The table *is* the source of truth.** We parse it back defensively; a
//!   hand-edited row we can't understand is preserved verbatim, never dropped.
//! - **Stable row order** (sorted by player, then language) so each person only
//!   ever rewrites their own rows — concurrent edits auto-merge in git.
//! - **Per-language rows plus an `overall` rollup** per player.
//!
//! The split mirrors [`crate::stats`]: [`Leaderboard::parse`] and
//! [`Leaderboard::render`] are pure string functions (the testable core),
//! while [`Leaderboard::load`]/[`Leaderboard::save`] are the thin IO shell.

use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Context;

use crate::stats::SessionStats;

/// The rollup row aggregating a player's sessions across every language.
const OVERALL: &str = "overall";

/// Header + alignment rows of the leaderboard table. Right-aligned numeric
/// columns (`---:`) so GitHub renders the numbers flush right.
const HEADER: &str =
    "| Player | Lang | Best WPM | Avg WPM | Accuracy | Sessions | Last played |";
const SEPARATOR: &str = "| --- | --- | ---: | ---: | ---: | ---: | --- |";

/// Default text written above the table on a fresh file. Preserved verbatim on
/// every rewrite, and explicit that scores are honor-system (plain text, no
/// tamper-proofing — see the design's non-goals).
const DEFAULT_PREAMBLE: &str = "# CodeType Leaderboard\n\n\
     <!-- Managed by CodeType. Rows are sorted by player, then language, and the\n\
     \x20    tool rewrites only your own rows. Scores are honor-system: they're\n\
     \x20    plain text and editable. -->";

/// Who the current run is attributed to. For v1 the player's display *name* is
/// the identity in the file (the visible table can't carry a hidden email key
/// without showing it), so two people sharing a git `user.name` would share a
/// row — acceptable for a small team, and `--as` lets anyone pick a handle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    pub name: String,
}

/// Resolve the player identity for `repo`: an explicit `override_name` wins,
/// else git's `user.name`, else `user.email`. `None` means "can't tell who you
/// are" — the caller suppresses the submit prompt rather than guessing.
pub fn resolve_identity(repo: &Path, override_name: Option<&str>) -> Option<Identity> {
    let raw = match override_name {
        Some(name) if !name.trim().is_empty() => name.to_string(),
        _ => git_config(repo, "user.name").or_else(|| git_config(repo, "user.email"))?,
    };
    let name = sanitize_name(&raw);
    (!name.is_empty()).then_some(Identity { name })
}

/// Strip characters that would corrupt the Markdown table: the `|` column
/// delimiter and any control characters (newlines, tabs). Internal whitespace
/// is collapsed and the result trimmed, so the mapping is stable — the same
/// person always resolves to the same row key.
fn sanitize_name(raw: &str) -> String {
    let cleaned: String = raw
        .chars()
        .map(|c| if c == '|' || c.is_control() { ' ' } else { c })
        .collect();
    cleaned.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Read a single git config value as seen from `repo` (merged global + local).
/// Any failure — git missing, key unset, empty value — collapses to `None`.
fn git_config(repo: &Path, key: &str) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["config", "--get", key])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let value = String::from_utf8(output.stdout).ok()?.trim().to_string();
    (!value.is_empty()).then_some(value)
}

/// One player's stats for one language (or the `overall` rollup). Averages are
/// display-ready values updated by a running formula (see
/// [`Leaderboard::record_session`]); we deliberately don't keep the raw totals,
/// trading a fraction-of-a-WPM rounding drift for a file a human wants to read.
#[derive(Debug, Clone, PartialEq)]
pub struct Record {
    pub best_wpm: f64,
    pub avg_wpm: f64,
    /// Fraction in `[0.0, 1.0]`; rendered as a percentage.
    pub accuracy: f64,
    pub sessions: u32,
    /// ISO date `YYYY-MM-DD`.
    pub last_played: String,
}

/// The whole `CODETYPE.md`: the prose above the table plus identity-keyed rows.
/// A [`BTreeMap`] keyed by `(player, lang)` gives the stable sort order for
/// free, so writes touch only the current player's rows.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Leaderboard {
    /// Text above the table, preserved verbatim across rewrites.
    preamble: String,
    rows: BTreeMap<(String, String), Record>,
    /// Table rows we parsed but couldn't understand (hand-edited / corrupt).
    /// Re-emitted untouched so a stray edit never costs someone their data.
    opaque_rows: Vec<String>,
}

/// The result of recording a session — everything the terminal "you placed
/// #N" line needs. Returned by [`Leaderboard::record_session`].
#[derive(Debug, Clone, PartialEq)]
pub struct SessionPlacement {
    pub session_wpm: f64,
    pub language: String,
    /// 1-based rank among all players by `overall` best WPM.
    pub rank: usize,
    pub total: usize,
    /// True if this session set a new personal best for the language.
    pub new_best: bool,
}

impl SessionPlacement {
    /// One-line summary for stdout after the session ends.
    pub fn message(&self) -> String {
        let best = if self.new_best {
            "  🏆 new personal best!"
        } else {
            ""
        };
        format!(
            "Submitted {:.0} WPM in {} — you're #{} of {} on the board.{}",
            self.session_wpm, self.language, self.rank, self.total, best
        )
    }
}

impl Leaderboard {
    /// Load `path`, or an empty board (with the default preamble) if it's
    /// missing. Other IO errors propagate — the caller decides whether a
    /// stats-file problem should be fatal.
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(content) => Ok(Self::parse(&content)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::empty()),
            Err(e) => Err(e).with_context(|| format!("read {}", path.display())),
        }
    }

    /// Write the rendered board to `path`.
    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        // Refuse to write through a symlink. The target is always
        // `<repo>/CODETYPE.md`, so a symlink is the only way this write could
        // escape the repo — an untrusted repo could plant one pointing at an
        // arbitrary user-writable file to be clobbered when the user submits.
        if let Ok(meta) = std::fs::symlink_metadata(path)
            && meta.file_type().is_symlink()
        {
            anyhow::bail!(
                "{} is a symlink; refusing to write scores through it",
                path.display()
            );
        }
        std::fs::write(path, self.render()).with_context(|| format!("write {}", path.display()))
    }

    fn empty() -> Self {
        Self {
            preamble: DEFAULT_PREAMBLE.to_string(),
            rows: BTreeMap::new(),
            opaque_rows: Vec::new(),
        }
    }

    /// Parse a `CODETYPE.md` body. Everything before the table header is kept
    /// as the preamble; if no table is found the whole document becomes the
    /// preamble (so a hand-written file is preserved, with our table appended
    /// on the next save). Unparseable table rows land in `opaque_rows`.
    pub fn parse(content: &str) -> Self {
        let lines: Vec<&str> = content.lines().collect();
        let Some(header_idx) = lines.iter().position(|l| is_header_row(l)) else {
            // No table yet — treat the entire file as preamble.
            let preamble = content.trim_end().to_string();
            return Self {
                preamble: if preamble.is_empty() {
                    DEFAULT_PREAMBLE.to_string()
                } else {
                    preamble
                },
                rows: BTreeMap::new(),
                opaque_rows: Vec::new(),
            };
        };

        let preamble = lines[..header_idx].join("\n").trim_end().to_string();
        let mut rows = BTreeMap::new();
        let mut opaque_rows = Vec::new();

        // Data rows start after the header and its `---` separator. Stop at the
        // first non-table line (the tool owns the file from the header down, so
        // we don't expect trailing prose, but we won't loop past it either).
        for line in lines.iter().skip(header_idx + 1) {
            let trimmed = line.trim();
            if !trimmed.starts_with('|') {
                break;
            }
            if is_separator_row(trimmed) {
                continue;
            }
            match parse_row(trimmed) {
                Some(((player, lang), record)) => {
                    rows.insert((player, lang), record);
                }
                None => opaque_rows.push(trimmed.to_string()),
            }
        }

        Self {
            preamble: if preamble.is_empty() {
                DEFAULT_PREAMBLE.to_string()
            } else {
                preamble
            },
            rows,
            opaque_rows,
        }
    }

    /// Render the board back to a full `CODETYPE.md` body. Deterministic: rows
    /// come out in `BTreeMap` (stable) order, so `render` is idempotent through
    /// a `parse` round-trip.
    pub fn render(&self) -> String {
        let mut out = String::with_capacity(256);
        out.push_str(self.preamble.trim_end());
        out.push_str("\n\n");
        out.push_str(HEADER);
        out.push('\n');
        out.push_str(SEPARATOR);
        out.push('\n');
        for ((player, lang), r) in &self.rows {
            out.push_str(&render_row(player, lang, r));
            out.push('\n');
        }
        for raw in &self.opaque_rows {
            out.push_str(raw);
            out.push('\n');
        }
        out
    }

    /// Fold a finished session for `player` into the board: update the matching
    /// language row and the player's `overall` rollup, then report where they
    /// landed. `today` is injected (ISO `YYYY-MM-DD`) so the core stays pure.
    pub fn record_session(
        &mut self,
        player: &str,
        language: &str,
        session: &SessionStats,
        today: &str,
    ) -> SessionPlacement {
        let wpm = session.wpm();
        let accuracy = session.accuracy();

        let new_best = self.upsert(player, language, wpm, accuracy, today);
        // The rollup gets the same session; it averages across all languages.
        self.upsert(player, OVERALL, wpm, accuracy, today);

        let (rank, total) = self.rank_of(player);
        SessionPlacement {
            session_wpm: wpm,
            language: language.to_string(),
            rank,
            total,
            new_best,
        }
    }

    /// Update (or insert) one `(player, lang)` row with a session result.
    /// Returns whether this session set a new best for that row.
    fn upsert(&mut self, player: &str, lang: &str, wpm: f64, accuracy: f64, today: &str) -> bool {
        let record = self
            .rows
            .entry((player.to_string(), lang.to_string()))
            .or_insert_with(|| Record {
                best_wpm: 0.0,
                avg_wpm: 0.0,
                accuracy: 0.0,
                sessions: 0,
                last_played: String::new(),
            });

        let new_best = wpm > record.best_wpm;
        // Session-count-weighted running mean: new = (old*n + x) / (n+1).
        let n = record.sessions as f64;
        record.avg_wpm = (record.avg_wpm * n + wpm) / (n + 1.0);
        record.accuracy = (record.accuracy * n + accuracy) / (n + 1.0);
        record.best_wpm = record.best_wpm.max(wpm);
        record.sessions += 1;
        record.last_played = today.to_string();
        new_best
    }

    /// Rank `player` (1-based) among all players by `overall` best WPM,
    /// descending; ties broken by name so the ordering is deterministic.
    /// Returns `(rank, total_players)`.
    fn rank_of(&self, player: &str) -> (usize, usize) {
        let mut overalls: Vec<(&str, f64)> = self
            .rows
            .iter()
            .filter(|((_, lang), _)| lang == OVERALL)
            .map(|((p, _), r)| (p.as_str(), r.best_wpm))
            .collect();
        overalls.sort_by(|a, b| {
            b.1.partial_cmp(&a.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.0.cmp(b.0))
        });
        let total = overalls.len();
        let rank = overalls
            .iter()
            .position(|(p, _)| *p == player)
            .map_or(total, |i| i + 1);
        (rank, total)
    }

    #[cfg(test)]
    fn record(&self, player: &str, lang: &str) -> Option<&Record> {
        self.rows.get(&(player.to_string(), lang.to_string()))
    }
}

// -- Row parsing / rendering -------------------------------------------------

/// True if `line` is the table's header row (first cell is `Player`).
fn is_header_row(line: &str) -> bool {
    split_row(line).first().map(String::as_str) == Some("Player")
}

/// True if `line` is a Markdown alignment separator (`| --- | ---: | ... |`).
fn is_separator_row(line: &str) -> bool {
    let cells = split_row(line);
    !cells.is_empty()
        && cells
            .iter()
            .all(|c| !c.is_empty() && c.chars().all(|ch| ch == '-' || ch == ':'))
}

/// Split a `| a | b | c |` row into trimmed cell strings, dropping the empty
/// edges produced by the leading/trailing pipes.
fn split_row(line: &str) -> Vec<String> {
    let trimmed = line.trim();
    // The leading/trailing pipes are both optional in GitHub-flavored Markdown.
    // Strip them independently — note the suffix step falls back to the
    // *prefix-stripped* string, not the original, so a row without a trailing
    // pipe doesn't re-introduce a phantom leading empty cell.
    let without_prefix = trimmed.strip_prefix('|').unwrap_or(trimmed);
    let inner = without_prefix.strip_suffix('|').unwrap_or(without_prefix);
    inner.split('|').map(|c| c.trim().to_string()).collect()
}

/// Parse one data row into a keyed [`Record`], or `None` if any cell is
/// malformed (the caller keeps the raw line instead of dropping it).
fn parse_row(line: &str) -> Option<((String, String), Record)> {
    let cells = split_row(line);
    let [player, lang, best, avg, acc, sessions, last] = cells.as_slice() else {
        return None;
    };
    if player.is_empty() || lang.is_empty() {
        return None;
    }
    let record = Record {
        best_wpm: best.parse().ok()?,
        avg_wpm: avg.parse().ok()?,
        accuracy: acc.strip_suffix('%')?.trim().parse::<f64>().ok()? / 100.0,
        sessions: sessions.parse().ok()?,
        last_played: last.clone(),
    };
    Some(((player.clone(), lang.clone()), record))
}

fn render_row(player: &str, lang: &str, r: &Record) -> String {
    format!(
        "| {} | {} | {:.1} | {:.1} | {:.1}% | {} | {} |",
        player,
        lang,
        r.best_wpm,
        r.avg_wpm,
        r.accuracy * 100.0,
        r.sessions,
        r.last_played,
    )
}

// -- Today's date ------------------------------------------------------------

/// Today's date as ISO `YYYY-MM-DD`, in UTC. Dependency-free: we convert the
/// Unix day count with the civil-calendar algorithm below rather than pull in a
/// date crate for one string.
pub fn today() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let (y, m, d) = civil_from_days((secs / 86_400) as i64);
    format!("{y:04}-{m:02}-{d:02}")
}

/// Convert a count of days since the Unix epoch into `(year, month, day)`.
/// Howard Hinnant's `civil_from_days` — exact for the full proleptic Gregorian
/// calendar, no lookup tables.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    (if month <= 2 { year + 1 } else { year }, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use std::time::Duration;

    /// Build a `SessionStats` with a chosen WPM and accuracy. WPM is
    /// `(correct_chars / 5) / minutes`; we fix a 60s session so `correct_chars
    /// = wpm * 5`, then pick errors to hit the target accuracy.
    fn session(wpm: f64, accuracy: f64) -> SessionStats {
        let correct = (wpm * 5.0).round() as usize;
        // accuracy = correct / (correct + errors)  =>  errors = correct/acc - correct
        let errors = ((correct as f64 / accuracy) - correct as f64).round() as usize;
        SessionStats {
            exercises_completed: 1,
            total_correct_chars: correct,
            total_errors: errors,
            total_time: Duration::from_secs(60),
        }
    }

    // -- civil_from_days --

    #[test]
    fn civil_from_days_known_dates() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(365), (1971, 1, 1));
        assert_eq!(civil_from_days(730), (1972, 1, 1)); // 1972 is a leap year
        assert_eq!(civil_from_days(19723), (2024, 1, 1));
    }

    // -- parse / render --

    #[test]
    fn empty_file_is_empty_board_with_default_preamble() {
        let board = Leaderboard::parse("");
        assert!(board.rows.is_empty());
        assert!(board.preamble.contains("CodeType Leaderboard"));
    }

    #[test]
    fn render_then_parse_round_trips() {
        let mut board = Leaderboard::empty();
        board.record_session("Luca", "rust", &session(71.0, 0.95), "2026-06-04");
        board.record_session("Sam", "swift", &session(82.0, 0.97), "2026-06-03");

        let rendered = board.render();
        let reparsed = Leaderboard::parse(&rendered);
        // Rendering is idempotent through a parse: the file is a stable fixed
        // point. (We can't assert `reparsed == board` because rendering rounds
        // the running averages to one decimal — the file, not the in-memory
        // float, is the source of truth.)
        assert_eq!(reparsed.render(), rendered);
        assert_eq!(reparsed.record("Luca", "rust").unwrap().sessions, 1);
        assert_eq!(reparsed.record("Sam", "swift").unwrap().best_wpm, 82.0);
    }

    #[test]
    fn rows_render_in_stable_sorted_order() {
        let mut board = Leaderboard::empty();
        // Insert out of order; expect sorted (player, then lang with `overall`
        // first since 'o' < 'r'/'s').
        board.record_session("Sam", "rust", &session(90.0, 0.9), "2026-06-04");
        board.record_session("Ada", "swift", &session(70.0, 0.9), "2026-06-04");
        let rendered = board.render();
        let ada = rendered.find("| Ada |").unwrap();
        let sam = rendered.find("| Sam |").unwrap();
        assert!(ada < sam, "Ada should sort before Sam");
        // `overall` row precedes the language row for the same player.
        let sam_overall = rendered.find("| Sam | overall |").unwrap();
        let sam_rust = rendered.find("| Sam | rust |").unwrap();
        assert!(sam_overall < sam_rust);
    }

    #[test]
    fn custom_preamble_is_preserved() {
        let doc = "# Our Team Board\n\nType fast or perish.\n\n\
            | Player | Lang | Best WPM | Avg WPM | Accuracy | Sessions | Last played |\n\
            | --- | --- | ---: | ---: | ---: | ---: | --- |\n\
            | Luca | rust | 71.0 | 71.0 | 95.0% | 1 | 2026-06-04 |\n";
        let board = Leaderboard::parse(doc);
        assert!(board.render().contains("Type fast or perish."));
    }

    #[test]
    fn parses_rows_without_a_trailing_pipe() {
        // GitHub-flavored Markdown allows omitting the closing pipe; a
        // formatter or hand-edit can rewrite the file this way. We must still
        // detect the header and parse the rows (not append a second table).
        let doc = "# CodeType Leaderboard\n\n\
            | Player | Lang | Best WPM | Avg WPM | Accuracy | Sessions | Last played\n\
            | --- | --- | ---: | ---: | ---: | ---: | ---\n\
            | Luca | rust | 71.0 | 71.0 | 95.0% | 1 | 2026-06-04\n";
        let board = Leaderboard::parse(doc);
        assert!(board.opaque_rows.is_empty());
        assert_eq!(board.record("Luca", "rust").unwrap().sessions, 1);
    }

    #[test]
    fn malformed_row_is_preserved_not_dropped() {
        let doc = "# CodeType Leaderboard\n\n\
            | Player | Lang | Best WPM | Avg WPM | Accuracy | Sessions | Last played |\n\
            | --- | --- | ---: | ---: | ---: | ---: | --- |\n\
            | Luca | rust | 71.0 | 71.0 | 95.0% | 1 | 2026-06-04 |\n\
            | Mallory | rust | not-a-number | hax | 999 | x | yesterday |\n";
        let board = Leaderboard::parse(doc);
        assert_eq!(board.opaque_rows.len(), 1);
        assert!(board.record("Luca", "rust").is_some());
        // The junk survives a round-trip.
        assert!(board.render().contains("Mallory"));
    }

    // -- record_session math --

    #[test]
    fn first_session_seeds_the_record() {
        let mut board = Leaderboard::empty();
        let placement = board.record_session("Luca", "rust", &session(60.0, 0.96), "2026-06-04");
        let rec = board.record("Luca", "rust").unwrap();
        assert_eq!(rec.sessions, 1);
        assert!((rec.best_wpm - 60.0).abs() < 1e-9);
        assert!((rec.avg_wpm - 60.0).abs() < 1e-9);
        assert_eq!(rec.last_played, "2026-06-04");
        assert!(placement.new_best);
        assert_eq!(placement.rank, 1);
        assert_eq!(placement.total, 1);
    }

    #[test]
    fn second_session_updates_running_average_and_best() {
        let mut board = Leaderboard::empty();
        board.record_session("Luca", "rust", &session(60.0, 1.0), "2026-06-04");
        let placement = board.record_session("Luca", "rust", &session(80.0, 1.0), "2026-06-05");
        let rec = board.record("Luca", "rust").unwrap();
        assert_eq!(rec.sessions, 2);
        assert!((rec.best_wpm - 80.0).abs() < 1e-9);
        assert!((rec.avg_wpm - 70.0).abs() < 1e-9); // mean of 60 and 80
        assert!(placement.new_best); // 80 > 60
        assert_eq!(rec.last_played, "2026-06-05");
    }

    #[test]
    fn slower_session_is_not_a_new_best() {
        let mut board = Leaderboard::empty();
        board.record_session("Luca", "rust", &session(80.0, 1.0), "2026-06-04");
        let placement = board.record_session("Luca", "rust", &session(60.0, 1.0), "2026-06-05");
        assert!(!placement.new_best);
        assert!((board.record("Luca", "rust").unwrap().best_wpm - 80.0).abs() < 1e-9);
    }

    #[test]
    fn overall_rollup_aggregates_across_languages() {
        let mut board = Leaderboard::empty();
        board.record_session("Luca", "rust", &session(60.0, 1.0), "2026-06-04");
        board.record_session("Luca", "swift", &session(80.0, 1.0), "2026-06-05");
        let overall = board.record("Luca", OVERALL).unwrap();
        assert_eq!(overall.sessions, 2);
        assert!((overall.avg_wpm - 70.0).abs() < 1e-9);
        assert!((overall.best_wpm - 80.0).abs() < 1e-9);
    }

    // -- merge safety: recording one player never touches another's row --

    #[test]
    fn recording_one_player_leaves_others_byte_identical() {
        let mut board = Leaderboard::empty();
        board.record_session("Ada", "rust", &session(70.0, 0.9), "2026-06-04");
        let ada_before = board.record("Ada", "rust").unwrap().clone();

        board.record_session("Sam", "rust", &session(90.0, 0.9), "2026-06-05");
        let ada_after = board.record("Ada", "rust").unwrap();
        assert_eq!(&ada_before, ada_after);
    }

    // -- ranking --

    #[test]
    fn rank_orders_players_by_overall_best() {
        let mut board = Leaderboard::empty();
        board.record_session("Ada", "rust", &session(70.0, 0.9), "2026-06-04");
        let sam = board.record_session("Sam", "rust", &session(90.0, 0.9), "2026-06-04");
        // Sam (90) ranks above Ada (70).
        assert_eq!(sam.rank, 1);
        assert_eq!(sam.total, 2);
        let ada = board.record_session("Ada", "swift", &session(50.0, 0.9), "2026-06-05");
        assert_eq!(ada.rank, 2);
    }

    // -- identity sanitizing --

    #[test]
    fn sanitize_name_strips_table_breakers() {
        assert_eq!(sanitize_name("a|b"), "a b"); // pipe would add a phantom cell
        assert_eq!(sanitize_name("Luca\n"), "Luca"); // newline would split the row
        assert_eq!(sanitize_name("  Ada  Lovelace  "), "Ada Lovelace");
    }

    #[test]
    fn identity_override_is_sanitized() {
        // A `|` in the handle can't leak into the table and corrupt the row.
        let id = resolve_identity(Path::new("."), Some("ev|l")).unwrap();
        assert_eq!(id.name, "ev l");
    }

    #[test]
    fn pipe_safe_name_round_trips_through_the_table() {
        let mut board = Leaderboard::empty();
        let name = sanitize_name("a|b");
        board.record_session(&name, "rust", &session(70.0, 0.95), "2026-06-04");
        let reparsed = Leaderboard::parse(&board.render());
        // The row survives as a real (parsed) row, not an opaque one.
        assert!(reparsed.opaque_rows.is_empty());
        assert_eq!(reparsed.record(&name, "rust").unwrap().sessions, 1);
    }

    // -- IO: save / load --

    #[test]
    fn save_then_load_round_trips_a_regular_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("CODETYPE.md");
        let mut board = Leaderboard::empty();
        board.record_session("Luca", "rust", &session(70.0, 0.95), "2026-06-04");
        board.save(&path).unwrap();
        let loaded = Leaderboard::load(&path).unwrap();
        assert_eq!(loaded.record("Luca", "rust").unwrap().sessions, 1);
    }

    #[cfg(unix)]
    #[test]
    fn save_refuses_to_write_through_a_symlink() {
        use std::os::unix::fs::symlink;
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("secret.txt");
        std::fs::write(&target, "do not touch").unwrap();
        let link = dir.path().join("CODETYPE.md");
        symlink(&target, &link).unwrap();

        let board = Leaderboard::empty();
        assert!(board.save(&link).is_err());
        // The symlink target is left untouched.
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "do not touch");
    }

    // -- placement message --

    #[test]
    fn placement_message_mentions_rank_and_best() {
        let p = SessionPlacement {
            session_wpm: 72.4,
            language: "rust".to_string(),
            rank: 2,
            total: 5,
            new_best: true,
        };
        let msg = p.message();
        assert!(msg.contains("72 WPM in rust"));
        assert!(msg.contains("#2 of 5"));
        assert!(msg.contains("personal best"));
    }
}
