//! USB driver type aliases, identity helpers, and infra tasks
//! (USB device pump + watchdog feeder).

use embassy_rp::{
    Peri,
    flash::Flash,
    peripherals::{FLASH, USB},
    usb::Driver,
    watchdog::Watchdog,
};
use embassy_time::Timer;
use embassy_usb::UsbDevice;
use static_cell::StaticCell;

use crate::config::{WATCHDOG_FEED_INTERVAL, WATCHDOG_TIMEOUT};

pub(crate) type UsbDriver = Driver<'static, USB>;

// ── USB serial number from RP2040 unique chip ID ───────────────────
// Reads the 64-bit unique ID baked into the on-board SPI flash, formats
// it as 16 ASCII hex chars in a static buffer, and returns a `'static`
// string suitable for `embassy_usb::Config::serial_number`. This makes
// the device's USB identity stable per-board across replugs (so hosts
// stop treating each plug as a new device) while still being unique
// between different boards.
const FLASH_SIZE: usize = 2 * 1024 * 1024; // Xiao RP2040 has 2 MB.

pub(crate) fn make_serial(flash_periph: Peri<'static, FLASH>) -> &'static str {
    static SERIAL: StaticCell<[u8; 16]> = StaticCell::new();
    const HEX: &[u8; 16] = b"0123456789ABCDEF";

    let mut flash = Flash::<_, _, FLASH_SIZE>::new_blocking(flash_periph);
    let mut id = [0u8; 8];
    let _ = flash.blocking_unique_id(&mut id);

    let buf = SERIAL.init([0; 16]);
    for (i, &b) in id.iter().enumerate() {
        buf[i * 2] = HEX[(b >> 4) as usize];
        buf[i * 2 + 1] = HEX[(b & 0x0f) as usize];
    }
    core::str::from_utf8(buf).unwrap()
}

// ── Tasks ──────────────────────────────────────────────────────────

#[embassy_executor::task]
pub(crate) async fn usb_task(mut device: UsbDevice<'static, UsbDriver>) {
    device.run().await;
}

#[embassy_executor::task]
pub(crate) async fn watchdog_task(mut wd: Watchdog) -> ! {
    loop {
        wd.feed(WATCHDOG_TIMEOUT);
        Timer::after(WATCHDOG_FEED_INTERVAL).await;
    }
}
