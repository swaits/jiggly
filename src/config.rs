//! Compile-time tuning constants — timing, geometry, brightness.
//!
//! Every magic number lives here so the rest of the firmware reads as
//! pure behaviour. Several values are joint-tuned by the `tuning/` crate
//! — see the README's "Why these timings…" section for the rationale.

use embassy_time::Duration as EDuration;
use hsmc::Duration;

// ── Lifecycle timing (statechart Durations) ────────────────────────
// 3h51m: a-posteriori pick from a 4-objective NSGA-III Pareto search
// over (RUN_DURATION, YELLOW_AT, RED_AT, FAST_RED_AT) — minimize work-
// time failures, maximize lunch sleep, minimize button presses, minimize
// after-hours waste. See the README's "Why these timings?" section and
// `tuning/` for the search.
pub(crate) const RUN_DURATION: Duration = Duration::from_mins(3 * 60 + 51);
pub(crate) const SHUTDOWN_LEAD: Duration = Duration::from_secs(30);
pub(crate) const RUN_BEFORE_SHUTDOWN: Duration = RUN_DURATION.saturating_sub(SHUTDOWN_LEAD);
pub(crate) const SHUTDOWN_ANIM_BUDGET: Duration = Duration::from_secs(5);
pub(crate) const QUIET_AFTER_ANIM: Duration = SHUTDOWN_LEAD.saturating_sub(SHUTDOWN_ANIM_BUDGET);
pub(crate) const JIGGLE_PERIOD: Duration = Duration::from_secs(270);
pub(crate) const FLASH_DURATION: Duration = Duration::from_millis(100);

// Phase boundaries — compared against time *remaining* in Active.
// Joint pick from the NSGA-III Pareto search; see the README's
// "Why these timings…" section for the rationale.
pub(crate) const YELLOW_AT: EDuration = EDuration::from_secs(22 * 60);
pub(crate) const RED_AT: EDuration = EDuration::from_secs(11 * 60);
pub(crate) const FAST_RED_AT: EDuration = EDuration::from_secs(4 * 60);

// LED breathing math
pub(crate) const LED_TICK: EDuration = EDuration::from_millis(20);
pub(crate) const SLOW_GREEN_PERIOD: EDuration = EDuration::from_secs(4);
pub(crate) const YELLOW_PERIOD: EDuration = EDuration::from_secs(3);
pub(crate) const RED_PERIOD: EDuration = EDuration::from_secs(2);
pub(crate) const FAST_RED_PERIOD: EDuration = EDuration::from_millis(500);

// Wake-up LED feedback periods. Blue on WakingHost (kbd+mouse), white on
// Settling, then a linear white→green fade across the Spinning duration so
// the LED hands off cleanly to Active's green breathing.
pub(crate) const WAKING_PULSE_PERIOD: EDuration = EDuration::from_millis(800);
pub(crate) const SETTLING_PULSE_PERIOD: EDuration = EDuration::from_millis(1200);
pub(crate) const SPINNER_FADE_DURATION: EDuration = EDuration::from_millis(600);

// LED brightness (raw WS2812 PWM, 0..=255).
pub(crate) const BREATHE_FLOOR: u8 = 1;
pub(crate) const BREATHE_PEAK: u8 = 16;
pub(crate) const FLASH_PEAK: u8 = 160;

// ── Animation parameters ───────────────────────────────────────────
// 8ms frames + HID poll_ms=8 means each frame's report actually reaches the
// host instead of being coalesced — at 16ms frames against a 60ms poll the
// shake looked sluggish because three reports out of four were dropped on the
// floor.
pub(crate) const ANIM_FRAME: EDuration = EDuration::from_millis(8);

// Frantic side-to-side, the gesture a person makes to wake a sleeping display.
// 10 full oscillations × 8 frames of full period = 80 frames × 8 ms = 640 ms,
// which works out to a ~12 Hz alternation — visibly "shaking", not "sweeping".
pub(crate) const WAKE_OSCILLATIONS: u32 = 10;
pub(crate) const WAKE_FRAMES_PER_HALF: u32 = 4;
pub(crate) const WAKE_AMPLITUDE: f32 = 60.0;
pub(crate) const WAKE_JITTER: f32 = 1.0;

// 2 s pause after the shake so the display has time to actually wake before
// we draw the spinner. The user wants this delay to live *here*, not before
// the shake.
pub(crate) const SETTLING_DELAY: Duration = Duration::from_secs(2);

// ── Keyboard wake (host-wake first pass) ───────────────────────────
// macOS often won't wake from raw HID mouse motion alone, but reliably wakes
// from any keyboard event. We tap **F13** four times before the mouse shake.
// F13–F24 are intentionally unmapped on every mainstream OS, so even in the
// nightmare scenario where the deadline preempts the loop *between* a key-
// down and key-up report and the host ends up holding F13 forever, nothing
// visible happens — unlike with Shift, which would silently capitalise every
// keystroke from the user's real keyboard until they unplug the device.
// Earlier versions used Left Shift; that turned out to be exactly that
// nightmare scenario in practice.
pub(crate) const KBD_WAKE_TAPS: u32 = 4;
pub(crate) const KBD_TAP_HOLD: EDuration = EDuration::from_millis(30);
pub(crate) const KBD_TAP_GAP: EDuration = EDuration::from_millis(50);
// Hard internal deadline on the keyboard-wake entry action. The taps total
// ~320 ms so they finish well before this; the deadline only kicks in if a
// USB write blocks (e.g. the host hasn't bound the keyboard endpoint yet).
pub(crate) const KBD_WAKE_DEADLINE: EDuration = EDuration::from_millis(500);
// Statechart timer for the WakingWithKeyboard state — chosen above the
// internal deadline so the chart timer is what drives the transition out.
pub(crate) const KBD_PHASE_DURATION: Duration = Duration::from_millis(550);
// HID Keyboard usage page keycode for F13.
pub(crate) const KBD_KEY_F13: u8 = 0x68;
// Final-cleanup deadline — the all-keys-released report we send after the
// main work loop is bounded by this so a misbehaving endpoint can't pin
// the chart. Best-effort; if it doesn't land we tried.
pub(crate) const KBD_RELEASE_DEADLINE: EDuration = EDuration::from_millis(100);

// ── Mouse wake (host-wake second pass) ─────────────────────────────
// Internal deadline on the mouse-shake entry action — the shake itself takes
// ~640 ms; the cap exists so a misbehaving USB endpoint can't pin the chart.
pub(crate) const MOUSE_WAKE_DEADLINE: EDuration = EDuration::from_millis(1000);
pub(crate) const MOUSE_PHASE_DURATION: Duration = Duration::from_millis(1050);

// Three quick clockwise circles read more clearly as "spinner / running"
// than one slow lap. 25 frames per circle × 3 × 8 ms = 600 ms total.
pub(crate) const RUN_RADIUS: f32 = 40.0;
pub(crate) const RUN_FRAMES_PER_CIRCLE: u32 = 25;
pub(crate) const RUN_CIRCLES: u32 = 3;

// Shared "spin-down" spiral. Both radius and angle are driven by an eased phase
// u(t) = t^EASE_POW with EASE_POW > 1 — slow at the start, ~EASE_POW× the
// average rate at the finish. Because radius and angle share u, the inward
// spiral and the rotation accelerate together: a coin/Euler-disk feel.
pub(crate) const EASE_POW: f32 = 2.5;
pub(crate) const SPIRAL_RADIUS_END: f32 = 2.0;

// Final spiral — the dramatic full version, 30 s before USB goes silent.
pub(crate) const FINAL_SPIRAL_RADIUS_START: f32 = 80.0;
pub(crate) const FINAL_SPIRAL_TURNS: f32 = 5.0;
pub(crate) const FINAL_SPIRAL_FRAMES: u32 = 625; // 5.0 s @ 8 ms/frame

// 5-min warning — medium escalation.
pub(crate) const WARN5_RADIUS_START: f32 = 50.0;
pub(crate) const WARN5_TURNS: f32 = 3.0;
pub(crate) const WARN5_FRAMES: u32 = 312; // ~2.5 s

// 10-min warning — small foreshadow.
pub(crate) const WARN10_RADIUS_START: f32 = 30.0;
pub(crate) const WARN10_TURNS: f32 = 2.0;
pub(crate) const WARN10_FRAMES: u32 = 187; // ~1.5 s

// Offsets from Active-entry. The hsmc parent timer rule: timers in a parent
// state start on parent entry and survive sibling-substate transitions, so
// these three `after`s race concurrently against the same epoch.
pub(crate) const WARN_10_AT: Duration = RUN_BEFORE_SHUTDOWN.saturating_sub(Duration::from_mins(10));
pub(crate) const WARN_5_AT: Duration = RUN_BEFORE_SHUTDOWN.saturating_sub(Duration::from_mins(5));

// ── Watchdog (independent task) ────────────────────────────────────
pub(crate) const WATCHDOG_TIMEOUT: EDuration = EDuration::from_secs(8);
pub(crate) const WATCHDOG_FEED_INTERVAL: EDuration = EDuration::from_secs(5);

// ── Jiggle dwell ───────────────────────────────────────────────────
pub(crate) const PIXEL_DWELL: EDuration = EDuration::from_millis(25);

// ── Boot LED sweep ─────────────────────────────────────────────────
// Brief power-on confirmation. Kept short so the cursor shake — the actual
// "wake the display" gesture — happens promptly after reset.
pub(crate) const BOOT_SWEEP_STEP: EDuration = EDuration::from_millis(60);

// ── Shutdown LED flashes ───────────────────────────────────────────
pub(crate) const SHUTDOWN_FLASH_STEP: EDuration = EDuration::from_millis(400);
