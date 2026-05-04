#![no_std]
#![no_main]

use embassy_executor::Spawner;
use embassy_rp::{
    bind_interrupts, dma,
    gpio::{Level, Output},
    peripherals::{DMA_CH0, PIO0, USB},
    pio::{InterruptHandler as PioInterruptHandler, Pio},
    pio_programs::ws2812::{PioWs2812, PioWs2812Program},
    usb::{Driver, InterruptHandler as UsbInterruptHandler},
    watchdog::Watchdog,
};
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, channel::Channel};
use embassy_time::Timer;
use embassy_usb::{
    Builder, Config as UsbConfig,
    class::hid::{Config as HidConfig, HidBootProtocol, HidSubclass, HidWriter, State},
};
#[cfg(not(feature = "defmt"))]
use panic_reset as _;
use static_cell::StaticCell;
use usbd_hid::descriptor::{KeyboardReport, MouseReport, SerializedDescriptor};
#[cfg(feature = "defmt")]
use {defmt_rtt as _, panic_probe as _};

mod chart;
mod config;
mod kbd;
mod led;
mod mouse;
mod usb;

use chart::{Ctx, Ev, Jiggly};
use config::{WATCHDOG_FEED_INTERVAL, WATCHDOG_TIMEOUT};
use led::Neo;
use usb::{make_serial, usb_task, watchdog_task};

bind_interrupts!(struct Irqs {
    USBCTRL_IRQ => UsbInterruptHandler<USB>;
    PIO0_IRQ_0 => PioInterruptHandler<PIO0>;
    DMA_IRQ_0 => dma::InterruptHandler<DMA_CH0>;
});

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let p = embassy_rp::init(Default::default());

    #[cfg(feature = "defmt")]
    defmt::info!("jiggly v{} boot", env!("CARGO_PKG_VERSION"));

    let mut watchdog = Watchdog::new(p.WATCHDOG);
    // Pause the countdown while a debugger has the core halted, so
    // breakpoints and single-stepping under `probe-rs` don't trip the
    // 8 s reset. Cheap and always-correct, so leave it on for release too.
    watchdog.pause_on_debug(true);
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

    // pid.codes community VID with a self-allocated PID — using a real
    // Logitech Unifying Receiver VID/PID was a mistake: Linux has a kernel
    // driver (`hid-logitech-dj`) that special-cases that PID and tries to
    // talk HID++ to enumerate paired wireless devices. We don't speak
    // HID++, so the driver waits through ~10–20 s of timeouts before
    // unbinding and letting `hid-generic` actually start polling our
    // endpoints. Generic VID/PID routes straight to `hid-generic`.
    let mut config = UsbConfig::new(0x1209, 0xb0b0);
    config.manufacturer = Some("swaits.com");
    config.product = Some("jiggly");
    let serial = make_serial(p.FLASH);
    #[cfg(feature = "defmt")]
    defmt::info!("usb serial: {}", serial);
    config.serial_number = Some(serial);
    config.device_release = 0x0200; // matches firmware version 0.2.0
    config.max_power = 100;
    config.max_packet_size_0 = 64;

    static CONFIG_DESCRIPTOR: StaticCell<[u8; 256]> = StaticCell::new();
    static BOS_DESCRIPTOR: StaticCell<[u8; 256]> = StaticCell::new();
    static MSOS_DESCRIPTOR: StaticCell<[u8; 256]> = StaticCell::new();
    static CONTROL_BUF: StaticCell<[u8; 64]> = StaticCell::new();
    static MOUSE_HID_STATE: StaticCell<State> = StaticCell::new();
    static KBD_HID_STATE: StaticCell<State> = StaticCell::new();

    let mut builder = Builder::new(
        driver,
        config,
        CONFIG_DESCRIPTOR.init([0; 256]),
        BOS_DESCRIPTOR.init([0; 256]),
        MSOS_DESCRIPTOR.init([0; 256]),
        CONTROL_BUF.init([0; 64]),
    );

    let mouse_config = HidConfig {
        report_descriptor: MouseReport::desc(),
        request_handler: None,
        // 8 ms (125 Hz) — standard for full-speed mice. At the previous 60 ms
        // the host was throwing away ~7 of every 8 animation frames we sent.
        poll_ms: 8,
        max_packet_size: 8,
        hid_subclass: HidSubclass::Boot,
        hid_boot_protocol: HidBootProtocol::Mouse,
    };
    let mouse = HidWriter::<_, 5>::new(
        &mut builder,
        MOUSE_HID_STATE.init(State::new()),
        mouse_config,
    );

    let kbd_config = HidConfig {
        report_descriptor: KeyboardReport::desc(),
        request_handler: None,
        // The keyboard only fires once at boot; no need for fast polling.
        poll_ms: 10,
        max_packet_size: 8,
        hid_subclass: HidSubclass::Boot,
        hid_boot_protocol: HidBootProtocol::Keyboard,
    };
    let kbd = HidWriter::<_, 8>::new(&mut builder, KBD_HID_STATE.init(State::new()), kbd_config);

    let usb = builder.build();
    spawner.spawn(usb_task(usb).unwrap());
    spawner.spawn(watchdog_task(watchdog).unwrap());

    #[cfg(feature = "defmt")]
    defmt::info!("usb + watchdog tasks spawned, starting statechart");

    static EVENT_CHAN: Channel<CriticalSectionRawMutex, Ev, 8> = Channel::new();

    let ctx = Ctx {
        mouse,
        kbd,
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
