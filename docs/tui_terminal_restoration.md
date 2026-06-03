# TUI Terminal Restoration Smoke Check

The Rust TUI installs a panic hook that disables raw mode and leaves the alternate screen before the default panic handler runs.

Manual debug smoke check:

1. Run `EMT_TUI_FORCE_PANIC_AFTER_FIRST_DRAW=1 cargo run -- --tui`.
2. Confirm the program panics after the first rendered frame.
3. Confirm the shell prompt is visible, typed input is echoed normally, and `stty -a` does not show raw mode behavior.
4. Run `cargo run -- --tui`, press `q`, and confirm normal shutdown still restores the terminal.

`EMT_TUI_FORCE_PANIC_AFTER_FIRST_DRAW` is compiled only for debug builds.
