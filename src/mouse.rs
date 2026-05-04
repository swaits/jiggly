//! Mouse HID: report send + the family of cursor animations
//! (wake-shake, spinner, eased spirals).

use core::f32::consts::PI;

use embassy_rp::clocks::RoscRng;
use embassy_time::{Duration as EDuration, Timer, with_timeout};
use embassy_usb::class::hid::HidWriter;
use libm::{cosf, powf, roundf, sinf};
use usbd_hid::descriptor::MouseReport;

use crate::chart::Ev;
use crate::config::{
    ANIM_FRAME, EASE_POW, FINAL_SPIRAL_FRAMES, FINAL_SPIRAL_RADIUS_START, FINAL_SPIRAL_TURNS,
    RUN_CIRCLES, RUN_FRAMES_PER_CIRCLE, RUN_RADIUS, SPIRAL_RADIUS_END, WAKE_AMPLITUDE,
    WAKE_FRAMES_PER_HALF, WAKE_JITTER, WAKE_OSCILLATIONS, WARN5_FRAMES, WARN5_RADIUS_START,
    WARN5_TURNS, WARN10_FRAMES, WARN10_RADIUS_START, WARN10_TURNS,
};
use crate::usb::UsbDriver;

pub(crate) type MouseHid = HidWriter<'static, UsbDriver, 5>;

// Frantic horizontal mouse shake. Plain oneshot helper — the action method
// that calls this races it against MOUSE_WAKE_DEADLINE.
pub(crate) async fn wake_with_mouse(mouse: &mut MouseHid) {
    let period_frames = (WAKE_FRAMES_PER_HALF * 2) as f32;
    let total_frames = WAKE_OSCILLATIONS * WAKE_FRAMES_PER_HALF * 2;
    let mut prev_x: f32 = 0.0;
    let mut prev_y: f32 = 0.0;
    let mut acc_x: f32 = 0.0;
    let mut acc_y: f32 = 0.0;
    for f in 0..total_frames {
        let phase = (f as f32) * 2.0 * PI / period_frames;
        let next_x = WAKE_AMPLITUDE * sinf(phase);
        let jitter = ((RoscRng::next_u8() as f32) / 255.0 - 0.5) * 2.0 * WAKE_JITTER;
        let next_y = jitter;
        let (dx, dy, used_x, used_y) = step_delta(prev_x, prev_y, next_x, next_y, acc_x, acc_y);
        acc_x = used_x;
        acc_y = used_y;
        send_mouse(mouse, dx, dy).await;
        prev_x = next_x;
        prev_y = next_y;
        Timer::after(ANIM_FRAME).await;
    }
}

pub(crate) async fn animate_spinner(mouse: &mut MouseHid) -> Ev {
    let total_frames = RUN_CIRCLES * RUN_FRAMES_PER_CIRCLE;
    let period_frames = RUN_FRAMES_PER_CIRCLE as f32;
    let mut prev_x: f32 = 0.0;
    let mut prev_y: f32 = 0.0;
    let mut acc_x: f32 = 0.0;
    let mut acc_y: f32 = 0.0;
    for f in 0..total_frames {
        let angle = (f as f32) * 2.0 * PI / period_frames;
        let next_x = RUN_RADIUS * sinf(angle);
        let next_y = RUN_RADIUS * (1.0 - cosf(angle));
        let (dx, dy, used_x, used_y) = step_delta(prev_x, prev_y, next_x, next_y, acc_x, acc_y);
        acc_x = used_x;
        acc_y = used_y;
        send_mouse(mouse, dx, dy).await;
        prev_x = next_x;
        prev_y = next_y;
        Timer::after(ANIM_FRAME).await;
    }
    Ev::SpinDone
}

async fn animate_spiral(
    mouse: &mut MouseHid,
    radius_start: f32,
    radius_end: f32,
    turns: f32,
    frames: u32,
) {
    let mut prev_x: f32 = 0.0;
    let mut prev_y: f32 = 0.0;
    let mut acc_x: f32 = 0.0;
    let mut acc_y: f32 = 0.0;
    for f in 0..frames {
        let t = (f as f32) / (frames as f32);
        let u = powf(t, EASE_POW);
        let angle = u * 2.0 * PI * turns;
        let radius = radius_start + (radius_end - radius_start) * u;
        // Subtract starting offset so the spiral begins at the cursor's entry point.
        let next_x = radius * cosf(angle) - radius_start;
        let next_y = radius * sinf(angle);
        let (dx, dy, used_x, used_y) = step_delta(prev_x, prev_y, next_x, next_y, acc_x, acc_y);
        acc_x = used_x;
        acc_y = used_y;
        send_mouse(mouse, dx, dy).await;
        prev_x = next_x;
        prev_y = next_y;
        Timer::after(ANIM_FRAME).await;
    }
}

pub(crate) async fn animate_final_spiral(mouse: &mut MouseHid) -> Ev {
    animate_spiral(
        mouse,
        FINAL_SPIRAL_RADIUS_START,
        SPIRAL_RADIUS_END,
        FINAL_SPIRAL_TURNS,
        FINAL_SPIRAL_FRAMES,
    )
    .await;
    Ev::SpiralDone
}

pub(crate) async fn animate_warning_5(mouse: &mut MouseHid) -> Ev {
    animate_spiral(
        mouse,
        WARN5_RADIUS_START,
        SPIRAL_RADIUS_END,
        WARN5_TURNS,
        WARN5_FRAMES,
    )
    .await;
    Ev::WarnDone
}

pub(crate) async fn animate_warning_10(mouse: &mut MouseHid) -> Ev {
    animate_spiral(
        mouse,
        WARN10_RADIUS_START,
        SPIRAL_RADIUS_END,
        WARN10_TURNS,
        WARN10_FRAMES,
    )
    .await;
    Ev::WarnDone
}

// Translate continuous (next_x, next_y) target into clamped i8 deltas while
// carrying sub-pixel residue forward, so a 90-frame circle of radius 50 doesn't
// lose ~half its motion to truncation. Returns (dx, dy, residual_x, residual_y).
fn step_delta(
    prev_x: f32,
    prev_y: f32,
    next_x: f32,
    next_y: f32,
    acc_x: f32,
    acc_y: f32,
) -> (i8, i8, f32, f32) {
    let want_x = (next_x - prev_x) + acc_x;
    let want_y = (next_y - prev_y) + acc_y;
    let dx = roundf(want_x.clamp(-127.0, 127.0)) as i8;
    let dy = roundf(want_y.clamp(-127.0, 127.0)) as i8;
    (dx, dy, want_x - dx as f32, want_y - dy as f32)
}

// ── HID primitive ──────────────────────────────────────────────────

pub(crate) async fn send_mouse(mouse: &mut MouseHid, x: i8, y: i8) {
    let report = MouseReport {
        buttons: 0,
        x,
        y,
        wheel: 0,
        pan: 0,
    };
    let _ = with_timeout(EDuration::from_secs(3), mouse.write_serialize(&report)).await;
}
