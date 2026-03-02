# Lamborghini Rust Project

A Rust project for Bluetooth connectivity and DualShock controller integration using tokio, btleplug, and rust-sdl2.

## Dependencies

- **tokio**: Async runtime with full features enabled
- **btleplug**: Bluetooth Low Energy (BLE) support
- **rust-sdl2**: SDL2 bindings for game controller input

## Prerequisites

### Linux (Debian/Ubuntu)
```bash
sudo apt-get update
sudo apt-get install libssl-dev pkg-config libsdl2-dev libsdl2-image-dev libsdl2-mixer-dev libsdl2-ttf-dev
```

### macOS (with Homebrew)
```bash
brew install sdl2
```

### Windows
Download and install SDL2 from https://www.libsdl.org/download-2.0.php

## Building

```bash
cargo build
```

## Running

```bash
cargo run
```

## Development

The project structure includes:
- `src/main.rs` - Main entry point
- `src/bluetooth.rs` - Bluetooth device scanning and connection
- `src/controller.rs` - DualShock controller initialization and input handling

## Features

- Async Bluetooth device scanning with tokio
- DualShock controller detection and input handling
- Cross-platform SDL2 support

## Notes

- Make sure your DualShock controller is in pairing mode
- You may need to pair the controller with your system first
- The project uses tokio's full feature set for convenience
- Game controller database can be updated from https://github.com/gabomdq/SDL_GameControllerDB
