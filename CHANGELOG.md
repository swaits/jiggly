# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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
