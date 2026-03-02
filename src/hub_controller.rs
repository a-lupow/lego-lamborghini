mod constants;
use constants::AUTHENTICATION_SEQUENCE;

use bitflags::bitflags;
use futures::StreamExt;
use std::error::Error;
use std::time::{self, Duration, Instant};

use btleplug::{
    api::{Central, CentralEvent, Characteristic, Manager as _, Peripheral as _, ScanFilter},
    platform::{Manager, Peripheral},
};

bitflags! {
    /**
     * DriveState represents the various states the Lamborghini Hub can be in while driving.
     */
     #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct DriveState: u8 {
        const Break = 1;
        const TurboOff = 1 << 1;
        const LightsOff = 1 << 2;
    }
}

pub struct HubController {
    mac: String,
    char_uuid: String,
    delay_between_commands: u64,
    last_command_time: time::Instant,

    target_device: Option<Peripheral>,
    target_characteristic: Option<Characteristic>,
}

impl HubController {
    pub fn new() -> Self {
        HubController {
            // Initialize fields
            mac: "0C:4B:EE:EA:76:F7".to_owned(),
            char_uuid: "00001624-1212-efde-1623-785feabcd123".to_owned(),
            target_device: None,
            target_characteristic: None,
            // Use standard delay of 50ms.
            delay_between_commands: 50,
            last_command_time: Instant::now() - Duration::from_millis(50),
        }
    }

    /**
     * Connects to the hub by scanning for Bluetooth devices, finding the target device by its MAC address,
     * and discovering the required characteristic for communication.
     */
    pub async fn connect(&mut self) -> Result<(), Box<dyn Error>> {
        // Initialize the manager.
        let manager = Manager::new().await.unwrap();
        let bt_adapters = manager.adapters().await.unwrap();

        // Take the first adapter.
        let adapter = bt_adapters
            .into_iter()
            .nth(0)
            .ok_or("No Bluetooth adapters found")
            .unwrap();

        let mut events = adapter.events().await?;
        adapter.start_scan(ScanFilter::default()).await?;

        // Wait for 2 seconds to allow the scan to populate devices.
        tokio::time::sleep(Duration::from_secs(2)).await;

        loop {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {
                    println!("Scan interrupted by user.");
                    return Err("Scan cancelled".into());
                }
                event = events.next() => {
                    match event {
                        Some(CentralEvent::DeviceDiscovered(id)) | Some(CentralEvent::DeviceUpdated(id)) => {
                            let peripheral = adapter.peripheral(&id).await?;
                            let properties = peripheral.properties().await?.unwrap_or_default();
                            if properties.address.to_string() == self.mac {
                                println!(
                                    "Found target device: {}",
                                    properties.local_name.unwrap_or("Unknown".to_string())
                                );
                                self.target_device = Some(peripheral);
                                break;
                            }
                        }
                        None => {
                            return Err("Bluetooth event stream ended unexpectedly".into());
                        }
                        _ => {}
                    }
                }
            }
        }

        println!("Device found, pairing...");
        let device = self
            .target_device
            .as_ref()
            .ok_or("Device not found")
            .unwrap();

        device.connect().await.unwrap();
        device.discover_services().await.unwrap();

        // Find the characteristic we want to write to.
        let characteristics = device.characteristics();
        self.target_characteristic = characteristics
            .into_iter()
            .find(|c| c.uuid.to_string() == self.char_uuid);

        if self.target_characteristic.is_none() {
            return Err("Characteristic not found".into());
        }

        return Ok(());
    }

    /**
     * Sends a command to the hub, ensuring that we respect the required delay between commands.
     */
    async fn send_command(&mut self, command: &[u8]) -> Result<(), Box<dyn Error>> {
        // Ensure we respect the delay between commands.
        let now = Instant::now();
        let duration_since = now.duration_since(self.last_command_time);

        if duration_since < Duration::from_millis(self.delay_between_commands) {
            let sleep_time = Duration::from_millis(self.delay_between_commands) - duration_since;
            println!("Waiting for {:?} before sending next command", sleep_time);

            tokio::time::sleep(sleep_time).await;
        }

        let device = self.target_device.as_ref().ok_or("Device not found")?;
        let characteristic = self
            .target_characteristic
            .as_ref()
            .ok_or("Characteristic not found")?;

        println!("Sending command: {:?}", command);

        device
            .write(
                characteristic,
                command.as_ref(),
                btleplug::api::WriteType::WithoutResponse,
            )
            .await?;
        self.last_command_time = Instant::now();

        Ok(())
    }

    /**
     * Performs the handshake sequence by sending a series of authentication commands to the hub.
     */
    pub async fn handshake(&mut self) -> Result<(), Box<dyn Error>> {
        for command in AUTHENTICATION_SEQUENCE {
            self.send_command(command).await?;
        }

        Ok(())
    }

    /**
     * Sends a drive command to the hub with the specified speed, steer, and mode.
     */
    pub async fn drive(
        &mut self,
        speed: i8,
        steer: i8,
        mode: DriveState,
    ) -> Result<(), Box<dyn Error>> {
        // Construct the command based on the speed and steer values.
        self.send_command(&[
            0x0d,
            0x00,
            0x81,
            0x36,
            0x11,
            0x51,
            0x00,
            0x03,
            0x00,
            speed as u8,
            steer as u8,
            mode.bits(),
            0x00,
        ])
        .await
        .unwrap();

        Ok(())
    }
}
