use std::error::Error;

use bluer::{Adapter, AdapterEvent, Device, DeviceEvent, DeviceProperty, Session};
use futures::{lock::Mutex, StreamExt};
use log::{debug, info, warn};

pub struct BtManager {
    pub adapter: Adapter,

    devices: Mutex<Vec<String>>,
}

impl BtManager {
    pub async fn new() -> Self {
        // Initiate the session.
        let session = Session::new().await.unwrap();
        let adapter = session.default_adapter().await.unwrap();

        BtManager {
            adapter,
            devices: Mutex::new(Vec::new()),
        }
    }

    /**
     * Attempts to connect to a Bluetooth device that matches the provided filter function.
     */
    async fn attempt_connection_known(
        &self,
        device_name: &str,
        filter: impl AsyncFn(&Device) -> bool,
    ) -> Result<Option<Device>, Box<dyn Error>> {
        // List available devices.
        let devices = self.adapter.device_addresses().await?;
        let mut connected_device: Option<Device> = None;

        info!(
            "Attempting to connect to a known device, {} devices found",
            devices.len()
        );

        for address in devices {
            let device = self.adapter.device(address)?;

            if !filter(&device).await {
                continue;
            }

            info!(
                "{}: Device {} matched the filter",
                device_name,
                device.address()
            );
            connected_device = Some(device);
            break;
        }

        let Some(connected_device) = connected_device else {
            info!("No devices found that match the filter.");
            return Ok(None);
        };

        // Attempt to connect to the found device.
        match self
            .attempt_connection(device_name, &connected_device)
            .await
        {
            Ok(_) => {
                info!("Device connected successfully.");
                return Ok(Some(connected_device));
            }
            Err(e) => {
                warn!("Failed to connect to device: {}, attempting discovery", e);
                return Ok(None);
            }
        }
    }

    async fn attempt_connection_discover(
        &self,
        device_name: &str,
        filter: impl AsyncFn(&Device) -> bool,
    ) -> Result<Device, Box<dyn Error>> {
        // Attempt to discover the device.
        let known_devices = self.adapter.device_addresses().await?;

        let mut discover_stream = self.adapter.discover_devices().await?;
        let mut known_event_stream = futures::stream::SelectAll::new();

        // Subscribe to known devices events.
        for address in known_devices {
            let device = self.adapter.device(address)?;

            if !filter(&device).await {
                continue;
            }

            info!(
                "{}: Watching known device {} for connections",
                device_name, address
            );

            let events = device.events().await?.map(move |event| (address, event));
            known_event_stream.push(events);
        }

        loop {
            tokio::select! {
                Some((address, event)) = known_event_stream.next() => {
                    if let DeviceEvent::PropertyChanged(DeviceProperty::Connected(true)) = event {
                        info!("{}: Known device {} connected, attempting to register it", device_name, address);
                        let device = self.adapter.device(address)?;

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
                                continue;
                            }
                        }
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
                            continue;
                        }
                    }
                }
            }
        }
    }

    /**
     * Attempts to connect to the provided Bluetooth device, pairing and trusting it if necessary.
     */
    async fn attempt_connection(
        &self,
        device_name: &str,
        device: &Device,
    ) -> Result<(), Box<dyn Error>> {
        let is_connected = device.is_connected().await?;
        let is_paired = device.is_paired().await?;
        let is_trusted = device.is_trusted().await?;

        // Pair and trust if not already paired.
        if !is_paired {
            info!(
                "{}: Device {} is not paired, attempting to pair...",
                device_name,
                device.address()
            );
            device.pair().await?;
        }

        // Make sure the device is trusted so it will not disconnect.
        if !is_trusted {
            info!(
                "{}: Device {} is not trusted, setting it to trusted...",
                device_name,
                device.address()
            );
            device.set_trusted(true).await?;
        }

        // Connect if not already connected.
        if !is_connected {
            info!(
                "{}: Device {} is not connected, attempting to connect...",
                device_name,
                device.address()
            );
            device.connect().await?;
        }

        // Register the device address.
        info!(
            "{}: Device {} connected successfully, registering it",
            device_name,
            device.address()
        );

        self.devices
            .lock()
            .await
            .push(device.address().to_string().into());

        return Ok(());
    }

    pub async fn connect(
        &self,
        device_name: &str,
        filter: impl AsyncFn(&Device) -> bool,
    ) -> Result<Device, Box<dyn Error>> {
        // First attempt to connect to a known device.
        if let Some(device) = self.attempt_connection_known(device_name, &filter).await? {
            return Ok(device);
        }

        // If that fails, attempt to discover the device and connect to it.
        return self.attempt_connection_discover(device_name, filter).await;
    }

    /**
     * Disconnects all devices that were previously connected via this manager.
     */
    pub async fn disconnect_all(&self) -> Result<(), Box<dyn Error>> {
        let devices = self.devices.lock().await.clone();

        for address in devices {
            let device = self.adapter.device(address.parse()?)?;
            if device.is_connected().await? {
                device.disconnect().await?;
            }
        }

        return Ok(());
    }
}
