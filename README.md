# CodeType

**Drill the actual code in your repo — a terminal typing trainer for developers.**

CodeType is a minimalist terminal typing trainer (TUI) that extracts real
exercises — functions, types, and other constructs — straight from the source
code in a local Git repository using [tree-sitter](https://tree-sitter.github.io/),
then has you type them. Build muscle memory on the syntax, identifiers, and
patterns you work with every day.

<!-- TODO: add demo gif / asciinema cast here, e.g.
     ![CodeType demo](docs/demo.gif)
     or an asciinema embed. -->

## Why

Typical typing trainers feed you lorem-ipsum prose or generic code snippets.
But the thing you actually type all day is *your* codebase — its naming
conventions, its boilerplate, its language's punctuation soup. CodeType pulls
exercises from the repo you point it at, so every keystroke you practice is one
you'll use for real.

## Features

- Generates exercises from real source files via tree-sitter parsing.
- Auto-detects the dominant language in a repository.
- Two practice modes: **lenient** (Monkeytype-style) and **strict**.
- Fast, distraction-free terminal UI built with [ratatui](https://ratatui.rs/)
  and [crossterm](https://github.com/crossterm-rs/crossterm).
- Single static binary, no runtime dependencies beyond `git`.
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
cargo install --path .
```

This builds and installs the `codetype` binary into your Cargo bin directory
(typically `~/.cargo/bin`).

## Usage

```
codetype <path-to-repo> [--lang <lang>] [--strict]
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

**Strict mode** — wrong keystrokes don't advance the cursor:

```bash
codetype ~/path/to/repo --strict
```

**Print mode** — dump the extracted exercises to stdout and exit (no TUI),
useful for debugging extraction. Limit how many with `--count` (default 5):

```bash
codetype ~/path/to/repo --print
codetype ~/path/to/repo --print --count 10
```

## Modes

- **Lenient** (default) — Monkeytype-style. A wrong key still advances the
  cursor and is marked as an error; press <kbd>Backspace</kbd> to go back and
  correct it. Good for flow and measuring raw speed.
- **Strict** (`--strict`) — the cursor only moves when you type the *expected*
  key. You can't get ahead of yourself, which forces precision.

## Keybindings

| Key            | Action                                   |
| -------------- | ---------------------------------------- |
| <kbd>Esc</kbd> | Quit (works on every screen)             |
| <kbd>n</kbd>   | Next exercise                            |
| <kbd>r</kbd>   | Retry the current exercise               |
| <kbd>:</kbd>   | Open the command palette                 |
| <kbd>?</kbd>   | Show help                                |
| <kbd>q</kbd>   | Quit (on the stats screen)               |

## Requires a Git repository

CodeType enumerates source files by shelling out to `git ls-files`, so the
target directory **must be a Git repository**. Only files that are *tracked* by
Git are drilled — untracked, ignored, and uncommitted-but-unstaged files are
skipped. If you point CodeType at a non-Git directory it will report an error.

## Roadmap / Known limitations

- **`.tsx` is not yet supported** — only plain `.ts` TypeScript files are
  parsed today.
- **Cross-session progress tracking** — surfacing "files you haven't drilled
  before" across runs is planned but not yet implemented; each session is
  currently independent.
- Additional languages and exercise heuristics are on the wishlist.

## Contributing

Contributions are welcome. Open an issue to discuss a change, or send a pull
request. Please keep exercises and parsing logic covered by tests where it
makes sense.

## License

MIT © Luca Kaufmann
