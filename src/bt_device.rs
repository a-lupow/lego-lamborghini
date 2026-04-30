use std::{collections::HashMap, error::Error};

use bluer::{agent::Agent, Adapter, AdapterEvent, Device, DeviceEvent, DeviceProperty, Session};
use futures::StreamExt;
use log::{debug, info, warn};

pub struct BtManager {
    pub adapter: Adapter,

    // Kept alive for the duration of the manager — dropping it unregisters the agent.
    _agent_handle: bluer::agent::AgentHandle,

    // Device addresses keyed by logical device name.
    devices: HashMap<String, String>,
}

impl BtManager {
    pub async fn new() -> Self {
        // Initialize the BlueZ session and adapter.
        let session = Session::new().await.unwrap();
        let adapter = session.default_adapter().await.unwrap();
        let agent_handle = session.register_agent(Agent::default()).await.unwrap();

        BtManager {
            adapter,
            _agent_handle: agent_handle,
            devices: HashMap::new(),
        }
    }

    pub async fn forget_device(&self, device_address: &str) -> Result<(), Box<dyn Error>> {
        info!("Forgetting device with address {}", device_address);
        self.adapter.remove_device(device_address.parse()?).await?;
        Ok(())
    }

    pub async fn connect(
        &mut self,
        device_name: &str,
        filter: impl AsyncFn(&Device) -> bool,
    ) -> Result<Device, Box<dyn Error>> {
        info!("{}: Starting a new connect attempt", device_name);

        // Gather known devices and start discovery.
        info!("{}: Fetching known device addresses...", device_name);
        let known_devices = self.adapter.device_addresses().await?;
        info!("{}: Got {} known devices", device_name, known_devices.len());

        info!("{}: Starting discovery stream...", device_name);
        let mut discover_stream = self.adapter.discover_devices().await?;
        info!("{}: Discovery stream started", device_name);
        let mut known_event_stream = futures::stream::SelectAll::new();

        // Subscribe to known devices events.
        for address in known_devices {
            let device = self.adapter.device(address)?;

            if !filter(&device).await {
                continue;
            }

            info!(
                "{}: Subscribing to events for known device {}",
                device_name, address
            );
            let events = device.events().await?.map(move |event| (address, event));
            info!("{}: Subscribed to events for {}", device_name, address);
            known_event_stream.push(events);
        }

        info!("{}: Entering event loop", device_name);

        loop {
            tokio::select! {
                Some((address, event)) = known_event_stream.next() => {
                    if let DeviceEvent::PropertyChanged(DeviceProperty::Connected(true)) = event {
                        info!("{}: Known device {} connected, attempting to register it", device_name, address);

                        let device = self.adapter.device(address)?;
                        let result = self.attempt_connection(device_name, &device).await;

                        if let Err(err) = result {
                            warn!(
                                "{}: Failed to connect to device: {}, looking for a new one",
                                device_name, err
                            );
                            // Remove the device from the adapter so BlueZ re-surfaces it as a
                            // DeviceAdded event on its next advertisement, allowing the
                            // discover_stream arm to pick it up and retry.
                            if let Err(remove_err) = self.adapter.remove_device(address).await {
                                warn!(
                                    "{}: Failed to remove device {} after failed connection: {}",
                                    device_name, address, remove_err
                                );
                            }
                            continue;
                        }

                        info!("{}: Device connected successfully.", device_name);
                        return Ok(device);
                    }
                }

                Some(event) = discover_stream.next() => {
                    // Only consider newly added devices.
                    let AdapterEvent::DeviceAdded(address) = event else {
                        debug!("Received non-device-added event: {:?}, ignoring", event);
                        continue;
                    };

                    let device = match self.adapter.device(address) {
                        Ok(item) => item,
                        Err(_) => {
                            info!("Failed to get device for address {}, skipping", address);
                            continue;
                        }
                    };

                    if !filter(&device).await {
                        continue;
                    }

                    // BlueZ may have auto-connected the device before the discovery
                    // event was delivered. If so, skip straight to registration.
                    // `attempt_connection` handles this gracefully, but checking
                    // `is_connected()` here avoids relying on a stale `is_paired` read.
                    if device.is_connected().await? {
                        info!(
                            "{}: Device {} appeared via discover but is already connected, registering it",
                            device_name, address
                        );
                        self.attempt_connection(device_name, &device).await?;
                        return Ok(device);
                    }

                    match self.attempt_connection(device_name, &device).await {
                        Ok(_) => {
                            info!("{}: Device connected successfully.", device_name);
                            return Ok(device);
                        }
                        Err(e) => {
                            warn!(
                                "{}: Failed to connect to device: {}, looking for a new one",
                                device_name, e
                            );
                            // Remove the device from the adapter so BlueZ re-surfaces it as a
                            // DeviceAdded event on its next advertisement, allowing a clean retry.
                            if let Err(remove_err) = self.adapter.remove_device(address).await {
                                warn!(
                                    "{}: Failed to remove device {} after failed connection: {}",
                                    device_name, address, remove_err
                                );
                            }
                            continue;
                        }
                    }
                }

                // Cancellation support can be added here in the future if connect
                // attempts need to become externally abortable.
            }
        }
    }

    /// Pair, connect, trust, and register the provided Bluetooth device.
    async fn attempt_connection(
        &mut self,
        device_name: &str,
        device: &Device,
    ) -> Result<(), Box<dyn Error>> {
        let is_paired = device.is_paired().await?;

        // Pair first if needed.
        if !is_paired {
            info!(
                "{}: Device {} is not paired, attempting to pair...",
                device_name,
                device.address()
            );
            if let Err(e) = device.pair().await {
                if e.to_string().contains("Already Exists") {
                    info!(
                        "{}: Device {} is already paired, continuing...",
                        device_name,
                        device.address()
                    );
                } else {
                    return Err(Box::from(e));
                }
            }
        }

        let is_connected = device.is_connected().await?;

        // Connect if needed.
        if !is_connected {
            info!(
                "{}: Device {} is not connected, attempting to connect...",
                device_name,
                device.address()
            );
            device.connect().await?;
        }

        // Trust is idempotent — don't bother reading is_trusted first.
        info!("{}: Trusting {}...", device_name, device.address());
        if let Err(e) = device.set_trusted(true).await {
            warn!(
                "{}: Failed to set device {} as trusted (non-fatal): {}",
                device_name,
                device.address(),
                e
            );
        }

        // Record the device address under its logical name.
        info!(
            "{}: Device {} connected successfully, registering it",
            device_name,
            device.address()
        );

        self.devices
            .insert(device_name.to_string(), device.address().to_string().into());

        Ok(())
    }

    pub async fn get_device(&self, device_name: &str) -> Result<Device, Box<dyn Error>> {
        let address = self.devices.get(device_name).ok_or("Device not found")?;

        Ok(self.adapter.device(address.parse()?)?)
    }

    /// Disconnects from a device with a given name, if such is registered.
    pub async fn disconnect(&mut self, device_name: &str) -> Result<(), Box<dyn Error>> {
        let address = self.devices.get(device_name).ok_or("Device not found")?;

        // Disconnect if the device is still connected.
        let device = self.adapter.device(address.parse()?)?;

        if device.is_connected().await? {
            device.disconnect().await?;
        }

        // Remove the entry from the registry.
        self.devices.remove(device_name);

        Ok(())
    }

    /// Disconnects all devices that were previously connected via the manager.
    pub async fn disconnect_all(&mut self) -> Result<(), Box<dyn Error>> {
        let devices: Vec<String> = self.devices.keys().cloned().collect();

        for name in devices {
            self.disconnect(&name).await?;
        }

        Ok(())
    }

    /// Returns true if the manager has a device with the key connected.
    pub async fn has_connection(&self, device_name: &str) -> bool {
        self.devices.contains_key(device_name)
    }
}
