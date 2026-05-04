//! Keyboard HID: report send + the F13-tap host-wake helper.

use embassy_time::{Duration as EDuration, Timer, with_timeout};
use embassy_usb::class::hid::HidWriter;
use usbd_hid::descriptor::KeyboardReport;

use crate::config::{KBD_KEY_F13, KBD_TAP_GAP, KBD_TAP_HOLD, KBD_WAKE_TAPS};
use crate::usb::UsbDriver;

pub(crate) type KbdHid = HidWriter<'static, UsbDriver, 8>;

// Tap F13 four times. macOS reliably wakes from any keyboard event but is
// inconsistent about waking from raw mouse motion. Plain oneshot helper —
// the action method that calls this races it against KBD_WAKE_DEADLINE
// AND unconditionally sends an all-keys-released cleanup report after,
// to make sure we never leave a key held on the host.
pub(crate) async fn wake_with_keyboard(kbd: &mut KbdHid) {
    for _ in 0..KBD_WAKE_TAPS {
        send_kbd(kbd, 0, [KBD_KEY_F13, 0, 0, 0, 0, 0]).await;
        Timer::after(KBD_TAP_HOLD).await;
        send_kbd(kbd, 0, [0; 6]).await;
        Timer::after(KBD_TAP_GAP).await;
    }
}

// ── HID primitive ──────────────────────────────────────────────────

pub(crate) async fn send_kbd(kbd: &mut KbdHid, modifier: u8, keycodes: [u8; 6]) {
    let report = KeyboardReport {
        modifier,
        reserved: 0,
        leds: 0,
        keycodes,
    };
    let _ = with_timeout(EDuration::from_secs(3), kbd.write_serialize(&report)).await;
}
