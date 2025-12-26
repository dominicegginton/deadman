use adw::gio::Settings;
use adw::glib;
use adw::gtk::{
    Application, Box, Button, Label, ListBox, MessageDialog, Orientation, ResponseType, Switch,
};
use adw::prelude::*;
use adw::{ActionRow, ApplicationWindow};
use libadwaita as adw;
use rusb::Context;
use rusb::UsbContext;
use tracing::{info, Level};

const APP_ID: &str = "com.dominicegginton.deadman";
const APP_NAME: &str = "Deadman";
const APP_DESCRIPTION: &str = "";

use std::cell::RefCell;
use std::io;
use std::process::Command;
use std::rc::Rc;
use std::thread;

use deadman_ipc::client;

fn main() {
    tracing_subscriber::fmt()
        .with_max_level(Level::INFO)
        .with_thread_ids(true)
        .init();

    let application = Application::builder().application_id(APP_ID).build();

    application.connect_startup(|_| {
        adw::init().expect("Failed to initialize libadwaita");
    });

    application.connect_activate(|app| {
        // helper to show a modal error dialog
        let show_error = move |parent: &adw::gtk::Window, text: &str| {
            let dialog = MessageDialog::builder()
                .text(text)
                .modal(true)
                .build();
            dialog.set_transient_for(Some(parent));
            dialog.show();
        };
        let app_for_click = app.clone();
        // single device list UI. status is reflected by each device row background.
        let list = ListBox::builder()
            .margin_top(32)
            .margin_end(32)
            .margin_bottom(32)
            .margin_start(32)
            .build();

        let devices_container = Box::new(Orientation::Vertical, 6);

        // Severe button: clears active tethers (requires privilege)
        let btn_severe = Button::with_label("Severe");
        list.append(&btn_severe);

        list.append(&devices_container);

        // shared selected device state (used if user selects a device row)
        let selected_device: Rc<RefCell<Option<(u8, u8)>>> = Rc::new(RefCell::new(None));

        // Populate devices list. status is obtained from IPC `get_status` and used to
        // determine which devices are currently tethered (highlighted background).
        if let Ok(ctx) = Context::new() {
            if let Ok(devices) = ctx.devices() {
                if devices.len() == 0 {
                    let label = Label::new(Some("no USB devices found"));
                    devices_container.append(&label);
                } else {
                    // query daemon status once and parse tethered device summaries
                    let mut tethered_summaries = Vec::new();
                    // Try IPC first, but if permission denied, try elevating to run the CLI (`deadman status`).
                    let status_text_res = client::get_status();
                    let mut status_text = String::new();
                    match status_text_res {
                        Ok(s) => status_text = s,
                        Err(err) => {
                            if matches!(err.kind(), io::ErrorKind::PermissionDenied) {
                                info!("permission denied contacting daemon for status — attempting elevation");
                                let elevated = Command::new("pkexec")
                                    .arg("deadman")
                                    .arg("status")
                                    .env_remove("SHELL")
                                    .output()
                                    .or_else(|_| {
                                        Command::new("sudo")
                                            .arg("deadman")
                                            .arg("status")
                                            .env_remove("SHELL")
                                            .output()
                                    });

                                match elevated {
                                    Ok(output) if output.status.success() => {
                                        status_text = String::from_utf8_lossy(&output.stdout).trim().to_string();
                                        info!(elev_out=%status_text, "elevated status succeeded");
                                    }
                                    Ok(output) => {
                                        let err_text = String::from_utf8_lossy(&output.stderr).trim().to_string();
                                        info!(error=%err_text, "elevated status failed");
                                    }
                                    Err(e) => {
                                        info!(error=%e.to_string(), "elevation attempt for status failed");
                                    }
                                }
                            } else {
                                info!(error=%err.to_string(), "failed to get status from daemon");
                            }
                        }
                    }

                    for line in status_text.lines() {
                        // status lines are like: "bus 001 address 002 1234:abcd - name [watching]"
                        // we only care about the product name (after " - ") so we can
                        // display only the device name and match tethered devices by name.
                        if let Some(idx) = line.find(" - ") {
                            let after = &line[idx + 3..];
                            // strip trailing status in brackets if present
                            let name = if let Some(br) = after.rfind('[') {
                                after[..br].trim()
                            } else {
                                after.trim()
                            };
                            if !name.is_empty() {
                                tethered_summaries.push(name.to_string());
                            }
                        }
                    }

                    for device in devices.iter() {
                        let bus = device.bus_number();
                        let addr = device.address();
                        let desc = match device.device_descriptor() {
                            Ok(d) => d,
                            Err(_) => continue,
                        };

                        let name = match device.open() {
                            Ok(handle) => match handle.read_product_string_ascii(&desc) {
                                Ok(n) => Some(n),
                                Err(_) => None,
                            },
                            Err(_) => None,
                        };

                        // Only display devices for which we could read a product name
                        let product_name = match name {
                            Some(n) => n,
                            None => continue,
                        };

                        let label_text = product_name.clone();
                        let btn = Button::with_label(&label_text);
                        let sel_inner = selected_device.clone();
                        // highlight tethered devices by matching the product name
                        if tethered_summaries.iter().any(|s| s == &product_name) {
                            btn.add_css_class("suggested-action");
                        }

                        // clicking a row will attempt to tether that device via IPC
                        let label_for_err = Label::new(None);
                        let label_text_clone = label_text.clone();
                        let app_for_click = app_for_click.clone();
                        btn.connect_clicked(move |b| {
                            *sel_inner.borrow_mut() = Some((bus, addr));

                            let bus_s = bus.to_string();
                            let dev_s = addr.to_string();
                            match client::tether(&bus_s, &dev_s) {
                                Ok(resp) => {
                                    info!(response=%resp, "tether command succeeded");
                                    // mark button as highlighted to reflect tether
                                    b.add_css_class("suggested-action");
                                    // quit the application after successful tether
                                    app_for_click.quit();
                                }
                                Err(err) => {
                                    // If we failed due to permission, try to elevate and run the CLI via pkexec or sudo
                                    let try_elevate = matches!(err.kind(), io::ErrorKind::PermissionDenied);
                                    if try_elevate {
                                        info!("permission denied contacting daemon — attempting elevation");
                                        // try pkexec first
                                        let elevated = Command::new("pkexec")
                                            .arg("deadman")
                                            .arg("tether")
                                            .arg(&bus_s)
                                            .arg(&dev_s)
                                            .env_remove("SHELL")
                                            .output()
                                            .or_else(|_| {
                                                // fallback to sudo if pkexec not available
                                                Command::new("sudo")
                                                    .arg("deadman")
                                                    .arg("tether")
                                                    .arg(&bus_s)
                                                    .arg(&dev_s)
                                                    .env_remove("SHELL")
                                                    .output()
                                            });

                                        match elevated {
                                            Ok(output) if output.status.success() => {
                                                let out = String::from_utf8_lossy(&output.stdout).trim().to_string();
                                                info!(elev_out=%out, "elevated tether succeeded");
                                                b.add_css_class("suggested-action");
                                                app_for_click.quit();
                                            }
                                            Ok(output) => {
                                                let err_text = String::from_utf8_lossy(&output.stderr).trim().to_string();
                                                let msg = if err_text.is_empty() {
                                                    format!("elevated command failed (exit {})", output.status)
                                                } else {
                                                    format!("elevated error: {}", err_text)
                                                };
                                                info!(error=%msg, "elevated tether failed");
                                                if let Some(window) = app_for_click.active_window() {
                                                    show_error(&window, &msg);
                                                }
                                            }
                                            Err(e) => {
                                                let msg = format!("failed to launch elevation helper: {}", e);
                                                info!(error=%msg, "elevation attempt failed");
                                                if let Some(window) = app_for_click.active_window() {
                                                    show_error(&window, &msg);
                                                }
                                            }
                                        }
                                        } else {
                                            let msg = format!("tether error: {}", err);
                                            info!(error=%msg, device=%label_text_clone, "tether failed");
                                            if let Some(window) = app_for_click.active_window() {
                                                show_error(&window, &msg);
                                            }
                                        }
                                }
                            }
                        });

                        devices_container.append(&btn);
                    }
                }
            }
        }

        // severe button handler: ask for confirmation, then call IPC (with elevation fallback)
        let app_for_severe = app_for_click.clone();
        let show_error_for_severe = show_error.clone();
        btn_severe.connect_clicked(move |_| {
            let app_for_severe = app_for_severe.clone();
            let show_error_for_severe = show_error_for_severe.clone();
            if let Some(window) = app_for_severe.active_window() {
                let dialog = MessageDialog::builder()
                    .text("Are you sure?")
                    .modal(true)
                    .build();
                dialog.add_button("Cancel", ResponseType::Cancel);
                dialog.add_button("Proceed", ResponseType::Ok);
                dialog.set_transient_for(Some(&window));
                dialog.connect_response(move |d, resp| {
                    d.close();
                    if resp == ResponseType::Ok {
                        // attempt IPC severe
                        match client::severe() {
                            Ok(resp) => {
                                info!(response=%resp, "severe command succeeded");
                                if let Some(w) = app_for_severe.active_window() {
                                    show_error_for_severe(&w, &resp);
                                }
                                app_for_severe.quit();
                            }
                            Err(err) => {
                                // try elevation on permission denied
                                if matches!(err.kind(), io::ErrorKind::PermissionDenied) {
                                    info!("permission denied contacting daemon for severe — attempting elevation");
                                    let elevated = Command::new("pkexec")
                                        .arg("deadman")
                                        .arg("severe")
                                        .env_remove("SHELL")
                                        .output()
                                        .or_else(|_| {
                                            Command::new("sudo")
                                                .arg("deadman")
                                                .arg("severe")
                                                .env_remove("SHELL")
                                                .output()
                                        });

                                    match elevated {
                                        Ok(output) if output.status.success() => {
                                            let out = String::from_utf8_lossy(&output.stdout).trim().to_string();
                                            info!(elev_out=%out, "elevated severe succeeded");
                                            if let Some(w) = app_for_severe.active_window() {
                                                show_error_for_severe(&w, &out);
                                            }
                                            app_for_severe.quit();
                                        }
                                        Ok(output) => {
                                            let err_text = String::from_utf8_lossy(&output.stderr).trim().to_string();
                                            let msg = if err_text.is_empty() {
                                                format!("elevated command failed (exit {})", output.status)
                                            } else {
                                                format!("elevated error: {}", err_text)
                                            };
                                            if let Some(w) = app_for_severe.active_window() {
                                                show_error_for_severe(&w, &msg);
                                            }
                                        }
                                        Err(e) => {
                                            let msg = format!("failed to launch elevation helper: {}", e);
                                            if let Some(w) = app_for_severe.active_window() {
                                                show_error_for_severe(&w, &msg);
                                            }
                                        }
                                    }
                                } else {
                                    let msg = format!("severe error: {}", err);
                                    if let Some(w) = app_for_severe.active_window() {
                                        show_error_for_severe(&w, &msg);
                                    }
                                }
                            }
                        }
                    }
                });
                dialog.show();
            }
        });

        let content = Box::new(Orientation::Vertical, 0);
        content.append(&list);

        let window = ApplicationWindow::builder()
            .application(app)
            .content(&content)
            .build();

        window.show();
    });

    application.run();
}
