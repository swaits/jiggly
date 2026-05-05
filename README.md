# jiggly

A USB mouse jiggler that keeps your screen awake during the workday and
goes quiet when you're done. Rust `no_std` firmware for the [Seeed
Studio Xiao RP2040][xiao], built on [embassy][embassy] and an
[hsmc][hsmc] statechart.

## What it does

Plugs into USB, presents as a composite mouse + keyboard HID device
(`1209:b0b0`, manufacturer `swaits.com`, product `jiggly`), and:

- On boot, taps `F13` four times, then jiggles the cursor — enough to
  wake any sleeping host. Mouse motion alone doesn't reliably wake
  macOS; a key tap does. F13 is chosen because it's harmless if it ever
  ends up stuck — no OS maps it by default.
- For the next 3 h 51 m, nudges the cursor one pixel every 4½ minutes
  so the host never falls asleep.
- Breathes the on-board NeoPixel green → yellow → red as time runs
  down. Two on-screen "spiral" warnings fire 10 min and 5 min before
  expiry.
- At end-of-life, draws a coin-spinning-down spiral on the cursor,
  blinks the LED red, and goes silent until you press `RESET`.

## Hardware

A Xiao RP2040. Nothing else — the on-board NeoPixel and USB-C are all
you need.

## Build & flash

The toolchain is pinned through [mise][mise], builds run through
[just][just]:

```
mise install        # one-time: rust toolchain, target, cargo helpers
just                # list available recipes
just release        # optimised build
just uf2            # produce a flashable .uf2
just flash          # build + copy to a mounted RPI-RP2 volume
just ci             # check + fmt-check + clippy (-D warnings) + release
```

To flash by hand: hold `B` (BOOT) and tap `R` (RESET) on the Xiao,
which mounts the `RPI-RP2` volume. Drop
`target/thumbv6m-none-eabi/release/jiggly.uf2` onto it.

## How it works

The whole device lifecycle is one [hsmc][hsmc] statechart:

```
Booting                                  LED R→G→B sweep
  └─ BootDone ──▶ WakingHost
                   ├─ WakingWithKeyboard  4× F13 tap, then idle
                   │    └─ (timeout) ──▶
                   └─ WakingWithMouse     ~12 Hz horizontal shake
                        └─ WakeDone ──▶ Settling (2 s pause)
                                          └─ ▶ Spinning  (3 quick circles)
                                               └─ SpinDone ──▶ Active
Active                                   green→yellow→red breathing
  ├─ every 4½ min: jiggle ±1 px          (Flash subtate paints the LED white)
  ├─ T-10 min: Warning10 mini-spiral
  ├─ T-5 min:  Warning5  mini-spiral
  └─ T-0:      ──▶ Ending                fast red blink
                    └─ Spiraling          full coin-down spiral
                         └─ SpiralDone ──▶ Quiet ──▶ PoweringDown
                                                       └─ 3 green flashes,
                                                          NeoPixel off,
                                                          USB silent
```

A separate embassy task feeds the hardware watchdog every 5 s.

## Why these timings, and why those LED thresholds?

The point isn't to keep the screen awake forever. It's to keep it
awake while you're at your desk and let it sleep when you're not. The
cleanest "not at desk" signal in a typical workday is **lunch**, so
the design goal is: most days the device should expire some time
during the noon hour, the screen locks, and one tap restarts the
cycle when you sit back down.

That's a four-knob problem:

| constant      | what it controls                                         |
|---------------|----------------------------------------------------------|
| `RUN_DURATION`| how long one full cycle lasts                            |
| `YELLOW_AT`   | minutes-remaining where breathing-yellow begins          |
| `RED_AT`      | minutes-remaining where breathing-red begins             |
| `FAST_RED_AT` | minutes-remaining where the fast-pulse-red blink begins  |

The `tuning/` crate solves it as a four-objective Pareto search
using [`heuropt`][heuropt] and NSGA-III:

1. **minimize work-time failures** (screen sleeps while the user is at their desk)
2. **maximize lunch sleep**
3. **minimize button presses**
4. **minimize after-hours waste** (screen still awake past clock-out)

The user model:

- Workday start is `Triangular(8:00, mode 8:30, 9:30)`, end is
  `Triangular(16:00, mode 17:30, 19:00)`. Lunch is fixed at 12:00–13:00.
- The user sees the LED and may tap `RESET` to extend the cycle:
  ~1.5 %/min during yellow, ~4 %/min during red, ~6 %/min during
  fast-red, plus small one-shot bumps the minute each spiral warning
  fires.
- Free `RESET` at boot and at 13:00 (re-login after lunch).

NSGA-III returns a Pareto front of ≈28 non-dominated points across
those four objectives — every one of them a legitimate tradeoff. To
pick a single recommendation the tuner applies explicit decision
weights (lunch_sleep 30 %, after_hours 25 %, work_fail 20 %, presses
15 %, balance 10 %) plus a press-count comfort cap. The pick — and
what the firmware ships:

```
RUN_DURATION = 3h51m   YELLOW_AT = 22   RED_AT = 11   FAST_RED_AT = 4
```

(LED thresholds are minutes-remaining.) Across 1 000 simulated
workdays this combination averages **26 minutes** of lunch sleep
and lands in the 12:15–12:45 sweet spot on **~57 %** of days, with
**zero** mean work-time failure and ~2 minutes/day of after-hours
waste at a cost of ~2.9 button presses/day.

The interesting result is that **shorter warning phases are better**.
A long yellow phase gives you 30 minutes to glance up, notice the
LED, and tap `RESET` out of an abundance of caution — and a tap
during yellow extends the cycle into the afternoon, the opposite of
the goal. The Pareto-front winner runs an 11-minute yellow, a 7-
minute red, and a 4-minute fast-red: long enough to register the
warning, short enough that the natural reaction is to wait it out.

If your day looks different — different start/end distribution,
different press habits, different lunch length — edit the model
constants in `tuning/src/main.rs`, run `just tune` (or `cargo run
--release` from inside `tuning/`), and update the four values in
`src/config.rs`.

## USB identity

The firmware enumerates as VID `1209` / PID `b0b0`, manufacturer
`swaits.com`, product `jiggly`. `1209` is the [pid.codes][pidcodes]
community VID for open-source projects.

The serial number is the RP2040's 64-bit unique chip ID rendered as 16
hex chars — different across boards, stable across replugs, so hosts
treat each plug as the same device they saw last time.

## License

MIT — see [LICENSE](LICENSE).

[xiao]:     https://wiki.seeedstudio.com/XIAO-RP2040/
[embassy]:  https://embassy.dev/
[hsmc]:     https://crates.io/crates/hsmc
[heuropt]:  https://crates.io/crates/heuropt
[mise]:     https://mise.jdx.dev/
[just]:     https://just.systems/
[pidcodes]: https://pid.codes/
