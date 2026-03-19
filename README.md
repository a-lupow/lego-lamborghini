# Lamborghini Rust

An unofficial, open-source Rust controller for the **LEGO Technic Lamborghini 42214** (or any LEGO Control+ hub-based set), 
allowing you to drive it with a PlayStation DualSense (or DualShock) controller over Bluetooth from a Linux SoC such as the 
[Luckfox Pico Ultra W](https://www.luckfox.com) or any other ARMv7 Linux device.

> [!NOTE]  
> Note that that specific model is protected with a firmware lock that prevents it from being controlled by third-party tools, 
but this project uses reverse-engineering and custom BLE commands to bypass that restriction and enable full control.
I intentionally avoided releasing the exact handshake sequence to the public, so you'll have to come up with the 
`AUTHENTICATION_SEQUENCE` yourself in case you want to use this code. 


## Hardware Setup

| Component | Notes |
|-----------|-------|
| LEGO Technic Control+ set | Tested with the Lamborghini Revuelto (42214) |
| SoC with BLE | e.g. Luckfox Pico Ultra W, Raspberry Pi, or any ARMv7/ARM64 Linux board with Bluetooth |

The MAC address of your LEGO hub must be set in `src/main.rs`:

```lamborghini-rust/src/main.rs#L17-17
static HUB_BT_MAC: &str = "0C:4B:EE:EA:76:F7"; // TODO: Make this configurable.
```

Replace the value with your hub's Bluetooth MAC address (visible in your OS Bluetooth settings or in the LEGO app when the hub is powered on).

---

## Controls (DualSense / DualShock)

| Input | Action |
|-------|--------|
| R2 (right trigger) | Throttle forward |
| L2 (left trigger) | Reverse |
| Left stick X-axis | Steering |
| Cross (A) — hold | Brake |
| Square (X) | Toggle turbo mode |
| Triangle (Y) | Toggle headlights |

---

## Prerequisites

### Linux (Debian/Ubuntu) — native or on-device

```lamborghini-rust/README.md#L1-1
sudo apt-get update
sudo apt-get install libssl-dev pkg-config libsdl2-dev libdbus-1-dev bluez
```

SDL2 is bundled and statically linked for cross-compiled targets (no runtime dependency needed on the device).

### macOS (development only — BLE via BlueZ requires Linux)

```lamborghini-rust/README.md#L1-1
brew install sdl2
```

---

## Building

### Native

```lamborghini-rust/README.md#L1-1
cargo build --release
```

### Cross-compiling for ARMv7 (e.g. Luckfox Pico Ultra W)

The project includes a `Cross.toml` and `Cross.Dockerfile` for use with [`cross`](https://github.com/cross-rs/cross):

```lamborghini-rust/README.md#L1-1
cargo install cross
cross build --release --target armv7-unknown-linux-gnueabihf
```

The compiled binary will be at `target/armv7-unknown-linux-gnueabihf/release/lamborghini-rust`.

---

## Running

```lamborghini-rust/README.md#L1-1
cargo run --release
```

Or on the target device after copying the binary:

```lamborghini-rust/README.md#L1-1
./lamborghini-rust
```

The application will:
1. Scan for and connect to both the hub (by MAC) and the controller (by name prefix) — retrying automatically on failure.
2. Perform the BLE handshake with the hub.
3. Enter the main control loop, forwarding controller input to the hub at up to 20 commands/second.
4. Gracefully shut down on Ctrl+C.

---

## Dependencies

| Crate | Purpose |
|-------|---------|
| `tokio` | Async runtime |
| `bluer` | BlueZ BLE via D-Bus |
| `dbus` | D-Bus bindings (vendored) |
| `sdl2` | Game controller input (bundled + static-linked) |
| `bitflags` | `DriveState` bitfield |
| `uuid` | GATT characteristic UUID handling |
| `log` | Logging facade |

---

## License

This project is licensed under the **MIT License**

> **This project is not affiliated with, endorsed by, or sponsored by the LEGO Group or Sony Interactive Entertainment. All trademarks belong to their respective owners.**
