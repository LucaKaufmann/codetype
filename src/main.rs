use std::path::PathBuf;

use clap::Parser;
use codetype::engine::Mode;

/// CodeType — drill the actual code in your repo.
#[derive(Parser)]
#[command(version, about, long_about = None)]
struct Cli {
    /// Path to a local Git repository.
    repo: PathBuf,

    /// Language to drill (e.g. swift, rust, typescript). When omitted,
    /// CodeType infers the dominant language from the repo's tracked files.
    #[arg(long)]
    lang: Option<String>,

    /// Name to record scores under in CODETYPE.md. Overrides the git identity
    /// (user.name / user.email) — handy for a handle instead of your real name.
    #[arg(long = "as")]
    as_name: Option<String>,

    /// Use strict mode: wrong keystrokes do not advance the cursor — the
    /// user must type the expected key to make progress. Default is lenient
    /// mode (Monkeytype-style: wrong keys advance, backspace to fix).
    #[arg(long)]
    strict: bool,

    /// Debug: print the first few extracted Exercises to stdout and exit
    /// (no TUI). Leading whitespace is shown with visible markers so we can
    /// tell whether dedent or the renderer is at fault for any indent bugs.
    #[arg(long)]
    print: bool,

    /// Number of Exercises to print when `--print` is set.
    #[arg(long, default_value_t = 5)]
    count: usize,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    if cli.print {
        codetype::app::print_exercises(&cli.repo, cli.count, cli.lang)
    } else {
        let mode = if cli.strict {
            Mode::Strict
        } else {
            Mode::Lenient
        };
        codetype::app::run(&cli.repo, mode, cli.lang, cli.as_name)
    }
}
