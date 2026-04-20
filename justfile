# Every recipe runs inside `mise exec --`, so cargo, rustup components, and
# cargo helpers (objcopy, uf2conv, …) come from the toolchain pinned in
# mise.toml — no shell activation required.
set shell := ["mise", "exec", "--", "sh", "-eu", "-c"]

firmware     := "jiggly"
target       := "thumbv6m-none-eabi"
base_addr    := "0x10000000"
family_id    := "0xE48BFF56"

out_release  := "target" / target / "release" / firmware
out_bin      := "target" / target / "release" / firmware + ".bin"
out_uf2      := "target" / target / "release" / firmware + ".uf2"

# Show available recipes.
default:
    @just --list --unsorted

# Debug build.
build:
    cargo build

# Optimised release build.
release:
    cargo build --release

# `cargo check` on the firmware binary.
check:
    cargo check

# Clippy — treat warnings as errors.
clippy:
    cargo clippy -- -D warnings

# Format source in place.
fmt:
    cargo fmt --all

# Verify formatting without writing.
fmt-check:
    cargo fmt --all -- --check

# All lint gates (fmt-check + clippy).
lint: fmt-check clippy

# Local CI pipeline: check + lint + release build.
ci: check lint release

# Remove build artefacts.
clean:
    cargo clean

# Re-run `cargo check` on source changes.
watch:
    cargo watch -x check

# Print section sizes of the release ELF.
size: release
    cargo size --release -- -A

# Symbol-level size breakdown of the release ELF.
bloat: release
    cargo bloat --release -n 30

# Expand macros (helpful for inspecting `bind_interrupts!` / `#[embassy_executor::main]`).
expand:
    cargo expand

# Build rustdoc for this crate and dependencies, then open in a browser.
doc:
    cargo doc --open

# Strip the release ELF to a raw binary image.
bin: release
    cargo objcopy --release -- -O binary {{ out_bin }}
    @echo "→ {{ out_bin }}"

# Produce a UF2 image ready to flash onto the XIAO bootloader volume.
uf2: bin
    uf2conv {{ out_bin }} -b {{ base_addr }} -f {{ family_id }} -o {{ out_uf2 }}
    @echo "→ {{ out_uf2 }}"

# Build UF2 and copy it to the first mounted XIAO UF2 volume (Linux/macOS).
flash: uf2
    #!/usr/bin/env bash
    set -euo pipefail
    echo "Hold BOOT (B) and tap RESET (R) on the XIAO RP2040 to enter bootloader mode…"
    for i in {1..60}; do
        for mount in /media/*/RPI-RP2* /run/media/*/RPI-RP2* /Volumes/RPI-RP2*; do
            if [[ -d "$mount" ]]; then
                cp "{{ out_uf2 }}" "$mount/"
                sync
                echo "→ Copied to $mount"
                exit 0
            fi
        done
        sleep 1
    done
    echo "Timed out waiting for XIAO UF2 volume. Copy {{ out_uf2 }} manually." >&2
    exit 1

# Install the toolchain and cargo helpers declared in mise.toml.
bootstrap:
    mise install

# Show firmware file size summary.
stats: uf2
    @ls -lh {{ out_release }} {{ out_bin }} {{ out_uf2 }}
