# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- `hsmc` 0.5.1 statechart drives the entire device lifecycle. The control
  flow (boot → wake-up animation → settle → running animation → active
  jiggle/flash → end-of-day shutdown spiral → power-down) is expressed
  declaratively in one `statechart!` block; no more atomic flags, no more
  per-tick `Phase` enum, no hand-rolled main loop.
- Mouse animations driven by the chart's `during:` activities:
  - **Wake-up** (`WakingDisplay`): 10 frantic horizontal sin-shaped sweeps
    (~12 Hz, ±60 px, ~640 ms) — simulates "shake the mouse to wake the
    display."
  - **Running** (`ShowingRunning`): 3 quick clockwise circles
    (radius 40 px, ~600 ms) — universal "spinner / in progress" cue.
  - **Pre-shutdown** (`Ending → ShutdownAnim`): inward spiral 50 → 5 px
    over 3 turns (~2 s), 30 s before lifetime end — cursor visibly winds
    down.
- 2 s `Settling` state between wake-up and running animations so the
  display has time to come out of sleep before the spinner draws.
- All animations carry sub-pixel residue forward across `i8` HID delta
  reports so the rendered shape matches the intended geometry rather than
  losing ~half its motion to truncation.

### Changed

- **`RUN_DURATION` from 8 hours to 3 h 50 m**. Math-optimal under the
  user's workday distribution (start `triangular(8.0, mode=8.5, 9.5)`,
  lunch 12:00–13:00, end `triangular(16.0, mode=17.5, 19.0)`) for "screen
  goes to sleep during the lunch hour on most days." See
  `/tmp/jiggly_runtime_v3.py` for the simulation.
- HID `poll_ms` 60 → 8 (125 Hz). At the previous 60 ms poll the host
  was discarding ~7 of every 8 animation frames the firmware emitted.
- Boot LED `R→G→B` step 180 ms → 60 ms; 1.5 s pre-animation USB-settle
  delay removed. Reset → wake-shake latency dropped from ~2 s to ~240 ms.
- Watchdog feeding moved out of the imperative main loop into its own
  `#[embassy_executor::task]`; the chart owns user-visible state, the
  watchdog task owns the periodic kick.
- USB descriptor unchanged (still spoofs Logitech G502 VID `0x046d` /
  PID `0xc07d`).

### Removed

- `JIGGLE_FLAG: AtomicBool` and all `swap`/`store` calls.
- `Phase` enum and the per-tick `phase_for(remaining)` switch — the
  breathing color is now read directly from elapsed time inside the
  `breathe_color` `during:`.
- `led_task` and `sleep_chunked` helper — both subsumed by the chart.
- `RUN_DURATION = Duration::from_secs(8 * 60 * 60)` and similar
  raw-integer-millisecond constants. All timing is now `Duration`-typed
  with named constructors (`from_hours`, `from_mins`, `from_millis`).

## [0.1.0] - 2026-04-27

### Added

- Initial Embassy-based `no_std` firmware for the Seeed Studio Xiao RP2040.
- USB HID Boot Mouse, masquerading as a Dell MS116 (VID `0x413c`, PID `0x301a`).
- Jiggle pattern: pick a random axis from the RP2040 ROSC `RANDOMBIT`, nudge
  +1 px, dwell 25 ms, nudge −1 px, then sleep 4.5 minutes before the next
  cycle.
- 8-hour workday window after boot or `RESET (R)` press, then silent idle
  (watchdog still fed) until the next manual reset.
- 8-second hardware watchdog, fed in 5-second chunks during long sleeps.
- `mise.toml` pinning rust 1.95.0 with explicit components
  (`cargo`, `clippy`, `llvm-tools`, `rust-src`, `rust-std`, `rustc`,
  `rustfmt`) and the `thumbv6m-none-eabi` target, plus cargo helpers
  (`cargo-binutils`, `uf2conv`, `cargo-watch`, `cargo-bloat`,
  `cargo-expand`).
- `justfile` recipes for `build`, `release`, `check`, `clippy`, `fmt`,
  `lint`, `ci`, `bin`, `uf2`, `flash`, `size`, `bloat`, `expand`, and
  `bootstrap`. All recipes execute inside `mise exec -- sh -eu -c` so the
  pinned toolchain is used regardless of shell activation state.

[Unreleased]: https://github.com/swaits/jiggly/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/swaits/jiggly/releases/tag/v0.1.0
