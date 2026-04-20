#![no_std]
#![no_main]

use embassy_executor::Spawner;
use embassy_rp::{
    bind_interrupts,
    clocks::RoscRng,
    peripherals::USB,
    usb::{Driver, InterruptHandler},
    watchdog::Watchdog,
};
use embassy_time::{Duration, Instant, Timer, with_timeout};
use embassy_usb::{
    Builder, Config as UsbConfig, UsbDevice,
    class::hid::{Config as HidConfig, HidBootProtocol, HidSubclass, HidWriter, State},
};
use panic_reset as _;
use static_cell::StaticCell;
use usbd_hid::descriptor::{MouseReport, SerializedDescriptor};

bind_interrupts!(struct Irqs {
    USBCTRL_IRQ => InterruptHandler<USB>;
});

type UsbDriver = Driver<'static, USB>;

const RUN_DURATION: Duration = Duration::from_secs(8 * 60 * 60);
const IDLE_BETWEEN: Duration = Duration::from_secs(270);
const PIXEL_DWELL_MS: u64 = 25;
const WATCHDOG_FEED: Duration = Duration::from_secs(8);
const SLEEP_CHUNK: Duration = Duration::from_secs(5);

#[embassy_executor::task]
async fn usb_task(mut device: UsbDevice<'static, UsbDriver>) {
    device.run().await;
}

async fn send(writer: &mut HidWriter<'static, UsbDriver, 5>, x: i8, y: i8) {
    let report = MouseReport {
        buttons: 0,
        x,
        y,
        wheel: 0,
        pan: 0,
    };
    let _ = with_timeout(Duration::from_secs(3), writer.write_serialize(&report)).await;
}

async fn sleep_chunked(watchdog: &mut Watchdog, total: Duration) {
    let mut remaining = total;
    while remaining > Duration::from_ticks(0) {
        watchdog.feed(WATCHDOG_FEED);
        let step = if remaining > SLEEP_CHUNK {
            SLEEP_CHUNK
        } else {
            remaining
        };
        Timer::after(step).await;
        remaining -= step;
    }
}

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let p = embassy_rp::init(Default::default());

    let mut watchdog = Watchdog::new(p.WATCHDOG);
    watchdog.start(Duration::from_secs(8));

    let driver = Driver::new(p.USB, Irqs);

    let mut config = UsbConfig::new(0x413c, 0x301a);
    config.manufacturer = Some("Dell");
    config.product = Some("Dell MS116 USB Optical Mouse");
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
        poll_ms: 60,
        max_packet_size: 8,
        hid_subclass: HidSubclass::Boot,
        hid_boot_protocol: HidBootProtocol::Mouse,
    };
    let mut writer = HidWriter::<_, 5>::new(&mut builder, HID_STATE.init(State::new()), hid_config);

    let usb = builder.build();
    spawner.spawn(usb_task(usb).unwrap());

    let start = Instant::now();
    loop {
        if start.elapsed() >= RUN_DURATION {
            break;
        }

        let (dx, dy): (i8, i8) = if (RoscRng::next_u8() & 1) == 0 {
            (1, 0)
        } else {
            (0, 1)
        };

        watchdog.feed(WATCHDOG_FEED);
        send(&mut writer, dx, dy).await;
        Timer::after_millis(PIXEL_DWELL_MS).await;
        send(&mut writer, -dx, -dy).await;

        sleep_chunked(&mut watchdog, IDLE_BETWEEN).await;
    }

    loop {
        watchdog.feed(WATCHDOG_FEED);
        Timer::after(SLEEP_CHUNK).await;
    }
}
