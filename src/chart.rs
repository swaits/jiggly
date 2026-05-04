//! The Jiggly hierarchical state machine: context, event enum, the
//! `statechart!` definition itself, and the entry-action impls
//! (the `during:` activities live in the `led` and `mouse` modules).

use embassy_rp::{clocks::RoscRng, gpio::Output};
use embassy_time::{Instant, Timer};
use hsmc::statechart;

use crate::config::{
    BREATHE_PEAK, FLASH_DURATION, FLASH_PEAK, JIGGLE_PERIOD, KBD_PHASE_DURATION,
    KBD_RELEASE_DEADLINE, KBD_WAKE_DEADLINE, MOUSE_PHASE_DURATION, MOUSE_WAKE_DEADLINE,
    PIXEL_DWELL, QUIET_AFTER_ANIM, RUN_BEFORE_SHUTDOWN, SETTLING_DELAY, SHUTDOWN_FLASH_STEP,
    WARN_5_AT, WARN_10_AT,
};
use crate::kbd::{KbdHid, send_kbd, wake_with_keyboard};
use crate::led::{
    Neo, blink_fast_red, boot_sweep, breathe_color, fade_to_green, paint, pulse_blue,
    pulse_white,
};
use crate::mouse::{
    MouseHid, animate_final_spiral, animate_spinner, animate_warning_5, animate_warning_10,
    send_mouse, wake_with_mouse,
};

pub struct Ctx {
    pub mouse: MouseHid,
    pub kbd: KbdHid,
    pub neo: Neo,
    pub neo_pwr: Output<'static>,
    pub active_start: Option<Instant>,
}

#[derive(Debug, Clone)]
pub enum Ev {
    BootDone,
    SpinDone,
    Jiggled,
    WarnDone,
    SpiralDone,
}

statechart! {
Jiggly {
    context: Ctx;
    events: Ev;
    default(Booting);

    state Booting {
        during: boot_sweep(neo, neo_pwr);
        on(BootDone) => WakingHost;
    }

    // Wake the host using whichever input the host actually responds to.
    // Keyboard first (more reliable on macOS), then a mouse shake as
    // belt-and-suspenders. Each substate runs a oneshot entry action with
    // its own internal deadline; the chart timer is what advances the
    // chart. No durings, no events — purely entry + timer.
    state WakingHost {
        during: pulse_blue(neo);
        default(WakingWithKeyboard);

        state WakingWithKeyboard {
            entry: keyboard_wake;
            on(after KBD_PHASE_DURATION) => WakingWithMouse;
        }

        state WakingWithMouse {
            entry: mouse_wake;
            on(after MOUSE_PHASE_DURATION) => Settling;
        }
    }

    // Quiet pause so the display has time to come out of sleep before the
    // cursor starts drawing the spinner.
    state Settling {
        during: pulse_white(neo);
        on(after SETTLING_DELAY) => Spinning;
    }

    state Spinning {
        during: animate_spinner(mouse);
        during: fade_to_green(neo);
        on(SpinDone) => Active;
    }

    state Active {
        entry: capture_active_start;
        on(every JIGGLE_PERIOD) => jiggle_pair;
        on(after WARN_10_AT) => Warning10;
        on(after WARN_5_AT) => Warning5;
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

        state Warning10 {
            during: animate_warning_10(mouse);
            on(WarnDone) => Breathing;
        }

        state Warning5 {
            during: animate_warning_5(mouse);
            on(WarnDone) => Breathing;
        }
    }

    state Ending {
        during: blink_fast_red(neo);
        default(Spiraling);

        state Spiraling {
            during: animate_final_spiral(mouse);
            on(SpiralDone) => Quiet;
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

    async fn keyboard_wake(&mut self) {
        let result = embassy_futures::select::select(
            wake_with_keyboard(&mut self.kbd),
            Timer::after(KBD_WAKE_DEADLINE),
        )
        .await;
        #[cfg(feature = "defmt")]
        match result {
            embassy_futures::select::Either::First(_) => defmt::info!("kbd wake: completed"),
            embassy_futures::select::Either::Second(_) => {
                defmt::warn!(
                    "kbd wake: deadline hit ({} ms)",
                    KBD_WAKE_DEADLINE.as_millis()
                )
            }
        }
        #[cfg(not(feature = "defmt"))]
        let _ = result;
        // Belt-and-suspenders: always send an all-keys-released report,
        // even if the deadline preempted the loop *between* a key-down
        // and its key-up. Without this, a stuck modifier (Shift!) or key
        // on the host side could persist until the user unplugs the
        // device. Bounded by KBD_RELEASE_DEADLINE so a misbehaving
        // endpoint can't pin the chart.
        let _ = embassy_futures::select::select(
            send_kbd(&mut self.kbd, 0, [0; 6]),
            Timer::after(KBD_RELEASE_DEADLINE),
        )
        .await;
    }

    async fn mouse_wake(&mut self) {
        let result = embassy_futures::select::select(
            wake_with_mouse(&mut self.mouse),
            Timer::after(MOUSE_WAKE_DEADLINE),
        )
        .await;
        #[cfg(feature = "defmt")]
        match result {
            embassy_futures::select::Either::First(_) => defmt::info!("mouse wake: completed"),
            embassy_futures::select::Either::Second(_) => defmt::warn!(
                "mouse wake: deadline hit ({} ms)",
                MOUSE_WAKE_DEADLINE.as_millis()
            ),
        }
        #[cfg(not(feature = "defmt"))]
        let _ = result;
    }

    async fn jiggle_pair(&mut self) {
        let (dx, dy): (i8, i8) = if (RoscRng::next_u8() & 1) == 0 {
            (1, 0)
        } else {
            (0, 1)
        };
        #[cfg(feature = "defmt")]
        defmt::info!("jiggle: dx={} dy={}", dx, dy);
        send_mouse(&mut self.mouse, dx, dy).await;
        Timer::after(PIXEL_DWELL).await;
        send_mouse(&mut self.mouse, -dx, -dy).await;
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
