mod constants;
use constants::AUTHENTICATION_SEQUENCE;

use bitflags::bitflags;
use bluer::Device;
use log::debug;
use std::error::Error;
use std::time::{self, Duration, Instant};
use uuid::Uuid;

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

#[derive(PartialEq)]
pub struct DriveCommand {
    pub speed: i8,
    pub steer: i8,
    pub mode: DriveState,
}

pub struct HubController {
    device: Device,
    char_uuid: Uuid,
    delay_between_commands: u64,
    last_command_time: time::Instant,

    characteristic: Option<bluer::gatt::remote::Characteristic>,

    last_drive_command: Option<DriveCommand>,
}

static HUB_CHAR_UUID: &str = "00001624-1212-efde-1623-785feabcd123";

impl HubController {
    pub fn new(device: Device) -> Self {
        HubController {
            device,
            char_uuid: Uuid::parse_str(HUB_CHAR_UUID).expect("Invalid characteristic UUID"),
            characteristic: None,
            // Use standard delay of 50ms.
            delay_between_commands: 50,
            last_command_time: Instant::now() - Duration::from_millis(50),
            last_drive_command: None,
        }
    }

    /**
     * Discovers the required GATT characteristic for communication.
     * Bluetooth connection and pairing are handled externally by BtManager.
     */
    pub async fn connect(&mut self) -> Result<(), Box<dyn Error>> {
        for service in self.device.services().await? {
            for characteristic in service.characteristics().await? {
                if characteristic.uuid().await? == self.char_uuid {
                    debug!("Found target characteristic: {}", self.char_uuid);
                    self.characteristic = Some(characteristic);

                    return Ok(());
                }
            }
        }

        return Err("Characteristic not found".into());
    }

    /**
     * Sends a command to the hub, ensuring that we respect the required delay between commands.
     */
    async fn send_command(
        &mut self,
        command: &[u8],
        drop_if_busy: bool,
    ) -> Result<(), Box<dyn Error>> {
        // Ensure we respect the delay between commands.
        let now = Instant::now();
        let duration_since = now.duration_since(self.last_command_time);

        if duration_since < Duration::from_millis(self.delay_between_commands) {
            // This is required to make sure we get responsive behaviour from the hub when controlled.
            // A better solution would be to store it in a queue and send it as soon as the delay has passed, if the command is still relevant.
            // Since that is not the priority right now, I'll address it later.
            if drop_if_busy {
                debug!(
                    "Dropping command {:?} because we're still within the delay period",
                    command
                );
                return Ok(());
            }

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

    /**
     * Performs the handshake sequence by sending a series of authentication commands to the hub.
     */
    pub async fn handshake(&mut self) -> Result<(), Box<dyn Error>> {
        for command in AUTHENTICATION_SEQUENCE {
            self.send_command(command, false).await?;
        }

        Ok(())
    }

    /**
     * Sends a drive command to the hub with the specified speed, steer, and mode.
     */
    pub async fn drive(&mut self, command: DriveCommand) -> Result<(), Box<dyn Error>> {
        // Make sure the last drive command is different.
        if self
            .last_drive_command
            .as_ref()
            .is_some_and(|last_command| *last_command == command)
        {
            debug!("Drive command did not change, skipping");

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

        Ok(())
    }
}
