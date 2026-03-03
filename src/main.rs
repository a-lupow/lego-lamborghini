mod bt_device;
mod hub_controller;
mod logger;

use bluer::Device;
use log::{debug, info, warn};
use sdl2::{
    controller::{Axis, Button},
    event::Event,
};

use crate::{
    bt_device::BtManager,
    hub_controller::{DriveState, HubController},
    logger::SimpleLogger,
};

static HUB_BT_MAC: &str = "0C:4B:EE:EA:76:F7"; // TODO: Make this configurable.
static CONTROLLER_PREFIX: &str = "DualSense Wireless Controller";
static LOGGER: SimpleLogger = SimpleLogger;

#[tokio::main]
async fn main() {
    log::set_logger(&LOGGER).unwrap();
    log::set_max_level(log::LevelFilter::Info);

    let ctrl_c_signal = tokio::signal::ctrl_c();

    tokio::select! {
        _ = do_main() => {
            debug!("Main task completed, exiting.");
        },
        _ = ctrl_c_signal => {
            info!("Application terminated by user.");
        }
    }
}

/**
 * Main application logic, including connection management and the main control loop.
 */
async fn do_main() {
    let manager = BtManager::new().await;

    loop {
        // Simultaneously attempt to connect to both the controller and hub, and wait for either to complete or for a Ctrl+C signal to cancel the attempt.
        // If either connection fails, we will retry the entire process.
        let (controller_result, hub_result) = tokio::select! {
            results = async { tokio::join!(
                manager.connect("Controller", async |device: &Device| {
                    let name = device.name().await.unwrap_or_default();

                    if let Some(name) = name {
                        // Since any controller is supported, match by prefix to avoid accidentally connecting to other BT devices.
                        return name.starts_with(CONTROLLER_PREFIX);
                    }

                    return false;
                }),
                manager.connect("Hub", async |device: &Device| {
                    // Simply match by MAC address.
                    return device.address().to_string() == HUB_BT_MAC;
                }),
            ) } => results,
            _ = tokio::signal::ctrl_c() => {
                info!("Connection attempt cancelled by user.");
                manager.disconnect_all().await.unwrap();
                return;
            }
        };

        if let Err(e) = controller_result {
            warn!("Failed to connect to controller: {}. Retrying...", e);
            manager.disconnect_all().await.unwrap();
            continue;
        }

        let hub = match hub_result {
            Ok(hub) => hub,
            Err(e) => {
                warn!("Failed to connect to hub: {}. Retrying...", e);
                manager.disconnect_all().await.unwrap();
                continue;
            }
        };

        // Initialize the hub controller.
        let mut hub_controller = HubController::new(hub);

        match hub_controller.connect().await {
            Ok(_) => info!("Connected to hub controller"),
            Err(e) => {
                warn!("Failed to connect to hub controller: {}. Retrying...", e);

                // Reset all connections to ensure a clean state for the next attempt.
                manager.disconnect_all().await.unwrap();
                continue;
            }
        }

        info!("Initiating handshake with hub controller");
        match hub_controller.handshake().await {
            Ok(_) => info!("Handshake successful"),
            Err(e) => {
                warn!("Handshake failed: {}. Retrying...", e);

                // Reset all connections to ensure a clean state for the next attempt.
                manager.disconnect_all().await.unwrap();
                continue;
            }
        }

        // At this point we have successfully connected to both the controller and hub, and completed the handshake. We can now enter the main control loop.
        let sdl_context = sdl2::init().unwrap();
        let controller_subsystem = sdl_context.game_controller().unwrap();

        // Open every SDL2-enumerated game controller and keep the handles alive
        // in a Vec — dropping a GameController handle closes it immediately, which
        // would stop events from being delivered before the loop even starts.
        let _controllers: Vec<_> = (0..controller_subsystem.num_joysticks().unwrap_or(0))
            .filter(|&i| controller_subsystem.is_game_controller(i))
            .filter_map(|i| match controller_subsystem.open(i) {
                Ok(c) => {
                    info!("Opened controller {}: {}", i, c.name());
                    Some(c)
                }
                Err(e) => {
                    warn!("Failed to open controller {}: {}", i, e);
                    None
                }
            })
            .collect();

        info!("Entering main control loop");

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
}
