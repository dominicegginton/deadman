use anyhow::{Context as AnyhowContext, Result, anyhow};
use clap::{Parser, Subcommand};
use rusb::{Context, UsbContext};

use deadman_ipc::client;

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Some(Command::Status) => run_status()?,
        Some(Command::Tether { bus, device }) => run_tether(bus, device)?,
        Some(Command::Severe) => run_severe()?,
        None => list_devices()?,
    }

    Ok(())
}

#[derive(Parser)]
#[command(author, version, about = "deadman daemon control tool", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    Status,
    Tether {
        /// USB bus number (0-255)
        bus: u8,
        /// USB device address (0-255)
        device: u8,
    },
    Severe,
}

fn run_status() -> Result<()> {
    let response = client::get_status().context("failed to request status from deadmand")?;
    let message = parse_response(response)?;
    if message.is_empty() {
        println!("ok");
    } else {
        println!("{message}");
    }
    Ok(())
}

fn run_tether(bus: u8, device: u8) -> Result<()> {
    let bus_str = bus.to_string();
    let device_str = device.to_string();

    let response = client::tether(&bus_str, &device_str)
        .with_context(|| format!("failed to request tether for {:03}:{:03}", bus, device))?;
    let message = parse_response(response)?;
    println!("{message}");
    Ok(())
}

fn run_severe() -> Result<()> {
    let response = client::severe().context("failed to send severe command")?;
    let message = parse_response(response)?;
    println!("{message}");
    Ok(())
}

fn parse_response(response: String) -> Result<String> {
    let trimmed = response.trim();
    if let Some(err) = trimmed.strip_prefix("ERR: ") {
        return Err(anyhow!("{err}", err = err.trim()));
    }
    Ok(trimmed.to_string())
}

fn list_devices() -> Result<()> {
    let context = Context::new().context("failed to create USB context")?;
    let devices = context.devices().context("failed to list USB devices")?;

    if devices.len() == 0 {
        println!("no USB devices found");
        return Ok(());
    }

    for device in devices.iter() {
        let descriptor = match device.device_descriptor() {
            Ok(desc) => desc,
            Err(err) => {
                println!(
                    "bus {:03} address {:03}: failed to read descriptor ({err})",
                    device.bus_number(),
                    device.address()
                );
                continue;
            }
        };

        let name = match device.open() {
            Ok(handle) => match handle.read_product_string_ascii(&descriptor) {
                Ok(name) => Some(name),
                Err(_) => None,
            },
            Err(_) => None,
        };

        match name {
            Some(name) => println!(
                "bus {:03} address {:03} {:04x}:{:04x} - {}",
                device.bus_number(),
                device.address(),
                descriptor.vendor_id(),
                descriptor.product_id(),
                name
            ),
            None => println!(
                "bus {:03} address {:03} {:04x}:{:04x}",
                device.bus_number(),
                device.address(),
                descriptor.vendor_id(),
                descriptor.product_id()
            ),
        }
    }

    Ok(())
}
