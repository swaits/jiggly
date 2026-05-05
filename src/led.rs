//! WS2812 NeoPixel: paint primitive, breath/blink math, and the
//! long-running `during` activities the statechart drives during
//! Booting / WakingHost / Settling / Spinning / Active / Ending.

use core::f32::consts::PI;

use embassy_rp::{
    gpio::Output,
    peripherals::PIO0,
    pio_programs::ws2812::{Grb, PioWs2812},
};
use embassy_time::{Duration as EDuration, Instant, Timer};
use libm::cosf;
use smart_leds::RGB8;

use crate::chart::Ev;
use crate::config::{
    BOOT_SWEEP_STEP, BREATHE_FLOOR, BREATHE_PEAK, FAST_RED_AT, FAST_RED_PERIOD, LED_TICK, RED_AT,
    RED_PERIOD, RUN_BEFORE_SHUTDOWN, SETTLING_PULSE_PERIOD, SLOW_GREEN_PERIOD,
    SPINNER_FADE_DURATION, WAKING_PULSE_PERIOD, YELLOW_AT, YELLOW_PERIOD,
};

pub(crate) type Neo = PioWs2812<'static, PIO0, 0, 1, Grb>;

// ── During activities (free async fns) ─────────────────────────────

pub(crate) async fn boot_sweep(neo: &mut Neo, neo_pwr: &mut Output<'static>) -> Ev {
    neo_pwr.set_high();
    Timer::after_millis(2).await;

    paint(neo, BREATHE_PEAK, 0, 0).await;
    Timer::after(BOOT_SWEEP_STEP).await;
    paint(neo, 0, BREATHE_PEAK, 0).await;
    Timer::after(BOOT_SWEEP_STEP).await;
    paint(neo, 0, 0, BREATHE_PEAK).await;
    Timer::after(BOOT_SWEEP_STEP).await;
    paint(neo, 0, 0, 0).await;
    Ev::BootDone
}

pub(crate) async fn breathe_color(neo: &mut Neo, active_start: &mut Option<Instant>) -> Ev {
    loop {
        let elapsed = active_start.map(|s| s.elapsed()).unwrap_or_default();
        let total_run = run_before_shutdown_e();
        let remaining = total_run
            .checked_sub(elapsed)
            .unwrap_or(EDuration::from_ticks(0));
        let (r, g, b) = breathe_for(remaining, elapsed);
        paint(neo, r, g, b).await;
        Timer::after(LED_TICK).await;
    }
}

pub(crate) async fn blink_fast_red(neo: &mut Neo) -> Ev {
    let start = Instant::now();
    loop {
        let t = start.elapsed();
        let level = if blink_on(FAST_RED_PERIOD, t) {
            BREATHE_PEAK
        } else {
            BREATHE_FLOOR
        };
        paint(neo, level, 0, 0).await;
        Timer::after(LED_TICK).await;
    }
}

pub(crate) async fn pulse_blue(neo: &mut Neo) -> Ev {
    let start = Instant::now();
    loop {
        let level = sin_breath(
            WAKING_PULSE_PERIOD,
            start.elapsed(),
            BREATHE_FLOOR,
            BREATHE_PEAK,
        );
        paint(neo, 0, 0, level).await;
        Timer::after(LED_TICK).await;
    }
}

pub(crate) async fn pulse_white(neo: &mut Neo) -> Ev {
    let start = Instant::now();
    loop {
        let level = sin_breath(
            SETTLING_PULSE_PERIOD,
            start.elapsed(),
            BREATHE_FLOOR,
            BREATHE_PEAK,
        );
        paint(neo, level, level, level).await;
        Timer::after(LED_TICK).await;
    }
}

// Linear fade from white(peak) → green(peak) so the LED hands off into
// Active's green breathing without a visible jump.
pub(crate) async fn fade_to_green(neo: &mut Neo) -> Ev {
    let start = Instant::now();
    let total_ms = SPINNER_FADE_DURATION.as_millis() as f32;
    loop {
        let t = ((start.elapsed().as_millis() as f32) / total_ms).min(1.0);
        let rb = (BREATHE_PEAK as f32 * (1.0 - t)) as u8;
        paint(neo, rb, BREATHE_PEAK, rb).await;
        Timer::after(LED_TICK).await;
    }
}

// ── Color / breathing helpers ──────────────────────────────────────

fn run_before_shutdown_e() -> EDuration {
    // RUN_BEFORE_SHUTDOWN is a `core::time::Duration`; convert to the embassy
    // type once at the call site so the comparator below is apples-to-apples.
    EDuration::from_secs(RUN_BEFORE_SHUTDOWN.as_secs())
}

fn breathe_for(remaining: EDuration, elapsed: EDuration) -> (u8, u8, u8) {
    if remaining > YELLOW_AT {
        let level = sin_breath(SLOW_GREEN_PERIOD, elapsed, BREATHE_FLOOR, BREATHE_PEAK);
        (0, level, 0)
    } else if remaining > RED_AT {
        // Green is perceptually brighter on WS2812 — scale it down for a warm yellow.
        let level = sin_breath(YELLOW_PERIOD, elapsed, BREATHE_FLOOR, BREATHE_PEAK);
        (level, ((level as u16 * 5) / 10) as u8, 0)
    } else if remaining > FAST_RED_AT {
        let level = sin_breath(RED_PERIOD, elapsed, BREATHE_FLOOR, BREATHE_PEAK);
        (level, 0, 0)
    } else {
        let level = if blink_on(FAST_RED_PERIOD, elapsed) {
            BREATHE_PEAK
        } else {
            BREATHE_FLOOR
        };
        (level, 0, 0)
    }
}

fn sin_breath(period: EDuration, t: EDuration, floor: u8, peak: u8) -> u8 {
    let period_ms = period.as_millis() as f32;
    let t_ms = (t.as_millis() % period.as_millis()) as f32;
    let phase = 2.0 * PI * t_ms / period_ms;
    let val = (1.0 - cosf(phase)) * 0.5;
    let span = peak.saturating_sub(floor) as f32;
    floor + (val * span) as u8
}

fn blink_on(period: EDuration, t: EDuration) -> bool {
    let period_ms = period.as_millis();
    (t.as_millis() % period_ms) < (period_ms / 2)
}

// ── LED primitive ──────────────────────────────────────────────────

pub(crate) async fn paint(neo: &mut Neo, r: u8, g: u8, b: u8) {
    neo.write(&[RGB8 { r, g, b }]).await;
}
