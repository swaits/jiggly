#![no_std]
#![no_main]

use core::f32::consts::PI;

use embassy_executor::Spawner;
use embassy_rp::{
    bind_interrupts,
    clocks::RoscRng,
    dma,
    gpio::{Level, Output},
    peripherals::{DMA_CH0, PIO0, USB},
    pio::{InterruptHandler as PioInterruptHandler, Pio},
    pio_programs::ws2812::{Grb, PioWs2812, PioWs2812Program},
    usb::{Driver, InterruptHandler as UsbInterruptHandler},
    watchdog::Watchdog,
};
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, channel::Channel};
use embassy_time::{Duration as EDuration, Instant, Timer, with_timeout};
use embassy_usb::{
    Builder, Config as UsbConfig, UsbDevice,
    class::hid::{Config as HidConfig, HidBootProtocol, HidSubclass, HidWriter, State},
};
use hsmc::{Duration, statechart};
use libm::{cosf, roundf, sinf};
use panic_reset as _;
use smart_leds::RGB8;
use static_cell::StaticCell;
use usbd_hid::descriptor::{MouseReport, SerializedDescriptor};

bind_interrupts!(struct Irqs {
    USBCTRL_IRQ => UsbInterruptHandler<USB>;
    PIO0_IRQ_0 => PioInterruptHandler<PIO0>;
    DMA_IRQ_0 => dma::InterruptHandler<DMA_CH0>;
});

type UsbDriver = Driver<'static, USB>;
type Neo = PioWs2812<'static, PIO0, 0, 1, Grb>;
type Writer = HidWriter<'static, UsbDriver, 5>;

// ── Lifecycle timing (statechart Durations) ────────────────────────
// 3h50m: math-optimal under the user's schedule for "screen sleeps during
// lunch on most days". Balances early-start re-tap risk against late-start
// alive-through-lunch risk. See /tmp/jiggly_runtime_v3.py.
const RUN_DURATION: Duration = Duration::from_hours(3).saturating_add(Duration::from_mins(50));
const SHUTDOWN_LEAD: Duration = Duration::from_secs(30);
const RUN_BEFORE_SHUTDOWN: Duration = RUN_DURATION.saturating_sub(SHUTDOWN_LEAD);
const SHUTDOWN_ANIM_BUDGET: Duration = Duration::from_secs(2);
const QUIET_AFTER_ANIM: Duration = SHUTDOWN_LEAD.saturating_sub(SHUTDOWN_ANIM_BUDGET);
const JIGGLE_PERIOD: Duration = Duration::from_secs(270);
const FLASH_DURATION: Duration = Duration::from_millis(100);

// Phase boundaries — compared against time *remaining* in Active.
const YELLOW_AT: EDuration = EDuration::from_secs(60 * 60);
const RED_AT: EDuration = EDuration::from_secs(30 * 60);
const FAST_RED_AT: EDuration = EDuration::from_secs(10 * 60);

// LED breathing math
const LED_TICK: EDuration = EDuration::from_millis(20);
const SLOW_GREEN_PERIOD: EDuration = EDuration::from_secs(4);
const YELLOW_PERIOD: EDuration = EDuration::from_secs(3);
const RED_PERIOD: EDuration = EDuration::from_secs(2);
const FAST_RED_PERIOD: EDuration = EDuration::from_millis(500);

// LED brightness (raw WS2812 PWM, 0..=255).
const BREATHE_FLOOR: u8 = 1;
const BREATHE_PEAK: u8 = 16;
const FLASH_PEAK: u8 = 160;

// ── Animation parameters ───────────────────────────────────────────
// 8ms frames + HID poll_ms=8 means each frame's report actually reaches the
// host instead of being coalesced — at 16ms frames against a 60ms poll the
// shake looked sluggish because three reports out of four were dropped on the
// floor.
const ANIM_FRAME: EDuration = EDuration::from_millis(8);

// Frantic side-to-side, the gesture a person makes to wake a sleeping display.
// 10 full oscillations × 8 frames of full period = 80 frames × 8 ms = 640 ms,
// which works out to a ~12 Hz alternation — visibly "shaking", not "sweeping".
const WAKE_OSCILLATIONS: u32 = 10;
const WAKE_FRAMES_PER_HALF: u32 = 4;
const WAKE_AMPLITUDE: f32 = 60.0;
const WAKE_JITTER: f32 = 1.0;

// 2 s pause after the shake so the display has time to actually wake before
// we draw the "running" circles. The user wants this delay to live *here*,
// not before the shake.
const SETTLING_DELAY: Duration = Duration::from_secs(2);

// Three quick clockwise circles read more clearly as "spinner / running"
// than one slow lap. 25 frames per circle × 3 × 8 ms = 600 ms total.
const RUN_RADIUS: f32 = 40.0;
const RUN_FRAMES_PER_CIRCLE: u32 = 25;
const RUN_CIRCLES: u32 = 3;

const SHUTDOWN_RADIUS_START: f32 = 50.0;
const SHUTDOWN_RADIUS_END: f32 = 5.0;
const SHUTDOWN_TURNS: u32 = 3;
const SHUTDOWN_FRAMES: u32 = 120;

// ── Watchdog (independent task) ────────────────────────────────────
const WATCHDOG_TIMEOUT: EDuration = EDuration::from_secs(8);
const WATCHDOG_FEED_INTERVAL: EDuration = EDuration::from_secs(5);

// ── Jiggle dwell ───────────────────────────────────────────────────
const PIXEL_DWELL: EDuration = EDuration::from_millis(25);

// ── Boot LED sweep ─────────────────────────────────────────────────
// Brief power-on confirmation. Kept short so the cursor shake — the actual
// "wake the display" gesture — happens promptly after reset.
const BOOT_SWEEP_STEP: EDuration = EDuration::from_millis(60);

// ── Shutdown LED flashes ───────────────────────────────────────────
const SHUTDOWN_FLASH_STEP: EDuration = EDuration::from_millis(400);

pub struct Ctx {
    pub writer: Writer,
    pub neo: Neo,
    pub neo_pwr: Output<'static>,
    pub active_start: Option<Instant>,
}

#[derive(Debug, Clone)]
pub enum Ev {
    SweepDone,
    WakeDone,
    RunDone,
    Jiggled,
    ShutdownAnimDone,
}

statechart! {
Jiggly {
    context: Ctx;
    events: Ev;
    default(BootSweep);

    state BootSweep {
        during: led_rgb_sweep(neo, neo_pwr);
        on(SweepDone) => WakingDisplay;
    }

    state WakingDisplay {
        during: animate_wake(writer);
        on(WakeDone) => Settling;
    }

    // Quiet pause so the display has time to come out of sleep before the
    // cursor starts drawing the "running" circles.
    state Settling {
        on(after SETTLING_DELAY) => ShowingRunning;
    }

    state ShowingRunning {
        during: animate_running(writer);
        on(RunDone) => Active;
    }

    state Active {
        entry: capture_active_start;
        on(every JIGGLE_PERIOD) => jiggle_pair;
        on(after RUN_BEFORE_SHUTDOWN) => Ending;
        default(Breathing);

        state Breathing {
            during: breathe_color(neo, active_start);
            on(Jiggled) => Flashing;
        }

        state Flashing {
            entry: paint_white;
            on(after FLASH_DURATION) => Breathing;
        }
    }

    state Ending {
        during: blink_fast_red(neo);
        default(ShutdownAnim);

        state ShutdownAnim {
            during: animate_shutdown(writer);
            on(ShutdownAnimDone) => Quiet;
        }

        state Quiet {
            on(after QUIET_AFTER_ANIM) => PoweringDown;
        }
    }

    state PoweringDown {
        entry: shutdown_flashes;
        entry: power_off_neo;
    }
}
}

impl JigglyActions for JigglyActionContext<'_> {
    async fn capture_active_start(&mut self) {
        self.active_start = Some(Instant::now());
    }

    async fn jiggle_pair(&mut self) {
        let (dx, dy): (i8, i8) = if (RoscRng::next_u8() & 1) == 0 {
            (1, 0)
        } else {
            (0, 1)
        };
        send(&mut self.writer, dx, dy).await;
        Timer::after(PIXEL_DWELL).await;
        send(&mut self.writer, -dx, -dy).await;
        let _ = self.emit(Ev::Jiggled);
    }

    async fn paint_white(&mut self) {
        paint(&mut self.neo, FLASH_PEAK, FLASH_PEAK, FLASH_PEAK).await;
    }

    async fn shutdown_flashes(&mut self) {
        for _ in 0..3 {
            paint(&mut self.neo, 0, BREATHE_PEAK, 0).await;
            Timer::after(SHUTDOWN_FLASH_STEP).await;
            paint(&mut self.neo, 0, 0, 0).await;
            Timer::after(SHUTDOWN_FLASH_STEP).await;
        }
    }

    async fn power_off_neo(&mut self) {
        self.neo_pwr.set_low();
    }
}

// ── During activities (free async fns) ─────────────────────────────

async fn led_rgb_sweep(neo: &mut Neo, neo_pwr: &mut Output<'static>) -> Ev {
    neo_pwr.set_high();
    Timer::after_millis(2).await;

    paint(neo, BREATHE_PEAK, 0, 0).await;
    Timer::after(BOOT_SWEEP_STEP).await;
    paint(neo, 0, BREATHE_PEAK, 0).await;
    Timer::after(BOOT_SWEEP_STEP).await;
    paint(neo, 0, 0, BREATHE_PEAK).await;
    Timer::after(BOOT_SWEEP_STEP).await;
    paint(neo, 0, 0, 0).await;
    Ev::SweepDone
}

async fn animate_wake(writer: &mut Writer) -> Ev {
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
        send(writer, dx, dy).await;
        prev_x = next_x;
        prev_y = next_y;
        Timer::after(ANIM_FRAME).await;
    }
    Ev::WakeDone
}

async fn animate_running(writer: &mut Writer) -> Ev {
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
        send(writer, dx, dy).await;
        prev_x = next_x;
        prev_y = next_y;
        Timer::after(ANIM_FRAME).await;
    }
    Ev::RunDone
}

async fn animate_shutdown(writer: &mut Writer) -> Ev {
    let mut prev_x: f32 = 0.0;
    let mut prev_y: f32 = 0.0;
    let mut acc_x: f32 = 0.0;
    let mut acc_y: f32 = 0.0;
    for f in 0..SHUTDOWN_FRAMES {
        let t = (f as f32) / (SHUTDOWN_FRAMES as f32);
        let angle = t * 2.0 * PI * (SHUTDOWN_TURNS as f32);
        let radius = SHUTDOWN_RADIUS_START + (SHUTDOWN_RADIUS_END - SHUTDOWN_RADIUS_START) * t;
        // Subtract starting offset so the spiral begins at the cursor's entry point.
        let next_x = radius * cosf(angle) - SHUTDOWN_RADIUS_START;
        let next_y = radius * sinf(angle);
        let (dx, dy, used_x, used_y) = step_delta(prev_x, prev_y, next_x, next_y, acc_x, acc_y);
        acc_x = used_x;
        acc_y = used_y;
        send(writer, dx, dy).await;
        prev_x = next_x;
        prev_y = next_y;
        Timer::after(ANIM_FRAME).await;
    }
    Ev::ShutdownAnimDone
}

async fn breathe_color(neo: &mut Neo, active_start: &mut Option<Instant>) -> Ev {
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

async fn blink_fast_red(neo: &mut Neo) -> Ev {
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

// ── HID + LED primitives ───────────────────────────────────────────

async fn send(writer: &mut Writer, x: i8, y: i8) {
    let report = MouseReport {
        buttons: 0,
        x,
        y,
        wheel: 0,
        pan: 0,
    };
    let _ = with_timeout(EDuration::from_secs(3), writer.write_serialize(&report)).await;
}

async fn paint(neo: &mut Neo, r: u8, g: u8, b: u8) {
    neo.write(&[RGB8 { r, g, b }]).await;
}

// ── Tasks ──────────────────────────────────────────────────────────

#[embassy_executor::task]
async fn usb_task(mut device: UsbDevice<'static, UsbDriver>) {
    device.run().await;
}

#[embassy_executor::task]
async fn watchdog_task(mut wd: Watchdog) -> ! {
    loop {
        wd.feed(WATCHDOG_TIMEOUT);
        Timer::after(WATCHDOG_FEED_INTERVAL).await;
    }
}

// ── Main ───────────────────────────────────────────────────────────

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let p = embassy_rp::init(Default::default());

    let mut watchdog = Watchdog::new(p.WATCHDOG);
    watchdog.start(WATCHDOG_TIMEOUT);

    // NeoPixel: GPIO11 powers it, GPIO12 is the WS2812 data line driven from
    // PIO0 + DMA_CH0. The user RGB on GPIO16/17/25 stays floating so those
    // LEDs remain dark.
    let neo_pwr = Output::new(p.PIN_11, Level::Low);
    let mut pio = Pio::new(p.PIO0, Irqs);
    let neo_program = PioWs2812Program::new(&mut pio.common);
    let neo: Neo = PioWs2812::new(
        &mut pio.common,
        pio.sm0,
        p.DMA_CH0,
        Irqs,
        p.PIN_12,
        &neo_program,
    );

    let driver = Driver::new(p.USB, Irqs);

    let mut config = UsbConfig::new(0x046d, 0xc07d);
    config.manufacturer = Some("Logitech");
    config.product = Some("G502 Mouse");
    config.max_power = 100;
    config.max_packet_size_0 = 64;

    static CONFIG_DESCRIPTOR: StaticCell<[u8; 256]> = StaticCell::new();
    static BOS_DESCRIPTOR: StaticCell<[u8; 256]> = StaticCell::new();
    static MSOS_DESCRIPTOR: StaticCell<[u8; 256]> = StaticCell::new();
    static CONTROL_BUF: StaticCell<[u8; 64]> = StaticCell::new();
    static HID_STATE: StaticCell<State> = StaticCell::new();

    let mut builder = Builder::new(
        driver,
        config,
        CONFIG_DESCRIPTOR.init([0; 256]),
        BOS_DESCRIPTOR.init([0; 256]),
        MSOS_DESCRIPTOR.init([0; 256]),
        CONTROL_BUF.init([0; 64]),
    );

    let hid_config = HidConfig {
        report_descriptor: MouseReport::desc(),
        request_handler: None,
        // 8 ms (125 Hz) — standard for full-speed mice. At the previous 60 ms
        // the host was throwing away ~7 of every 8 animation frames we sent.
        poll_ms: 8,
        max_packet_size: 8,
        hid_subclass: HidSubclass::Boot,
        hid_boot_protocol: HidBootProtocol::Mouse,
    };
    let writer = HidWriter::<_, 5>::new(&mut builder, HID_STATE.init(State::new()), hid_config);

    let usb = builder.build();
    spawner.spawn(usb_task(usb).unwrap());
    spawner.spawn(watchdog_task(watchdog).unwrap());

    static EVENT_CHAN: Channel<CriticalSectionRawMutex, Ev, 8> = Channel::new();

    let ctx = Ctx {
        writer,
        neo,
        neo_pwr,
        active_start: None,
    };
    let mut chart = Jiggly::new(ctx, &EVENT_CHAN);
    let _ = chart.run().await;

    // Unreachable in practice — the chart parks in PoweringDown forever and
    // run() never returns. The watchdog task keeps the chip alive.
    loop {
        Timer::after(WATCHDOG_FEED_INTERVAL).await;
    }
}
