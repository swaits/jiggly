# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- **Composite USB device — mouse + keyboard HID** under one VID/PID. A new
  `WakingHost` parent state wraps two substates: `WakingWithKeyboard`
  (`default`) taps Left Shift 4× then idles, and parent's
  `KBD_WAKE_DURATION` timeout drives the transition to `WakingWithMouse`
  which runs the existing shake animation. Reason: macOS does not reliably
  wake from raw HID mouse motion alone; a keyboard event does. Shift is a
  modifier with no character side effect, so it's safe even with focus on a
  text field at the moment of wake. Adds a second `HidWriter` against the
  same `embassy_usb::Builder` (no hub simulation; standard USB composite),
  plus `KbdHid`/`KBD_HID_STATE` machinery and a `send_kbd` helper.
- `hsmc` 0.5.1 statechart drives the entire device lifecycle. The control
  flow (boot → host wake (kbd → mouse) → settle → spinner → active
  jiggle/flash → end-of-day spiral → power-down) is expressed declaratively
  in one `statechart!` block; no more atomic flags, no more per-tick
  `Phase` enum, no hand-rolled main loop.
- Mouse animations driven by the chart's `during:` activities:
  - **Wake-up** (`WakingDisplay`): 10 frantic horizontal sin-shaped sweeps
    (~12 Hz, ±60 px, ~640 ms) — simulates "shake the mouse to wake the
    display."
  - **Running** (`ShowingRunning`): 3 quick clockwise circles
    (radius 40 px, ~600 ms) — universal "spinner / in progress" cue.
  - **Pre-shutdown** (`Ending → ShutdownAnim`): eased inward spiral
    80 → 2 px over 5 turns (~5 s), starting 30 s before lifetime end. A
    shared phase `u(t) = t^2.5` drives both radius and angle, so the
    rotation accelerates as the radius collapses — the visual signature
    of a coin/Euler-disk spinning down.
  - **Warnings** (`Active → Warning10 / Warning5`): mini versions of
    the same spiral fire 10 min and 5 min before shutdown — 30 → 2 px
    over 2 turns (~1.5 s) and 50 → 2 px over 3 turns (~2.5 s) — so the
    user gets escalating kinetic foreshadowing of the death gesture
    that's coming.
- 2 s `Settling` state between wake-up and running animations so the
  display has time to come out of sleep before the spinner draws.
- All animations carry sub-pixel residue forward across `i8` HID delta
  reports so the rendered shape matches the intended geometry rather than
  losing ~half its motion to truncation.

### Changed

- **USB descriptor: Logitech G502 (`046d:c07d`, mouse-only) → Logitech
  Unifying Receiver (`046d:c52b`, real composite mouse+keyboard).** Product
  string `"G502 Mouse"` → `"USB Receiver"`. Bumping the PID also invalidates
  the host's cached HID descriptor on first plug after reflash.
- **Statechart names cleaned up for symmetry**:
  - `BootSweep` → `Booting`; `Ev::SweepDone` → `Ev::BootDone`;
    `led_rgb_sweep` → `boot_sweep`.
  - `WakingDisplay` (flat) → `WakingHost` parent with `WakingWithKeyboard`
    + `WakingWithMouse` substates; `animate_wake` → `wake_with_mouse`,
    matched by new `wake_with_keyboard`.
  - `ShowingRunning` → `Spinning`; `Ev::RunDone` → `Ev::SpinDone`;
    `animate_running` → `animate_spinner`.
  - `Ending::ShutdownAnim` → `Ending::Spiraling`;
    `Ev::ShutdownAnimDone` → `Ev::SpiralDone`;
    `animate_shutdown` → `animate_final_spiral`;
    `SHUTDOWN_RADIUS_START`/`SHUTDOWN_TURNS`/`SHUTDOWN_FRAMES` →
    `FINAL_SPIRAL_*`.
  - `Ctx.writer: Writer` → `Ctx.mouse: MouseHid` (parallel to new
    `Ctx.kbd: KbdHid`); `send` → `send_mouse`.
- **`RUN_DURATION` from 8 hours to 3 h 50 m**. Math-optimal under the
  user's workday distribution (start `triangular(8.0, mode=8.5, 9.5)`,
  lunch 12:00–13:00, end `triangular(16.0, mode=17.5, 19.0)`) for "screen
  goes to sleep during the lunch hour on most days." See
  `/tmp/jiggly_runtime_v3.py` for the simulation.
- HID `poll_ms` 60 → 8 (125 Hz). At the previous 60 ms poll the host
  was discarding ~7 of every 8 animation frames the firmware emitted.
- Boot LED `R→G→B` step 180 ms → 60 ms; 1.5 s pre-animation USB-settle
  delay removed. Reset → wake-shake latency dropped from ~2 s to ~240 ms
  (mouse-shake now ~740 ms after reset since 500 ms of keyboard-wake taps
  precede it).
- Watchdog feeding moved out of the imperative main loop into its own
  `#[embassy_executor::task]`; the chart owns user-visible state, the
  watchdog task owns the periodic kick.

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
