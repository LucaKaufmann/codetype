# Code review — Session 4 changes (timer, notice, SessionStats, palette)

**Scope:** the working-tree diff of the Session 4 feedback work — `src/app.rs`
(+timer `Option<Instant>`, notice-lifetime fix, `AppState::SessionStats`,
`palette_overlay`) and `src/command.rs` (+`CommandInfo`/`COMMAND_INFO`/
`complete_info`). 629-line diff over 2 files.

**Method:** `/code-review` (high effort, recall-biased) — 4 independent finder
angles (line-by-line correctness, removed-behavior + cross-file, cleanup, and
altitude) followed by verification against the source. Build/clippy/tests were
green before review (186 passed, 0 failed, clippy clean, fmt clean); the items
below are review findings, not test failures.

Ranked most-severe first. Severity: **P1** = should fix before release ·
**P2** = maintainability/correctness worth fixing · **P3** = minor · **P4** =
micro.

## Resolution (this branch)

Findings **#1–#4 were fixed** in the follow-up commit on this branch
(185 tests pass, clippy + fmt clean):

- **#1** — `render`'s `Palette` arm now draws only `palette_overlay` into
  `chunks[1]` (no prior-body render underneath) and no longer returns early, so
  there's no bleed-through and the footer hint renders.
- **#2** — added `SessionStats::{cpm,wpm,accuracy}` in `stats.rs` (reusing the
  canonical private helpers); `app.rs` calls those, and the duplicated
  `cumulative_cpm`/`session_wpm`/`session_accuracy` (and their tests) are gone.
- **#3 + #4** — `execute_command` now gates "not yet implemented" on
  `command::info(name).available` (single source of truth, shared with the
  palette), via a new `Command::name()`. This also gives the previously-dead
  `command::info()` a real production caller.

Findings **#5–#9 were also fixed** in a second follow-up commit
(185 tests pass, clippy + fmt clean):

- **#5** — the timer now starts only on a real typing attempt (`Char`/`Enter`),
  not on a `Backspace` pressed while orienting (`handle_typing_key`).
- **#6** — `COMMAND_INFO` is now the single source of truth: the `COMMANDS`
  const is gone, `parse` looks commands up there and derives the arg-required
  rule from each entry's `arg` shape, and a round-trip test couples the table
  to the `Command` enum.
- **#7** — extracted `take_prior` / `return_to_prior` helpers; every
  overlay-dismiss and overlay-open site now calls them instead of repeating
  `std::mem::replace(prior.as_mut(), placeholder_state())`. (A full `Overlay`
  type was judged over-engineering for two overlays.)
- **#8** — a notice now persists until the next *meaningful* keypress: it's
  cleared only when the key produces an action (not `Stay`) and wasn't set this
  turn, so no-op keys no longer dismiss it.
- **#9** — `complete_info` filters `COMMAND_INFO` by prefix directly (no more
  `complete()` + O(n²) `contains` scan); `complete` is now a thin wrapper.

All nine findings resolved.

---

## 1. [P1 · correctness/UX] Palette overlay bleeds the body through — `src/app.rs:516–526`

`render`'s `Palette` arm draws the prior screen's body into `chunks[1]`
(lines 518/521) and then draws `palette_overlay(input)` into the **same**
`chunks[1]` (line 525). `palette_overlay` is a plain `Paragraph` with no
`Block`/background and no preceding `Clear` widget — ratatui's `Paragraph` only
writes the cells its spans cover (plus `set_style`), so the underlying exercise
or stats text **shows through** every row/column the suggestion list doesn't
paint over (e.g. all body rows below the ~7-line palette). The result is
overlapping, garbled text behind the palette.

This is a **regression**: before Session 4 the palette prompt (`palette_line`)
rendered into `chunks[3]` (the one-row footer), leaving the body a clean
backdrop. Moving it into the body region without clearing introduced the bleed.

Separately, the early `return` at line 526 means `footer(state)` (line 535) is
never reached for `Palette`, so the palette shows no bottom hint and the
`AppState::Palette => "Esc: cancel · Enter: run"` arm in `footer` is dead.

**Fix:** render `ratatui::widgets::Clear` over `chunks[1]` before
`palette_overlay`, **or** drop the underneath body render and let
`palette_overlay` own `chunks[1]` (give it a full-area background if you want a
visible panel). If a bottom hint is wanted, render the footer instead of
returning early.

## 2. [P2 · maintainability] Session WPM/accuracy duplicate `stats.rs` math — `src/app.rs:870–886`

`session_wpm` divides by a hardcoded `5.0` (the comment concedes it's copying
`stats::CHARS_PER_WORD` because that constant is private), and
`session_accuracy` re-derives `correct / (correct + errors)` with the same
`total == 0 → 1.0` guard as `stats::accuracy`. The per-exercise Stats screen
already gets these from `stats.rs`. If the WPM convention or the empty-input
rule ever changes in `stats.rs`, the per-exercise and session screens silently
diverge.

**Fix:** make `stats::{wpm, accuracy}` (and `CHARS_PER_WORD`) `pub(crate)` and
call them, or add `SessionStats::wpm()/accuracy()` in `stats.rs` so both screens
share one source of truth.

## 3. [P2 · maintainability] `CommandInfo.available` can drift from real wiring — `src/command.rs:93–95, 105–136`

Whether a command "works" is encoded twice: the hand-set `available` bool in
`COMMAND_INFO`, and independently in `app.rs::execute_command` (the
`File | Open => "command not yet implemented"` arm). Nothing couples them — the
doc comment merely asserts they agree, and the metadata tests only check
`available` against itself. When `:open`/`:file` are implemented, the palette
will keep showing (or stop showing) "(not yet)" out of sync with reality, with
no test to catch it.

**Fix:** drive availability from a single source, or add a test that asserts
`available` matches `execute_command`'s actual behavior per command.

## 4. [P2 · dead code] `command::info()` is unused in production — `src/command.rs:151`

`pub fn info(name)` is called only by the `info_lookup_round_trips` test;
`app.rs` uses `complete_info` exclusively (confirmed by grep). It's public API +
a doc comment + a test carrying weight for zero real callers.

**Fix:** remove `info()` and `info_lookup_round_trips`, or document it as
reserved for a planned caller.

## 5. [P3 · correctness] Timer starts on a no-op keystroke — `src/app.rs` (`handle_typing_key`, the `if started.is_none()` block)

The clock starts on the first keystroke the engine *processes*, but the
`Char`/`Backspace`/`Enter` arms all fall through to the `started` set — including
`Backspace` at cursor 0 (a no-op) or other keystrokes that make no forward
progress. A user who taps Backspace while orienting starts the clock before
typing a single correct character, slightly deflating WPM. The comment says
"first accepted keystroke" but conflates "engine was invoked" with "progress was
made."

**Fix:** start the clock only when the cursor actually advanced (e.g. capture
`engine.cursor()` before/after, or start on the first correct char), not on any
processed key.

## 6. [P3 · altitude] Command vocabulary is spread across 4 hand-aligned tables — `src/command.rs:32, 52–58, 105`

The `Command` enum + `parse()` arms, `COMMANDS`, and `COMMAND_INFO` (whose `arg`
strings re-encode the `arg_required` vs `no_arg` decision in `parse`) must all
stay aligned by hand. Alignment is pinned only by runtime tests, not the type
system: a command added to the enum/`parse` but forgotten in `COMMAND_INFO`
fails a test rather than failing to compile, and an `arg` shape can disagree
with `parse` for any command not covered by `command_info_arg_shape_matches_parse`.

**Fix:** a single table that drives `parse`, `complete`, and the metadata so the
arg-shape and availability can't disagree with the parser.

## 7. [P3 · altitude] `SessionStats`/`Help` duplicate the "overlay with prior" pattern — `src/app.rs`

`SessionStats` and `Help` are the same concept twice: identical
`{ prior: Box<AppState> }` shape, near-identical "any key returns to prior"
handlers, and copy-paste `execute_command` arms doing
`Box::new(std::mem::replace(prior.as_mut(), placeholder_state()))`. `render` and
`header` silently `_ => {}` / return empty for any prior that isn't
`Typing`/`Stats`, so a future overlay-over-overlay renders a blank body and
nobody notices until it's exercised by hand.

**Fix:** a small shared `Overlay` abstraction (or a helper for the prior-swap +
the render/header/footer arms) so new overlays don't re-replicate five places.

## 8. [P3 · behavior] Notice is cleared by *any* keypress, including no-ops — `src/app.rs:307` (`handle_key`)

`*notice = None` runs at the top of `handle_key` on every key, so a persisted
message (`:open not yet implemented`, a parse error) is dismissed by any key —
even ones that resolve to `Action::Stay` (the `_ => Stay` arms). The lifetime is
also an implicit ordering contract (every notice writer must run *after* line
307); a future handler that sets a notice before delegating would have it wiped
on the next key with no compiler help.

**Fix:** clear the notice only on a key that produces an action, or model it with
a TTL / carry it in `Action` rather than a shared `&mut Option<String>` cleared
by convention.

## 9. [P4 · efficiency] Palette suggestions rebuilt every render — `src/command.rs:141`, `src/app.rs:748`

`complete_info` calls `complete` then does an O(n²) `names.contains(&info.name)`
scan and allocates an intermediate `Vec`; `palette_overlay` then `format!`s
two/three heap strings per suggestion on every keystroke while the palette is
open. Trivial for 5 static commands, but it's gratuitous: `complete_info` could
prefix-filter `COMMAND_INFO` directly, and the usage strings are static per
command and could be precomputed.

**Fix:** filter `COMMAND_INFO` by the same prefix predicate `complete` uses;
store/precompute the static usage string on `CommandInfo`.

---

## Considered and refuted

- **Near-zero elapsed → absurd WPM on instant completion.** Raised by two
  finders (start the clock on the first keystroke, complete on the same one →
  `elapsed ≈ 0` → `cpm` divides by ~0 minutes). **Refuted:** the extractor's
  `LengthFilter` floors every exercise at ≥100 bytes / ≥4 lines, so an exercise
  cannot be completed on its first keystroke; `elapsed` always spans real human
  typing across ≥100 characters, and the exact-`Duration::ZERO` guard in
  `stats::cpm` covers the degenerate case.

## Suggested priority

Fix **#1** before any release/demo (it makes the palette look broken). **#2–#4**
are quick, high-value cleanups. **#5–#9** are optional polish; #6/#7 are the
natural groundwork if the command palette grows.
