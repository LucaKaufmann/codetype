# CodeType

**A terminal typing trainer that drills the real code from your own repo.**

CodeType is a terminal typing trainer (TUI). It parses the source files in a
local Git repository with [tree-sitter](https://tree-sitter.github.io/), pulls
out real functions and type definitions, and has you type them. You practice on
the code you actually write.

![CodeType drilling its own source in the terminal](assets/screenshot.png)

## Why

Most typing trainers give you lorem ipsum or generic snippets. The code you
type all day looks nothing like that: your project's identifiers, your
language's punctuation soup. CodeType pulls its exercises from the repo you
point it at, so you drill the keystrokes you'll actually use.

## Features

- Generates exercises from real source files via tree-sitter parsing.
- Auto-detects the dominant language in a repository.
- Two practice modes: **lenient** (Monkeytype-style) and **strict**.
- A distraction-free terminal UI built with [ratatui](https://ratatui.rs/)
  and [crossterm](https://github.com/crossterm-rs/crossterm).
- Single static binary, no runtime dependencies beyond `git`.
- Optional `CODETYPE.md` leaderboard you can commit and compete on as a team.
- Debug `--print` mode to inspect extracted exercises.

## Supported languages

| Language   | Extensions | `--lang` value          |
| ---------- | ---------- | ----------------------- |
| Swift      | `.swift`   | `swift`                 |
| Rust       | `.rs`      | `rust` (alias `rs`)     |
| TypeScript | `.ts`      | `typescript` (alias `ts`) |

## Install

Clone the repository, then install the binary with Cargo:

```bash
git clone https://github.com/LucaKaufmann/codetype.git
cd codetype
cargo install --path .
```

This builds and installs the `codetype` binary into your Cargo bin directory
(typically `~/.cargo/bin`).

## Usage

```
codetype <path-to-repo> [--lang <lang>] [--strict] [--as <name>]
```

**Auto-detect the language** (infers the dominant language from the repo's
tracked files):

```bash
codetype ~/path/to/repo
```

**Override the language** with `--lang` (accepts `swift`, `rust`/`rs`,
`typescript`/`ts`):

```bash
codetype ~/path/to/repo --lang rust
codetype ~/path/to/repo --lang ts
```

**Strict mode**, where wrong keystrokes don't advance the cursor:

```bash
codetype ~/path/to/repo --strict
```

**Print mode**, to dump the extracted exercises to stdout and exit (no TUI).
Handy for debugging extraction. Limit how many with `--count` (default 5):

```bash
codetype ~/path/to/repo --print
codetype ~/path/to/repo --print --count 10
```

## Tracking scores

CodeType can keep a running record of how you do across sessions in a
`CODETYPE.md` file at the root of the repo you're drilling. Check it into git
and a whole team gets a shared leaderboard, rendered as a table right on the
repo page.

Nothing is written unless you ask. When you quit a session that completed at
least one exercise, CodeType asks:

```
Submit this session to CODETYPE.md? [y/N]
```

Press <kbd>y</kbd> to fold the session into the leaderboard and write the file;
any other key quits without touching anything. After submitting, you get a line
telling you where you placed.

Each row tracks best and average WPM, accuracy, session count, and the date you
last played — split per language, with an `overall` rollup per player. Rows are
sorted by name (not score), so everyone only ever edits their own rows and
concurrent submissions merge cleanly in git.

You're identified by your git `user.name` (falling back to `user.email`). To use
a handle instead, pass `--as`:

```bash
codetype ~/path/to/repo --as speedy
```

Scores are plain text and on the honor system — there's no tamper-proofing, by
design.

## Modes

- **Lenient** (default). Monkeytype-style: a wrong key still advances the
  cursor and is marked as an error, and you press <kbd>Backspace</kbd> to
  correct it. Good for flow and raw speed.
- **Strict** (`--strict`). The cursor only moves when you type the expected
  key, so you can't get ahead of yourself. This is the mode for drilling
  precision.

## Keybindings

| Key            | Action                                   |
| -------------- | ---------------------------------------- |
| <kbd>Esc</kbd> | Quit (works on every screen)             |
| <kbd>n</kbd>   | Next exercise                            |
| <kbd>r</kbd>   | Retry the current exercise               |
| <kbd>:</kbd>   | Open the command palette                 |
| <kbd>?</kbd>   | Show help                                |
| <kbd>q</kbd>   | Quit (on the stats screen)               |
| <kbd>y</kbd>   | Submit the session (on the quit prompt)  |

## Requires a Git repository

CodeType lists source files by running `git ls-files`, so the target directory
**must be a Git repository**. Only Git-*tracked* files are drilled; anything
untracked or ignored is skipped. Point it at a non-Git directory and it reports
an error.

## Roadmap and known limitations

- No `.tsx` yet. Only plain `.ts` TypeScript files are parsed.
- The `CODETYPE.md` leaderboard tracks scores across sessions, but exercise
  selection still doesn't remember which files you've already drilled.
- More languages and smarter exercise selection are on the list.

## Contributing

Contributions are welcome. Open an issue to talk through a change, or send a
pull request. Keep the parsing logic covered by tests where it makes sense.

## License

MIT © Luca Kaufmann
