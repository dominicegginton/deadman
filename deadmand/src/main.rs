use std::collections::HashMap;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use deadman_ipc::server::start_ipc_server;
use rusb::{Context, Device, Hotplug, UsbContext};
use tracing::{debug, error, info, warn};
use tracing_subscriber::{EnvFilter, fmt, layer::SubscriberExt, util::SubscriberInitExt};

fn main() {
    init_tracing();

    check_privileges();

    info!("deadmand starting");

    if !rusb::has_hotplug() {
        warn!("libusb hotplug support is not available; tether commands will fail");
    }

    let state = Arc::new(Mutex::new(DaemonState::default()));

    start_ipc_server({
        let state = Arc::clone(&state);
        move |command| handle_command(command, Arc::clone(&state))
    });
}

fn init_tracing() {
    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    tracing_subscriber::registry()
        .with(env_filter)
        .with(
            fmt::layer()
                .with_target(false)
                .with_line_number(true)
                .with_thread_names(true)
                .with_ansi(false)
                .with_file(true),
        )
        .init();
}

#[cfg(unix)]
fn check_privileges() {
    use nix::unistd::Uid;

    if !Uid::effective().is_root() {
        error!("deadmand must be run with root privileges");
        eprintln!("Error: deadmand must be run with root privileges");
        std::process::exit(1);
    }
}

#[cfg(not(unix))]
fn check_privileges() {
    // On non-Unix systems, skip the privilege check
    warn!("Privilege checking is not implemented for this platform");
}

fn handle_command(command: &str, state: Arc<Mutex<DaemonState>>) -> Result<String, String> {
    debug!(command = command, "received IPC command");

    let mut parts = command.split_whitespace();
    let Some(name) = parts.next() else {
        error!("received empty message");
        return Err("empty command".to_string());
    };

    match name {
        "status" => {
            if let Some(extra) = parts.next() {
                return Err(format!("unexpected argument: {extra}"));
            }
            handle_status(state)
        }
        "tether" => {
            let bus = parts
                .next()
                .ok_or_else(|| "missing bus number".to_string())?;
            let address = parts
                .next()
                .ok_or_else(|| "missing device id".to_string())?;
            if let Some(extra) = parts.next() {
                return Err(format!("unexpected argument: {extra}"));
            }
            handle_tether(bus, address, state)
        }
        "severe" => {
            if let Some(extra) = parts.next() {
                return Err(format!("unexpected argument: {extra}"));
            }
            handle_severe(state)
        }
        other => {
            warn!(command = other, "unknown command");
            Err(format!("unknown command: {other}"))
        }
    }
}

fn handle_status(state: Arc<Mutex<DaemonState>>) -> Result<String, String> {
    let mut guard = state
        .lock()
        .map_err(|_| "failed to acquire daemon state".to_string())?;

    guard
        .monitors
        .retain(|_, monitor| !monitor.removed.load(Ordering::SeqCst));

    if guard.monitors.is_empty() {
        return Ok("no active tethers".to_string());
    }

    let mut lines = Vec::with_capacity(guard.monitors.len());
    for (key, monitor) in guard.monitors.iter() {
        let status = if monitor.removed.load(Ordering::SeqCst) {
            "disconnected"
        } else {
            "watching"
        };

        let summary = format_device_summary(
            *key,
            monitor.vendor_id,
            monitor.product_id,
            monitor.product_name.as_deref(),
        );

        lines.push(format!("{summary} [{status}]"));
    }

    Ok(lines.join("\n"))
}

fn handle_tether(
    bus: &str,
    address: &str,
    state: Arc<Mutex<DaemonState>>,
) -> Result<String, String> {
    if !rusb::has_hotplug() {
        warn!("tether requested but hotplug support is not available");
        return Err("libusb hotplug support is not available on this system".to_string());
    }

    let bus_number = bus
        .parse::<u8>()
        .map_err(|_| format!("invalid bus number: {bus}"))?;
    let device_address = address
        .parse::<u8>()
        .map_err(|_| format!("invalid device id: {address}"))?;

    let key = DeviceKey::new(bus_number, device_address);

    {
        let guard = state
            .lock()
            .map_err(|_| "failed to acquire daemon state".to_string())?;
        if guard.monitors.contains_key(&key) {
            return Err(format!(
                "device {:03}:{:03} is already tethered",
                bus_number, device_address
            ));
        }
    }

    let device_info = lookup_device(bus_number, device_address)?;
    let summary = format_device_summary(
        key,
        device_info.vendor_id,
        device_info.product_id,
        device_info.product_name.as_deref(),
    );

    let removed_flag = Arc::new(AtomicBool::new(false));
    let lock_on_remove = Arc::new(AtomicBool::new(true));

    {
        let mut guard = state
            .lock()
            .map_err(|_| "failed to acquire daemon state".to_string())?;
        if guard.monitors.contains_key(&key) {
            return Err(format!(
                "device {:03}:{:03} is already tethered",
                bus_number, device_address
            ));
        }

        guard.monitors.insert(
            key,
            DeviceMonitor {
                vendor_id: device_info.vendor_id,
                product_id: device_info.product_id,
                product_name: device_info.product_name.clone(),
                removed: Arc::clone(&removed_flag),
                lock_on_remove: Arc::clone(&lock_on_remove),
            },
        );
    }

    let thread_state = Arc::clone(&state);
    let product_name = device_info.product_name.clone();
    thread::spawn(move || {
        monitor_device(
            thread_state,
            key,
            device_info.vendor_id,
            device_info.product_id,
            product_name,
            removed_flag,
            lock_on_remove,
        );
    });

    info!(device = %summary, "tether activated");

    Ok(format!("tether active for {summary}"))
}

fn handle_severe(state: Arc<Mutex<DaemonState>>) -> Result<String, String> {
    warn!("received severe command; clearing active tethers");

    let mut guard = state
        .lock()
        .map_err(|_| "failed to acquire daemon state".to_string())?;

    if guard.monitors.is_empty() {
        info!("no tethers to clear");
        return Ok("no active tethers".to_string());
    }

    let cleared = guard.monitors.len();

    for (key, monitor) in guard.monitors.iter() {
        monitor.lock_on_remove.store(false, Ordering::SeqCst);
        monitor.removed.store(true, Ordering::SeqCst);
        info!(
            bus = key.bus,
            address = key.address,
            vendor_id = monitor.vendor_id,
            product_id = monitor.product_id,
            "clearing tether"
        );
    }

    guard.monitors.clear();

    Ok(format!("cleared {cleared} tether(s)"))
}

fn lock_all_sessions() -> Result<(), String> {
    let output = Command::new("loginctl")
        .arg("list-sessions")
        .output()
        .map_err(|err| format!("failed to list sessions: {err}"))?;

    if !output.status.success() {
        return Err(format!(
            "loginctl list-sessions exited with status {status}",
            status = output.status
        ));
    }

    for line in String::from_utf8_lossy(&output.stdout).lines().skip(1) {
        let session_id = match line.split_whitespace().next() {
            Some(id) => id,
            None => continue,
        };

        match Command::new("loginctl")
            .arg("lock-session")
            .arg(session_id)
            .status()
        {
            Ok(status) if status.success() => {
                info!(session = session_id, "locked session");
            }
            Ok(status) => {
                warn!(session = session_id, status = %status, "lock-session failed");
            }
            Err(err) => {
                warn!(session = session_id, error = %err, "failed to run lock-session");
            }
        }
    }

    Ok(())
}

fn monitor_device(
    state: Arc<Mutex<DaemonState>>,
    key: DeviceKey,
    vendor_id: u16,
    product_id: u16,
    product_name: Option<String>,
    removed: Arc<AtomicBool>,
    lock_on_remove: Arc<AtomicBool>,
) {
    let device_label = format_device_summary(key, vendor_id, product_id, product_name.as_deref());

    let context = match Context::new() {
        Ok(ctx) => ctx,
        Err(err) => {
            error!(device = %device_label, error = %err, "failed to create USB context");
            remove_monitor(&state, key);
            return;
        }
    };

    let watcher = SelectedDeviceWatcher {
        key,
        vendor_id,
        product_id,
        product_name,
        removed_flag: Arc::clone(&removed),
    };

    let registration =
        match context.register_callback(Some(vendor_id), Some(product_id), None, Box::new(watcher))
        {
            Ok(reg) => reg,
            Err(err) => {
                error!(device = %device_label, error = %err, "failed to register hotplug callback");
                remove_monitor(&state, key);
                return;
            }
        };

    info!(device = %device_label, "monitoring device for removal");

    while !removed.load(Ordering::SeqCst) {
        if let Err(err) = context.handle_events(Some(Duration::from_millis(250))) {
            error!(device = %device_label, error = %err, "error while handling USB events");
            break;
        }
    }

    drop(registration);

    if removed.load(Ordering::SeqCst) {
        if lock_on_remove.load(Ordering::SeqCst) {
            info!(device = %device_label, "device removal detected; locking sessions");
            if let Err(err) = lock_all_sessions() {
                error!(device = %device_label, error = %err, "failed to lock sessions");
            }
        } else {
            info!(device = %device_label, "tether cleared without locking sessions");
        }
    }

    remove_monitor(&state, key);
}

fn remove_monitor(state: &Arc<Mutex<DaemonState>>, key: DeviceKey) {
    match state.lock() {
        Ok(mut guard) => {
            guard.monitors.remove(&key);
        }
        Err(err) => {
            let mut guard = err.into_inner();
            guard.monitors.remove(&key);
        }
    }
}

fn lookup_device(bus: u8, address: u8) -> Result<DeviceInfo, String> {
    let context = Context::new().map_err(|err| format!("failed to create USB context: {err}"))?;
    let devices = context
        .devices()
        .map_err(|err| format!("failed to list USB devices: {err}"))?;

    for device in devices.iter() {
        if device.bus_number() == bus && device.address() == address {
            let descriptor = device
                .device_descriptor()
                .map_err(|err| format!("failed to read device descriptor: {err}"))?;

            let product_name = match device.open() {
                Ok(handle) => match handle.read_product_string_ascii(&descriptor) {
                    Ok(name) => Some(name),
                    Err(err) => {
                        warn!(
                            bus = bus,
                            address = address,
                            vendor_id = descriptor.vendor_id(),
                            product_id = descriptor.product_id(),
                            error = %err,
                            "could not read product string"
                        );
                        None
                    }
                },
                Err(err) => {
                    warn!(
                        bus = bus,
                        address = address,
                        vendor_id = descriptor.vendor_id(),
                        product_id = descriptor.product_id(),
                        error = %err,
                        "could not open device"
                    );
                    None
                }
            };

            return Ok(DeviceInfo {
                vendor_id: descriptor.vendor_id(),
                product_id: descriptor.product_id(),
                product_name,
            });
        }
    }

    Err(format!(
        "no device found on bus {:03} address {:03}",
        bus, address
    ))
}

fn format_device_summary(
    key: DeviceKey,
    vendor_id: u16,
    product_id: u16,
    product_name: Option<&str>,
) -> String {
    let mut summary = format!(
        "bus {:03} address {:03} {:04x}:{:04x}",
        key.bus, key.address, vendor_id, product_id
    );

    if let Some(name) = product_name {
        summary.push_str(" - ");
        summary.push_str(name);
    }

    summary
}

#[derive(Default)]
struct DaemonState {
    monitors: HashMap<DeviceKey, DeviceMonitor>,
}

#[derive(Clone, Copy, Hash, PartialEq, Eq)]
struct DeviceKey {
    bus: u8,
    address: u8,
}

impl DeviceKey {
    fn new(bus: u8, address: u8) -> Self {
        Self { bus, address }
    }
}

struct DeviceMonitor {
    vendor_id: u16,
    product_id: u16,
    product_name: Option<String>,
    removed: Arc<AtomicBool>,
    lock_on_remove: Arc<AtomicBool>,
}

struct DeviceInfo {
    vendor_id: u16,
    product_id: u16,
    product_name: Option<String>,
}

struct SelectedDeviceWatcher {
    key: DeviceKey,
    vendor_id: u16,
    product_id: u16,
    product_name: Option<String>,
    removed_flag: Arc<AtomicBool>,
}

impl SelectedDeviceWatcher {
    fn display_name(&self) -> &str {
        self.product_name.as_deref().unwrap_or("selected device")
    }
}

impl Hotplug<Context> for SelectedDeviceWatcher {
    fn device_arrived(&mut self, device: Device<Context>) {
        if device.bus_number() == self.key.bus && device.address() == self.key.address {
            info!(
                bus = self.key.bus,
                address = self.key.address,
                vendor_id = self.vendor_id,
                product_id = self.product_id,
                name = %self.display_name(),
                "device reattached"
            );
            self.removed_flag.store(false, Ordering::SeqCst);
        }
    }

    fn device_left(&mut self, device: Device<Context>) {
        if device.bus_number() == self.key.bus && device.address() == self.key.address {
            info!(
                bus = self.key.bus,
                address = self.key.address,
                vendor_id = self.vendor_id,
                product_id = self.product_id,
                name = %self.display_name(),
                "device unplugged"
            );
            self.removed_flag.store(true, Ordering::SeqCst);
        }
    }
}
