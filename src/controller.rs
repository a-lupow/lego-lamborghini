use std::error::Error;
use std::time::Duration;

use bluer::{agent::Agent, AdapterEvent, Session};
use futures::StreamExt;
use tokio::time::sleep;

static DEVICE_NAME_PREFIX: &str = "DualSense Wireless Controller";
const SCAN_TIMEOUT: Duration = Duration::from_secs(30);

type DynError = Box<dyn Error>;

/// Ensures a DualSense controller is paired, trusted, and connected via BlueZ
/// so that the OS registers it as an HID input device (making it visible to SDL2).
///
/// - If already paired and connected: no-op.
/// - If already paired but not connected: connects.
/// - If not paired: scans, pairs, trusts, then connects.
///   The DualSense must be in pairing mode (hold PS + Create until light bar blinks).
pub async fn connect_controller() -> Result<(), DynError> {
    println!("Initiating controller...");

    let session = Session::new().await?;
    let adapter = session.default_adapter().await?;
    adapter.set_powered(true).await?;

    // Register a no-input/no-output agent so BlueZ auto-confirms pairing
    // for devices like the DualSense that don't need a PIN or passkey.
    let agent = Agent::default();
    let _agent_handle = session.register_agent(agent).await?;

    // Check if the DualSense is already known to BlueZ.
    for addr in adapter.device_addresses().await? {
        let device = adapter.device(addr)?;
        let name = device.name().await?.unwrap_or_default();
        if !name.starts_with(DEVICE_NAME_PREFIX) {
            continue;
        }

        let paired = device.is_paired().await?;
        let connected = device.is_connected().await?;
        println!(
            "Found known DualSense: {} (paired={}, connected={})",
            addr, paired, connected
        );

        if connected {
            println!("DualSense is already connected.");
            return Ok(());
        }

        if paired {
            println!("DualSense is already paired, connecting...");
            device.connect().await?;
            println!("DualSense connected successfully.");
            return Ok(());
        }

        // Known but not paired — fall through to the full pairing flow below,
        // using this device directly instead of scanning.
        println!("DualSense found but not paired, pairing...");
        device.pair().await?;
        device.set_trusted(true).await?;
        device.connect().await?;
        println!("DualSense paired and connected successfully.");
        return Ok(());
    }

    // Device not known to BlueZ at all — scan for it.
    println!(
        "No known DualSense found. Starting scan ({}s)...",
        SCAN_TIMEOUT.as_secs()
    );
    println!("Put the DualSense into pairing mode: hold PS + Create until the light bar blinks.");

    let discover = adapter.discover_devices().await?;
    futures::pin_mut!(discover);

    let found = tokio::time::timeout(SCAN_TIMEOUT, async {
        while let Some(event) = discover.next().await {
            if let AdapterEvent::DeviceAdded(addr) = event {
                let device = match adapter.device(addr) {
                    Ok(d) => d,
                    Err(_) => continue,
                };
                let name = match device.name().await {
                    Ok(Some(n)) => n,
                    _ => continue,
                };
                if name.starts_with(DEVICE_NAME_PREFIX) {
                    println!("Found DualSense: {} ({})", name, addr);
                    return Some(addr);
                }
            }
        }
        None
    })
    .await;

    let addr = match found {
        Ok(Some(addr)) => addr,
        Ok(None) => return Err("Bluetooth event stream ended before DualSense was found.".into()),
        Err(_) => {
            return Err(format!(
                "Timed out after {}s scanning for DualSense. Make sure it is in pairing mode.",
                SCAN_TIMEOUT.as_secs()
            )
            .into())
        }
    };

    let device = adapter.device(addr)?;

    println!("Pairing with DualSense...");
    device.pair().await?;
    println!("Pairing successful.");

    device.set_trusted(true).await?;
    println!("Device trusted.");

    println!("Connecting...");
    device.connect().await?;

    // Give udev/kernel a moment to finish registering the /dev/input node
    // after BlueZ reports the connection as established.
    // On embedded hardware (e.g. Luckfox Ultra W) this can take several seconds.
    sleep(Duration::from_secs(5)).await;

    println!("DualSense connected and ready.");
    Ok(())
}
