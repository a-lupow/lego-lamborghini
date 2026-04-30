mod constants;
use constants::AUTHENTICATION_SEQUENCE;

use bitflags::bitflags;
use bluer::Device;
use log::debug;
use std::error::Error;
use std::time::{self, Duration, Instant};
use uuid::Uuid;

bitflags! {
    /// Drive mode flags sent to the LEGO hub with each drive command.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct DriveState: u8 {
        const Brake = 1;
        const TurboOff = 1 << 1;
        const LightsOff = 1 << 2;
    }
}

impl std::fmt::Display for DriveState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self)
    }
}

#[derive(Clone, PartialEq)]
pub struct DriveCommand {
    pub speed: i8,
    pub steer: i8,
    pub mode: DriveState,
}

pub struct HubController {
    device: Device,
    characteristic_uuid: Uuid,
    delay_between_commands: u64,
    last_command_time: time::Instant,

    characteristic: Option<bluer::gatt::remote::Characteristic>,

    pub last_drive_command: Option<DriveCommand>,
}

static HUB_CHARACTERISTIC_UUID: &str = "00001624-1212-efde-1623-785feabcd123";

impl HubController {
    pub fn new(device: Device) -> Self {
        HubController {
            device,
            characteristic_uuid: Uuid::parse_str(HUB_CHARACTERISTIC_UUID).unwrap_or_else(|error| {
                panic!("Invalid characteristic UUID {HUB_CHARACTERISTIC_UUID}: {error}")
            }),
            characteristic: None,
            // Use standard delay of 50ms.
            delay_between_commands: 50,
            last_command_time: Instant::now() - Duration::from_millis(50),
            last_drive_command: None,
        }
    }

    /// Discover the GATT characteristic used to send commands to the hub.
    ///
    /// Bluetooth connection, pairing, and trust are handled by `BtManager`.
    pub async fn connect(&mut self) -> Result<(), Box<dyn Error>> {
        for service in self.device.services().await? {
            for characteristic in service.characteristics().await? {
                if characteristic.uuid().await? == self.characteristic_uuid {
                    debug!("Found target characteristic: {}", self.characteristic_uuid);
                    self.characteristic = Some(characteristic);

                    return Ok(());
                }
            }
        }

        return Err("Characteristic not found".into());
    }

    /// Send a raw command to the hub while respecting the minimum inter-command delay.
    async fn send_command(
        &mut self,
        command: &[u8],
        ignore_timing: bool,
    ) -> Result<(), Box<dyn Error>> {
        // Ensure we respect the delay between commands.
        let now = Instant::now();
        let duration_since = now.duration_since(self.last_command_time);

        if !ignore_timing && duration_since < Duration::from_millis(self.delay_between_commands) {
            let sleep_time = Duration::from_millis(self.delay_between_commands) - duration_since;
            debug!("Waiting for {:?} before sending next command", sleep_time);
            tokio::time::sleep(sleep_time).await;
        }

        let characteristic = self
            .characteristic
            .as_ref()
            .ok_or("Characteristic not found")?;

        debug!("Sending command: {:?}", command);

        // WriteOp::Command is "write without response", which is the default.
        characteristic.write(command).await?;
        self.last_command_time = Instant::now();

        Ok(())
    }

    /// Perform the authentication handshake required by the hub.
    pub async fn handshake(&mut self) -> Result<(), Box<dyn Error>> {
        for command in AUTHENTICATION_SEQUENCE {
            self.send_command(command, false).await?;
        }

        Ok(())
    }

    /// Send a drive command to the hub.
    ///
    /// Timing is managed externally. The caller is expected to invoke this on a fixed
    /// 50 ms cadence so the hub receives updates at a stable rate.
    ///
    /// Deduplication is still performed here: if the command has not changed since the
    /// last successful send, the BLE write is skipped to avoid unnecessary traffic.
    pub async fn drive(&mut self, command: DriveCommand) -> Result<(), Box<dyn Error>> {
        // Skip the write if the command hasn't changed since last time.
        if self
            .last_drive_command
            .as_ref()
            .is_some_and(|last| *last == command)
        {
            debug!("Drive command did not change, skipping BLE write");
            return Ok(());
        }

        self.send_command(
            &[
                0x0d,
                0x00,
                0x81,
                0x36,
                0x11,
                0x51,
                0x00,
                0x03,
                0x00,
                command.speed as u8,
                command.steer as u8,
                command.mode.bits(),
                0x00,
            ],
            true,
        )
        .await?;

        self.last_drive_command = Some(command);

        Ok(())
    }
}
