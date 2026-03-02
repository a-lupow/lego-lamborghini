mod controller;
mod hub_controller;

use std::time::{Duration, Instant};

use sdl2::{
    controller::{Axis, Button},
    event::Event,
};

use crate::{
    controller::connect_controller,
    hub_controller::{DriveState, HubController},
};

#[tokio::main]
async fn main() {
    // Manually pair controllers via BT.
    connect_controller().await.unwrap();
    println!("Initiating sdl2");

    let sdl_context = sdl2::init().unwrap();
    let controller_subsystem = sdl_context.game_controller().unwrap();

    println!("Looking for a controller...");

    // On embedded hardware the kernel HID node may take several seconds to appear
    // after BlueZ reports the connection. Retry for up to 15 seconds before giving up.
    const CONTROLLER_SEARCH_TIMEOUT: Duration = Duration::from_secs(15);
    const CONTROLLER_RETRY_INTERVAL: Duration = Duration::from_millis(500);

    let search_start = Instant::now();

    'search: loop {
        let available = controller_subsystem
            .num_joysticks()
            .map_err(|e| e.to_string())
            .unwrap();

        for id in 0..available {
            if controller_subsystem.is_game_controller(id) {
                match controller_subsystem.open(id) {
                    Ok(ctrl) => {
                        println!("Found controller: {}", ctrl.name());
                        break 'search;
                    }
                    Err(e) => {
                        eprintln!("Failed to open controller {}: {}", id, e);
                    }
                }
            }
        }

        if search_start.elapsed() >= CONTROLLER_SEARCH_TIMEOUT {
            println!(
                "No controllers found after {}s. Please connect a controller and restart the application.",
                CONTROLLER_SEARCH_TIMEOUT.as_secs()
            );
            return;
        }

        println!("No controllers found yet, retrying...");
        std::thread::sleep(CONTROLLER_RETRY_INTERVAL);
    }

    println!("Initializing the hub");
    let mut hub_controller = HubController::new();

    hub_controller.connect().await.unwrap();
    hub_controller.handshake().await.unwrap();

    println!("Initiating game loop");

    const DEAD_ZONE: u16 = 500;
    let mut thrust_value = 0.0;
    let mut reverse_value = 0.0;
    let mut steer_value = 0.0;

    let mut drive_state = DriveState::empty();
    drive_state.set(DriveState::TurboOff, true);
    drive_state.set(DriveState::Break, false);
    drive_state.set(DriveState::LightsOff, false);

    let mut event_pump = sdl_context.event_pump().unwrap();
    'running: loop {
        for event in event_pump.poll_iter() {
            // Process SDL2 events, including controller input.
            match event {
                Event::Quit { .. } => break 'running,

                // Handle controller input events
                Event::ControllerAxisMotion {
                    timestamp: _,
                    which: _,
                    axis,
                    value,
                } => {
                    // Store state for all buttons.
                    match axis {
                        Axis::TriggerRight => {
                            thrust_value = (value as f32) / 32767.0; // Normalize to [0, 1]
                        }
                        Axis::TriggerLeft => {
                            reverse_value = (value as f32) / 32767.0; // Normalize to [0, 1]
                        }
                        Axis::LeftX => {
                            steer_value = if value.unsigned_abs() > DEAD_ZONE {
                                (value as f32) / 32767.0 // Normalize to [-1, 1]
                            } else {
                                0.0 // Within dead zone, treat as neutral
                            };
                        }
                        _ => {}
                    }
                }

                Event::ControllerButtonDown {
                    timestamp: _,
                    which: _,
                    button,
                } => {
                    match button {
                        Button::A => {
                            drive_state.set(DriveState::Break, true);
                        }

                        Button::X => {
                            // Toggle turbo mode on/off.
                            drive_state.toggle(DriveState::TurboOff);
                        }

                        Button::Y => {
                            // Toggle lights on/off.
                            drive_state.toggle(DriveState::LightsOff);
                        }

                        _ => {}
                    }
                }

                Event::ControllerButtonUp {
                    timestamp: _,
                    which: _,
                    button,
                } => match button {
                    Button::A => {
                        drive_state.set(DriveState::Break, false);
                    }
                    _ => {}
                },
                _ => {}
            }
        }

        // Apply events to the hub controller.
        hub_controller
            .drive(
                if thrust_value > 0.0 {
                    (thrust_value * 100.0) as i8
                } else {
                    -(reverse_value * 100.0) as i8
                },
                (steer_value * 100.0) as i8,
                drive_state,
            )
            .await
            .unwrap();
    }
}
