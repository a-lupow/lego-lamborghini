mod bt_device;
mod hub_controller;
mod logger;

use clap::{crate_name, crate_version, Arg, Command};
use rkyv::{Archive, Deserialize, Serialize};
use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::Arc,
    time::{Duration, Instant},
};

use log::{debug, info, warn};
use tokio::{
    net::UdpSocket,
    sync::{broadcast, watch},
    time,
};

use crate::{
    bt_device::BtManager,
    hub_controller::{DriveCommand, DriveState, HubController},
    logger::SimpleLogger,
};

static DEFAULT_HUB_BT_MAC: &str = "0C:4B:EE:EA:76:F7";
static HUB_DEVICE_NAME: &str = "hub";
static LOGGER: std::sync::OnceLock<SimpleLogger> = std::sync::OnceLock::new();

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let matches = Command::new(crate_name!())
        .version(crate_version!())
        .about("Control a Bluetooth-connected Lamborghini model car using a game controller.")
        .arg(
            Arg::new("log-level")
                .short('l')
                .long("log-level")
                .value_parser(["error", "warn", "info", "debug", "trace"])
                .default_value("info")
                .help("Set the logging level"),
        )
        .arg(
            Arg::new("target")
                .short('t')
                .long("target")
                .default_value(DEFAULT_HUB_BT_MAC)
                .help("Bluetooth MAC address of the hub"),
        )
        .get_matches();

    let logger = LOGGER.get_or_init(SimpleLogger::new);
    if let Err(error) = log::set_logger(logger) {
        eprintln!("Failed to initialize logger: {}", error);
        return;
    }

    let log_level = matches
        .get_one::<String>("log-level")
        .map(|value| value.as_str())
        .unwrap_or("info");

    log::set_max_level(match log_level {
        "error" => log::LevelFilter::Error,
        "warn" => log::LevelFilter::Warn,
        "info" => log::LevelFilter::Info,
        "debug" => log::LevelFilter::Debug,
        "trace" => log::LevelFilter::Trace,
        _ => log::LevelFilter::Warn,
    });

    // Single source of truth for the latest drive command.
    let (drive_tx, drive_rx) = watch::channel::<Option<DriveCommand>>(None);
    let (control_response_sender, control_response_receiver) =
        broadcast::channel::<ControlMessage>(32);

    let bt_manager = BtManager::new().await;
    let bt_mac = matches.get_one::<String>("target").unwrap().clone();

    tokio::join!(
        run_bt_task(bt_mac, bt_manager, control_response_sender, drive_rx),
        run_control_task(control_response_receiver, drive_tx),
    );
}

#[derive(Archive, Serialize, Deserialize)]
struct DriveCommandRaw {
    pub speed: i8,
    pub steer: i8,
    pub mode: u8,
}

impl Into<DriveCommand> for DriveCommandRaw {
    fn into(self) -> DriveCommand {
        DriveCommand {
            speed: self.speed,
            steer: self.steer,
            mode: DriveState::from_bits_truncate(self.mode),
        }
    }
}

#[derive(Archive, Deserialize)]
enum ControlCommand {
    Drive(DriveCommandRaw),
    Ping,
}

#[derive(Debug, Clone, Archive, Serialize)]
enum ReadyStatus {
    WaitingForHub,
    Connecting,
    Handshaking,
    Ready,
}

#[derive(Debug, Clone, Archive, Serialize)]
struct Status {
    ready: ReadyStatus,
}

#[derive(Debug, Clone, Archive, Serialize)]
enum ControlMessage {
    Info(Status),
}

async fn run_control_task(
    mut message_receiver: broadcast::Receiver<ControlMessage>,
    drive_tx: watch::Sender<Option<DriveCommand>>,
) {
    let socket = match UdpSocket::bind("0.0.0.0:8080").await {
        Ok(socket) => Arc::new(socket),
        Err(error) => {
            warn!("Failed to bind UDP socket on 0.0.0.0:8080: {}", error);
            return;
        }
    };

    match socket.local_addr() {
        Ok(addr) => info!("UDP listening on {}", addr),
        Err(error) => warn!("Failed to read UDP socket local address: {}", error),
    }

    let mut buffer = vec![0u8; 4096];
    let mut peers: HashMap<SocketAddr, Instant> = HashMap::new();
    let mut last_status: Option<Status> = None;

    loop {
        tokio::select! {
            Ok(message) = message_receiver.recv() => {
                let ControlMessage::Info(status) = &message;
                last_status = Some(status.clone());

                // Evict peers that haven't been active for a while.
                peers.retain(|peer, last_seen| {
                    if last_seen.elapsed().as_secs() > 300 {
                        info!("Evicted inactive peer: {}", peer);
                        false
                    } else {
                        true
                    }
                });

                let message_bytes = match rkyv::to_bytes::<rkyv::rancor::Error>(&message) {
                    Ok(bytes) => bytes,
                    Err(error) => {
                        warn!("Failed to serialize control message: {}", error);
                        continue;
                    }
                };

                for peer in peers.keys() {
                    if let Err(error) = socket.send_to(&message_bytes, peer).await {
                        warn!("Failed to send message to {}: {}", peer, error);
                    }
                }
            },

            Ok((len, peer_addr)) = socket.recv_from(&mut buffer) => {
                if !peers.contains_key(&peer_addr) {
                    info!("New peer connected: {}", peer_addr);
                }
                peers.insert(peer_addr, Instant::now());

                tokio::spawn(handle_udp_packet(
                    socket.clone(),
                    last_status.clone(),
                    buffer[..len].to_vec(),
                    peer_addr,
                    drive_tx.clone(),
                ));
            },

            else => {
                warn!("Control task encountered an error, exiting");
                break;
            }
        }
    }
}

async fn handle_udp_packet(
    socket: Arc<UdpSocket>,
    last_status: Option<Status>,
    data: Vec<u8>,
    peer: SocketAddr,
    drive_tx: watch::Sender<Option<DriveCommand>>,
) {
    let command = match rkyv::from_bytes::<ControlCommand, rkyv::rancor::Error>(&data) {
        Ok(cmd) => cmd,
        Err(error) => {
            warn!("Failed to deserialize UDP packet from {}: {}", peer, error);
            return;
        }
    };

    match command {
        ControlCommand::Ping => {
            let status = last_status.unwrap_or(Status {
                ready: ReadyStatus::WaitingForHub,
            });
            info!(
                "Received ping from {}, responding with status: {:?}",
                peer, status,
            );
            let message_bytes =
                match rkyv::to_bytes::<rkyv::rancor::Error>(&ControlMessage::Info(status)) {
                    Ok(bytes) => bytes,
                    Err(error) => {
                        warn!("Failed to serialize ping response for {}: {}", peer, error);
                        return;
                    }
                };
            if let Err(error) = socket.send_to(&message_bytes, peer).await {
                warn!("Failed to send ping response to {}: {}", peer, error);
            }
        }

        ControlCommand::Drive(raw) => {
            let drive: DriveCommand = raw.into();

            debug!(
                "Received drive command from {}: speed {}, steer {}, mode {}",
                peer, drive.speed, drive.steer, drive.mode
            );

            // Send it over the channel.
            drive_tx.send(Some(drive)).ok();
        }
    }
}

async fn run_bt_task(
    hub_mac: String,
    mut manager: BtManager,
    control_response: broadcast::Sender<ControlMessage>,
    mut drive_rx: watch::Receiver<Option<DriveCommand>>,
) {
    debug!("Initiating BT control loop task");

    loop {
        control_response
            .send(ControlMessage::Info(Status {
                ready: ReadyStatus::WaitingForHub,
            }))
            .ok();

        if manager.has_connection(HUB_DEVICE_NAME).await {
            info!("Already connected to hub, skipping connection step");
            continue;
        }

        if let Err(error) = manager.forget_device(&hub_mac).await {
            info!(
                "Failed to forget hub device (it might not have been remembered yet): {}",
                error
            );
        }

        let result = manager
            .connect(HUB_DEVICE_NAME, async |device| {
                device.address().to_string() == hub_mac
            })
            .await;

        if let Err(error) = &result {
            warn!("Failed to connect to hub: {}", error);
            continue;
        }

        control_response
            .send(ControlMessage::Info(Status {
                ready: ReadyStatus::Connecting,
            }))
            .ok();

        let hub = match result {
            Ok(device) => {
                debug!("Successfully connected to hub: {}", device.address());
                device
            }
            Err(_) => continue,
        };

        let mut hub_controller = HubController::new(hub);

        match hub_controller.connect().await {
            Ok(_) => info!("Connected to hub controller"),
            Err(e) => {
                warn!("Failed to connect to hub controller: {}. Retrying...", e);
                if let Err(error) = manager.disconnect_all().await {
                    warn!(
                        "Failed to disconnect devices after controller setup error: {}",
                        error
                    );
                }
                continue;
            }
        }

        control_response
            .send(ControlMessage::Info(Status {
                ready: ReadyStatus::Handshaking,
            }))
            .ok();

        info!("Initiating handshake with hub controller");
        match hub_controller.handshake().await {
            Ok(_) => info!("Handshake successful"),
            Err(e) => {
                warn!("Handshake failed: {}. Retrying...", e);
                if let Err(error) = manager.disconnect_all().await {
                    warn!(
                        "Failed to disconnect devices after handshake error: {}",
                        error
                    );
                }
                continue;
            }
        }

        info!("Entering main control loop");

        control_response
            .send(ControlMessage::Info(Status {
                ready: ReadyStatus::Ready,
            }))
            .ok();

        // Discard any command that was queued before we were ready to handle it.
        drive_rx.mark_unchanged();

        #[allow(unused_assignments)]
        let mut last_send_time = Instant::now();

        loop {
            if drive_rx.changed().await.is_err() {
                info!("Drive command channel closed, exiting control loop");
                return;
            }

            // Borrow and clone the latest value.
            let command = drive_rx.borrow_and_update().clone();

            if let Some(command) = command {
                info!(
                    "Sending command speed: {}, steer: {}",
                    command.speed, command.steer
                );

                last_send_time = Instant::now();
                if let Err(error) = hub_controller.drive(command).await {
                    warn!(
                        "Failed to send drive command: {}. Attempting to reconnect...",
                        error
                    );
                    break;
                }

                // Rate-limit: wait 50 ms before the next send.
                let duration = Instant::now().duration_since(last_send_time);

                if duration.as_millis() < 50 {
                    let sleep_duration = Duration::from_millis(50) - duration;
                    debug!("Sleeping for {:?} to rate-limit commands", sleep_duration);
                    time::sleep(sleep_duration).await;
                }
            }
        }
    }
}
