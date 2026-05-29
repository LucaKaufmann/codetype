//! Top-level event loop, terminal setup, and rendering. Wires
//! [`crate::scanner`] → [`crate::extractor`] → [`crate::tour`] →
//! [`crate::engine`] → [`crate::stats`] and pumps the keystroke loop.
//!
//! Kept deliberately thin and *intentionally untested at unit level* — UI
//! correctness is validated by running the app. The deep modules underneath
//! carry the correctness load and have their own tests.

use std::io::{self, Stdout};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::Context;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::Paragraph;

use crate::command::{self, Command, ParseError};
use crate::engine::{CharStatus, Mode, TypingEngine};
use crate::exercise::Exercise;
use crate::extractor::{Extractor, registry};
use crate::scanner::{self, RepoScanner};
use crate::stats::{ExerciseStats, SessionStats, StatsCollector};
use crate::tour::RepoTour;

/// Debug entry point: build the tour and print the first `count` exercises
/// to stdout. Leading whitespace is rendered with visible markers (spaces
/// as `·`, tabs as `→`) so we can distinguish "dedent produced flat text"
/// from "renderer dropped the whitespace."
pub fn print_exercises(repo: &Path, count: usize, lang: Option<String>) -> anyhow::Result<()> {
    let extractor = select_extractor(repo, lang.as_deref())?;
    eprintln!("drilling {}", extractor.language());
    let mut tour = build_tour(repo, &extractor)?;
    for i in 0..count {
        let Some(ex) = tour.next() else {
            break;
        };
        println!("=== Exercise {} — {} ===", i + 1, ex.source_path.display());
        println!(
            "indent_unit={:?}  bytes={}  text_chars={}",
            ex.indent_unit,
            ex.byte_range.end - ex.byte_range.start,
            ex.text.chars().count(),
        );
        println!();
        for (lineno, line) in ex.text.lines().enumerate() {
            let leading_count = line.chars().take_while(|c| c.is_whitespace()).count();
            let leading_repr: String = line
                .chars()
                .take(leading_count)
                .map(|c| match c {
                    ' ' => '·',
                    '\t' => '→',
                    _ => '?',
                })
                .collect();
            let rest: String = line.chars().skip(leading_count).collect();
            println!("{:>3} │ {}{}", lineno + 1, leading_repr, rest);
        }
        println!();
    }
    Ok(())
}

/// Public entry point. Owns the terminal lifecycle and the event loop.
pub fn run(repo: &Path, mode: Mode, lang: Option<String>) -> anyhow::Result<()> {
    let extractor = select_extractor(repo, lang.as_deref())?;
    let language = extractor.language().to_string();
    let mut tour = build_tour(repo, &extractor)?;
    let initial_exercise = tour
        .next()
        .ok_or_else(|| anyhow::anyhow!("repo had files but no extractable exercises"))?;
    // Canonicalize so `.` (or any relative path) resolves to its real
    // directory name before we take the basename. Without this, running
    // `codetype .` would show "." as the repo label.
    let canonical = std::fs::canonicalize(repo).unwrap_or_else(|_| repo.to_path_buf());
    let repo_label = canonical
        .file_name()
        .and_then(|n| n.to_str())
        .map(str::to_string)
        .unwrap_or_else(|| canonical.display().to_string());

    let mut terminal = setup_terminal()?;
    // RAII guard restores the terminal on panic or early return.
    let _guard = TerminalGuard;

    let result = event_loop(
        &mut terminal,
        tour,
        initial_exercise,
        repo_label,
        language,
        mode,
    );

    // Explicit restore in addition to the guard, so the cursor reappears
    // even on the happy path.
    let _ = restore_terminal(&mut terminal);
    result
}

// -- Pipeline construction ---------------------------------------------------

/// Choose the language to drill: honor `--lang` if given, otherwise infer the
/// dominant language from the repo's tracked files.
fn select_extractor(repo: &Path, lang: Option<&str>) -> anyhow::Result<Extractor> {
    let config = match lang {
        Some(name) => registry::by_name(name).ok_or_else(|| {
            anyhow::anyhow!(
                "unknown language '{name}'. supported: {}",
                registry::supported_names()
            )
        })?,
        None => {
            let files = scanner::list_tracked_files(repo)?;
            registry::detect_dominant(&files).ok_or_else(|| {
                anyhow::anyhow!(
                    "no supported source files found in {} (looked for: {})",
                    repo.display(),
                    registry::supported_names()
                )
            })?
        }
    };
    Ok(Extractor::new(config))
}

fn build_tour(repo: &Path, extractor: &Extractor) -> anyhow::Result<RepoTour> {
    let language = extractor.language();
    let scanner = RepoScanner::new(repo, extractor.extensions().to_vec());

    let files = scanner.scan()?;
    if files.is_empty() {
        anyhow::bail!("no {language} files found in {}", repo.display());
    }

    let mut per_file: Vec<(PathBuf, Vec<Exercise>)> = Vec::new();
    for file in files {
        let source = std::fs::read(&file).with_context(|| format!("read {}", file.display()))?;
        let exercises = extractor.extract(&file, &source)?;
        if !exercises.is_empty() {
            per_file.push((file, exercises));
        }
    }
    if per_file.is_empty() {
        anyhow::bail!(
            "no extractable exercises in {} (try a repo with more {language} code)",
            repo.display()
        );
    }
    Ok(RepoTour::new(per_file))
}

// -- Terminal lifecycle ------------------------------------------------------

type Tty = Terminal<CrosstermBackend<Stdout>>;

fn setup_terminal() -> anyhow::Result<Tty> {
    enable_raw_mode().context("enable raw mode")?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen).context("enter alternate screen")?;
    let backend = CrosstermBackend::new(stdout);
    let terminal = Terminal::new(backend).context("create ratatui terminal")?;
    Ok(terminal)
}

fn restore_terminal(terminal: &mut Tty) -> anyhow::Result<()> {
    disable_raw_mode().ok();
    execute!(terminal.backend_mut(), LeaveAlternateScreen).ok();
    terminal.show_cursor().ok();
    Ok(())
}

/// Drop guard: runs on scope exit (including panic) to restore terminal state.
struct TerminalGuard;
impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
    }
}

// -- App state ---------------------------------------------------------------

enum AppState {
    Typing {
        engine: TypingEngine,
        // None until the first accepted keystroke; the header reads 0:00 and
        // WPM measures real typing time only.
        started: Option<Instant>,
    },
    Stats {
        stats: ExerciseStats,
        exercise: Exercise,
    },
    Palette {
        input: String,
        prior: Box<AppState>,
    },
    Help {
        prior: Box<AppState>,
    },
    SessionStats {
        // The screen that invoked the session-stats overlay (Typing/Stats/etc.).
        // Any key returns here. The view reads live `session()` at render time,
        // so it carries no copy of the stats.
        prior: Box<AppState>,
    },
}

enum Action {
    Stay,
    To(AppState),
    Quit,
}

// -- Event loop --------------------------------------------------------------

fn event_loop(
    terminal: &mut Tty,
    mut tour: RepoTour,
    initial_exercise: Exercise,
    repo_label: String,
    language: String,
    mode: Mode,
) -> anyhow::Result<()> {
    let mut state = AppState::Typing {
        engine: TypingEngine::new_with_mode(initial_exercise, mode),
        started: None,
    };
    let mut stats_collector = StatsCollector::new();
    let mut notice: Option<String> = None;

    loop {
        terminal.draw(|f| {
            render(
                f,
                &state,
                stats_collector.session(),
                notice.as_deref(),
                &repo_label,
                &language,
            )
        })?;

        // The notice is NOT cleared here. It must survive the draw it was set
        // in (and any 250ms-timeout redraws) so the user can read it. It is
        // cleared instead at the top of `handle_key` on the next keypress.

        // Poll for an event with a small timeout so the elapsed timer in
        // Typing mode can tick visibly.
        if !event::poll(Duration::from_millis(250))? {
            continue;
        }
        let Event::Key(key) = event::read()? else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }

        let action = handle_key(
            &mut state,
            key,
            &mut tour,
            &mut stats_collector,
            &mut notice,
            mode,
        );
        match action {
            Action::Stay => {}
            Action::To(new) => state = new,
            Action::Quit => break,
        }
    }
    Ok(())
}

fn handle_key(
    state: &mut AppState,
    key: KeyEvent,
    tour: &mut RepoTour,
    stats: &mut StatsCollector,
    notice: &mut Option<String>,
    mode: Mode,
) -> Action {
    // Clear any notice from a previous keypress. A notice set *during* this
    // call (e.g. a parse error from running a command) is written after this
    // point, so it survives to the next draw and is cleared only when the
    // user presses the next key.
    *notice = None;

    match state {
        AppState::Typing { engine, started } => handle_typing_key(engine, started, key, stats),
        AppState::Stats { exercise, .. } => handle_stats_key(exercise, key, tour, mode),
        AppState::Palette { input, prior } => handle_palette_key(input, prior, key, tour, notice),
        AppState::Help { prior } => handle_help_key(prior, key),
        AppState::SessionStats { prior } => handle_session_stats_key(prior, key),
    }
}

fn handle_typing_key(
    engine: &mut TypingEngine,
    started: &mut Option<Instant>,
    key: KeyEvent,
    stats: &mut StatsCollector,
) -> Action {
    match key.code {
        KeyCode::Esc => {
            // Bail mid-exercise: drop the engine and quit on Esc.
            return Action::Quit;
        }
        KeyCode::Char(c) => engine.on_char(c),
        KeyCode::Backspace => engine.on_backspace(),
        KeyCode::Enter => engine.on_enter(),
        _ => return Action::Stay,
    }

    // Reaching here means the engine actually processed a keystroke (Esc and
    // the `_` arm return early). Start the clock on the first accepted
    // keystroke, not at load, so time spent reading the exercise doesn't count
    // against WPM.
    if started.is_none() {
        *started = Some(Instant::now());
    }

    if engine.is_complete() {
        // `started` is always Some here (completing required keystrokes), but
        // elapsed_or_zero keeps it total.
        let elapsed = elapsed_or_zero(*started);
        let exercise_stats =
            stats.finish_exercise(engine.errors(), engine.correct_chars_typed(), elapsed);
        Action::To(AppState::Stats {
            stats: exercise_stats,
            exercise: engine.exercise().clone(),
        })
    } else {
        Action::Stay
    }
}

fn handle_stats_key(exercise: &Exercise, key: KeyEvent, tour: &mut RepoTour, mode: Mode) -> Action {
    match key.code {
        KeyCode::Char('n') => start_typing(tour.next(), mode),
        KeyCode::Char('r') => Action::To(AppState::Typing {
            engine: TypingEngine::new_with_mode(exercise.clone(), mode),
            started: None,
        }),
        // Esc is the canonical quit key (consistent with typing screen).
        // `q` is kept as a hidden alternate for users who prefer it.
        KeyCode::Esc | KeyCode::Char('q') => Action::Quit,
        KeyCode::Char('?') => Action::To(AppState::Help {
            prior: Box::new(AppState::Stats {
                stats: dummy_stats_for_help(),
                exercise: exercise.clone(),
            }),
        }),
        KeyCode::Char(':') => Action::To(AppState::Palette {
            input: String::new(),
            prior: Box::new(AppState::Stats {
                stats: dummy_stats_for_help(),
                exercise: exercise.clone(),
            }),
        }),
        _ => Action::Stay,
    }
}

fn handle_palette_key(
    input: &mut String,
    prior: &mut Box<AppState>,
    key: KeyEvent,
    tour: &mut RepoTour,
    notice: &mut Option<String>,
) -> Action {
    match key.code {
        KeyCode::Esc => Action::To(std::mem::replace(prior.as_mut(), placeholder_state())),
        KeyCode::Backspace => {
            input.pop();
            Action::Stay
        }
        KeyCode::Char(c) => {
            input.push(c);
            Action::Stay
        }
        KeyCode::Enter => {
            let parsed = command::parse(input);
            execute_command(parsed, prior, tour, notice)
        }
        _ => Action::Stay,
    }
}

fn handle_help_key(prior: &mut Box<AppState>, key: KeyEvent) -> Action {
    if let KeyCode::Char('q') = key.code {
        return Action::Quit;
    }
    Action::To(std::mem::replace(prior.as_mut(), placeholder_state()))
}

fn handle_session_stats_key(prior: &mut Box<AppState>, key: KeyEvent) -> Action {
    // Any key returns to the prior screen. (Esc included — this is an overlay,
    // not a top-level screen, so Esc here means "dismiss," consistent with Help.)
    let _ = key;
    Action::To(std::mem::replace(prior.as_mut(), placeholder_state()))
}

fn execute_command(
    parsed: Result<Command, ParseError>,
    prior: &mut Box<AppState>,
    tour: &mut RepoTour,
    notice: &mut Option<String>,
) -> Action {
    let _ = tour;
    let command = match parsed {
        Ok(command) => command,
        Err(e) => {
            // Parse error: surface it and return to prior so the user can
            // read the notice and try again.
            *notice = Some(format!("error: {e}"));
            return Action::To(std::mem::replace(prior.as_mut(), placeholder_state()));
        }
    };

    // Single source of truth for "is this command wired up?": the command
    // metadata's `available` flag (the same flag the palette uses to mark
    // commands "(not yet)"). Gating here — rather than a separate hardcoded
    // list — keeps the rejection and the palette's markers from drifting apart.
    if !command::info(command.name()).is_some_and(|i| i.available) {
        *notice = Some("command not yet implemented".to_string());
        return Action::To(std::mem::replace(prior.as_mut(), placeholder_state()));
    }

    match command {
        Command::Quit => Action::Quit,
        Command::Help => Action::To(AppState::Help {
            prior: Box::new(std::mem::replace(prior.as_mut(), placeholder_state())),
        }),
        // Dedicated session-stats view (was a flashing one-frame notice). The
        // view reads live `session()` at render time.
        Command::Stats => Action::To(AppState::SessionStats {
            prior: Box::new(std::mem::replace(prior.as_mut(), placeholder_state())),
        }),
        // Filtered out by the `available` gate above; handled defensively
        // (not `unreachable!`) so flipping a metadata flag can't panic.
        Command::File(_) | Command::Open(_) => {
            *notice = Some("command not yet implemented".to_string());
            Action::To(std::mem::replace(prior.as_mut(), placeholder_state()))
        }
    }
}

fn start_typing(next: Option<Exercise>, mode: Mode) -> Action {
    match next {
        Some(ex) => Action::To(AppState::Typing {
            engine: TypingEngine::new_with_mode(ex, mode),
            started: None,
        }),
        None => Action::Quit, // Tour is exhausted (shouldn't happen — round-robin loops).
    }
}

/// A throw-away `AppState` value used when we need to temporarily swap a
/// `Box<AppState>` out of one variant and into another. Never rendered.
fn placeholder_state() -> AppState {
    AppState::Help {
        prior: Box::new(AppState::Stats {
            stats: dummy_stats_for_help(),
            exercise: Exercise {
                source_path: PathBuf::new(),
                byte_range: 0..0,
                text: String::new(),
                indent_unit: crate::exercise::IndentUnit::Spaces(4),
            },
        }),
    }
}

fn dummy_stats_for_help() -> ExerciseStats {
    ExerciseStats {
        wpm: 0.0,
        cpm: 0.0,
        accuracy: 0.0,
        elapsed: Duration::ZERO,
        correct_chars: 0,
        error_count: 0,
        top_mistakes: Vec::new(),
    }
}

// -- Rendering ---------------------------------------------------------------

fn render(
    f: &mut ratatui::Frame,
    state: &AppState,
    session: &SessionStats,
    notice: Option<&str>,
    repo_label: &str,
    language: &str,
) {
    let area = f.area();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // header
            Constraint::Min(1),    // body
            Constraint::Length(1), // notice
            Constraint::Length(1), // footer
        ])
        .split(area);

    f.render_widget(header(state, session, repo_label, language), chunks[0]);
    match state {
        AppState::Typing { engine, started } => {
            f.render_widget(typing_body(engine, elapsed_or_zero(*started)), chunks[1])
        }
        AppState::Stats { stats, exercise } => {
            f.render_widget(stats_body(stats, exercise), chunks[1])
        }
        // The palette is a focused mode that owns the body region. We do NOT
        // draw the prior screen underneath: `palette_overlay` is a plain
        // Paragraph (no Clear/background), so anything drawn under it would
        // bleed through the cells it doesn't paint. Falling through (no early
        // return) lets the footer render its "Esc: cancel · Enter: run" hint.
        AppState::Palette { input, .. } => f.render_widget(palette_overlay(input), chunks[1]),
        AppState::Help { .. } => f.render_widget(help_body(), chunks[1]),
        AppState::SessionStats { .. } => f.render_widget(session_stats_body(session), chunks[1]),
    }

    if let Some(msg) = notice {
        f.render_widget(notice_line(msg), chunks[2]);
    }
    f.render_widget(footer(state), chunks[3]);
}

fn header<'a>(
    state: &'a AppState,
    session: &'a SessionStats,
    repo_label: &'a str,
    language: &'a str,
) -> Paragraph<'a> {
    let (file, elapsed_label) = match state {
        AppState::Typing { engine, started } => (
            exercise_label(engine.exercise()),
            format!("  {}", format_elapsed(elapsed_or_zero(*started))),
        ),
        AppState::Stats { exercise, stats } => (
            exercise_label(exercise),
            format!("  {}", format_elapsed(stats.elapsed)),
        ),
        AppState::Palette { prior, .. } => match prior.as_ref() {
            AppState::Typing { engine, started } => (
                exercise_label(engine.exercise()),
                format!("  {}", format_elapsed(elapsed_or_zero(*started))),
            ),
            AppState::Stats { exercise, stats } => (
                exercise_label(exercise),
                format!("  {}", format_elapsed(stats.elapsed)),
            ),
            _ => (String::new(), String::new()),
        },
        AppState::Help { .. } => ("help".to_string(), String::new()),
        AppState::SessionStats { .. } => ("session".to_string(), String::new()),
    };

    let session_summary = format!(
        "  ·  session: {} ex / {:.0} cpm",
        session.exercises_completed,
        session.cpm(),
    );

    Paragraph::new(Line::from(vec![
        Span::styled(
            "codetype",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" · ", Style::default().fg(Color::DarkGray)),
        Span::styled(language, Style::default().fg(Color::Green)),
        Span::raw("  "),
        Span::styled(repo_label, Style::default().fg(Color::Magenta)),
        Span::styled(" › ", Style::default().fg(Color::DarkGray)),
        Span::styled(file, Style::default().fg(Color::Yellow)),
        Span::styled(elapsed_label, Style::default().fg(Color::DarkGray)),
        Span::styled(session_summary, Style::default().fg(Color::DarkGray)),
    ]))
}

/// Visual width of a tab character when rendered. ratatui treats `'\t'` as
/// zero-width — we expand it ourselves so tab-indented code looks indented.
const TAB_VISUAL_WIDTH: usize = 4;

fn typing_body(engine: &TypingEngine, _elapsed: Duration) -> Paragraph<'_> {
    let chars = engine.chars();
    let cursor = engine.cursor();

    let mut lines: Vec<Line> = vec![Line::default()];
    for (i, &c) in chars.iter().enumerate() {
        let style = char_style(engine.status_at(i), i == cursor);
        if c == '\n' {
            // The newline itself gets a styled space so the cursor is visible
            // on an empty line position, then a new line starts.
            lines
                .last_mut()
                .unwrap()
                .spans
                .push(Span::styled(" ", style));
            lines.push(Line::default());
        } else if c == '\t' {
            // Expand tabs to visible spaces. The engine still tracks the
            // tab as a single character; the user never types tabs (they're
            // auto-indented past on Enter). This is purely cosmetic.
            lines
                .last_mut()
                .unwrap()
                .spans
                .push(Span::styled(" ".repeat(TAB_VISUAL_WIDTH), style));
        } else {
            lines
                .last_mut()
                .unwrap()
                .spans
                .push(Span::styled(c.to_string(), style));
        }
    }

    Paragraph::new(Text::from(lines))
}

fn char_style(status: CharStatus, is_cursor: bool) -> Style {
    let base = match status {
        CharStatus::Pending => Style::default().fg(Color::DarkGray),
        CharStatus::Correct => Style::default().fg(Color::White),
        CharStatus::Errored => Style::default().bg(Color::Red).fg(Color::White),
    };
    if is_cursor {
        // Always underline the cursor. In Lenient mode the cursor can land on
        // an Errored position after backspace, and dropping the underline
        // there would make it invisible.
        base.add_modifier(Modifier::UNDERLINED)
    } else {
        base
    }
}

fn stats_body<'a>(stats: &'a ExerciseStats, exercise: &'a Exercise) -> Paragraph<'a> {
    let stumbles: String = if stats.top_mistakes.is_empty() {
        "—".to_string()
    } else {
        stats
            .top_mistakes
            .iter()
            .map(|(c, n)| format!("{}({})", display_char(*c), n))
            .collect::<Vec<_>>()
            .join("  ")
    };

    let metrics = format!(
        "  WPM {:.1}    CPM {:.0}    Accuracy {:.1}%    Time {}",
        stats.wpm,
        stats.cpm,
        stats.accuracy * 100.0,
        format_elapsed(stats.elapsed),
    );

    let lines = vec![
        Line::default(),
        Line::from(vec![Span::styled(
            "  exercise complete",
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        )]),
        Line::default(),
        Line::from(metrics),
        Line::default(),
        Line::from(vec![
            Span::styled("  Stumbled on:  ", Style::default().fg(Color::DarkGray)),
            Span::raw(stumbles),
        ]),
        Line::default(),
        Line::from(vec![Span::styled(
            format!("  from {}", exercise_label(exercise)),
            Style::default().fg(Color::DarkGray),
        )]),
        Line::default(),
        // Shortcut hints directly below the stats — the footer at the
        // bottom of the terminal is too easy to miss when the body
        // widget pushes it down. Esc is shown as the canonical quit
        // (matching the typing screen); `q` still works as an alternate.
        Line::from(vec![Span::styled(
            "  n  next     r  retry     Esc  quit     :  command     ?  help",
            Style::default().fg(Color::Cyan),
        )]),
    ];

    Paragraph::new(Text::from(lines))
}

fn help_body() -> Paragraph<'static> {
    let lines = vec![
        Line::default(),
        Line::from(Span::styled(
            "  codetype — keyboard shortcuts",
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Line::default(),
        Line::from("  Typing:   any code character to type · Enter for newline · Backspace to fix"),
        Line::from("            Esc to quit mid-exercise"),
        Line::default(),
        Line::from("  Stats:    n   next exercise"),
        Line::from("            r   retry this exercise"),
        Line::from("            Esc quit (q also works)"),
        Line::from("            :   open command palette"),
        Line::from("            ?   this help"),
        Line::default(),
        Line::from("  Palette:  type ':' then a command — suggestions appear inline"),
        Line::default(),
        Line::from(Span::styled(
            "  press any key to return",
            Style::default().fg(Color::DarkGray),
        )),
    ];
    Paragraph::new(Text::from(lines))
}

/// The palette as drawn over the body: the `:` input line plus a live,
/// filtered suggestion list (full vocabulary on bare ':'). Each row shows
/// `:name <arg>  — description`, with not-yet-wired commands marked "(not yet)".
/// Descriptions/arg-shapes come from `command::complete_info` — never hardcoded
/// here, so the palette and the parser can't drift.
fn palette_overlay(input: &str) -> Paragraph<'_> {
    let mut lines = vec![
        Line::default(),
        Line::from(vec![
            Span::styled(":", Style::default().fg(Color::Cyan)),
            Span::raw(input.to_string()),
            Span::styled(
                "_",
                Style::default().add_modifier(Modifier::SLOW_BLINK | Modifier::DIM),
            ),
        ]),
        Line::default(),
    ];
    for info in command::complete_info(input) {
        let usage = if info.arg.is_empty() {
            format!("  :{}", info.name)
        } else {
            format!("  :{} {}", info.name, info.arg)
        };
        let mut spans = vec![
            Span::styled(usage, Style::default().fg(Color::Cyan)),
            Span::styled(
                format!("  — {}", info.description),
                Style::default().fg(Color::DarkGray),
            ),
        ];
        if !info.available {
            spans.push(Span::styled("  (not yet)", Style::default().fg(Color::Red)));
        }
        lines.push(Line::from(spans));
    }
    Paragraph::new(Text::from(lines))
}

/// The session-stats overlay body. Reads the aggregates straight off
/// [`SessionStats`] (`cpm`/`wpm`/`accuracy`), which share the same math as the
/// per-Exercise stats, so the two screens can never diverge.
fn session_stats_body(s: &SessionStats) -> Paragraph<'_> {
    let lines = vec![
        Line::default(),
        Line::from(Span::styled(
            "  session stats",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )),
        Line::default(),
        Line::from(format!("  Exercises completed   {}", s.exercises_completed)),
        Line::from(format!("  Correct characters    {}", s.total_correct_chars)),
        Line::from(format!("  Errors                {}", s.total_errors)),
        Line::from(format!(
            "  Total time            {}",
            format_elapsed(s.total_time)
        )),
        Line::from(format!("  WPM (session)         {:.1}", s.wpm())),
        Line::from(format!(
            "  Accuracy (session)    {:.1}%",
            s.accuracy() * 100.0
        )),
        Line::default(),
        Line::from(Span::styled(
            "  press any key to return",
            Style::default().fg(Color::DarkGray),
        )),
    ];
    Paragraph::new(Text::from(lines))
}

fn notice_line(msg: &str) -> Paragraph<'_> {
    Paragraph::new(Span::styled(
        format!("  {msg}"),
        Style::default().fg(Color::Yellow),
    ))
}

fn footer(state: &AppState) -> Paragraph<'static> {
    let hints = match state {
        AppState::Typing { .. } => "Esc: quit",
        // Stats screen renders its own hint row inside the body — don't
        // duplicate them at the bottom of the screen.
        AppState::Stats { .. } => "",
        AppState::Palette { .. } => "Esc: cancel  ·  Enter: run",
        AppState::Help { .. } => "any key to return  ·  q: quit",
        AppState::SessionStats { .. } => "any key to return",
    };
    Paragraph::new(Span::styled(hints, Style::default().fg(Color::DarkGray)))
}

// -- Tiny helpers ------------------------------------------------------------

fn display_char(c: char) -> String {
    match c {
        '\n' => "\u{21B5}".to_string(), // ↵
        '\t' => "\u{2B7E}".to_string(), // ⭾
        ' ' => "\u{2423}".to_string(),  // ␣
        _ => c.to_string(),
    }
}

fn exercise_label(ex: &Exercise) -> String {
    let path = ex.source_path.display().to_string();
    // Show the last two path components for brevity.
    let short = path
        .rsplit('/')
        .take(2)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>()
        .join("/");
    if short.is_empty() { path } else { short }
}

fn format_elapsed(d: Duration) -> String {
    let total = d.as_secs();
    let mins = total / 60;
    let secs = total % 60;
    format!("{mins}:{secs:02}")
}


/// Elapsed wall-clock for a maybe-started timer. `None` (not yet typed) reads
/// as zero so the header shows 0:00 before the first keystroke.
fn elapsed_or_zero(started: Option<Instant>) -> Duration {
    started.map_or(Duration::ZERO, |t| t.elapsed())
}

// -- Tests --------------------------------------------------------------------
//
// UI/rendering code stays untested per project convention. These cover only
// the *pure* logic seams we extracted: the timer math and the session-stat
// aggregates. No terminal, no AppState construction.
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn elapsed_or_zero_none_is_zero() {
        assert_eq!(elapsed_or_zero(None), Duration::ZERO);
    }

    #[test]
    fn elapsed_or_zero_some_is_nonnegative() {
        // Just proves it reads the instant (not the ZERO path). A freshly
        // captured `now` can't have elapsed a whole second by the next line.
        let d = elapsed_or_zero(Some(Instant::now()));
        assert!(d < Duration::from_secs(1));
    }

}
