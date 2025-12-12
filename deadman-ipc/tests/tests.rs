// Unit tests for the deadman-ipc library
// These tests use the public API in deadman-ipc/src/lib.rs

use deadman_ipc::client;
use deadman_ipc::server;
use rand::{Rng, distributions::Alphanumeric};
use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

fn unique_socket_path() -> String {
    let rand_str: String = rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(8)
        .map(char::from)
        .collect();
    format!("/tmp/deadman-ipc-test-{}.sock", rand_str)
}

#[test]
fn test_ipc_server_and_client_status() {
    let socket_path = unique_socket_path();
    let socket_path_clone = socket_path.clone();
    if Path::new(&socket_path).exists() {
        let _ = fs::remove_file(&socket_path);
    }
    let handle = thread::spawn(move || {
        server::start_ipc_server_once_with_path(&socket_path_clone, |msg| {
            if msg == "status" {
                Ok("OK".to_string())
            } else {
                Err("Unknown command".to_string())
            }
        });
    });
    thread::sleep(Duration::from_millis(50));
    let response = client::get_status_with_path(&socket_path).unwrap();
    assert_eq!(response, "OK");
    let _ = fs::remove_file(&socket_path);
    let _ = handle.join();
}

#[test]
fn test_ipc_tether_command() {
    let socket_path = unique_socket_path();
    if Path::new(&socket_path).exists() {
        let _ = fs::remove_file(&socket_path);
    }
    let socket_path_clone = socket_path.clone();
    let handle = thread::spawn(move || {
        server::start_ipc_server_once_with_path(&socket_path_clone, |msg| {
            if msg.starts_with("tether ") {
                Ok(format!("Tethered: {}", msg))
            } else {
                Err("Unknown command".to_string())
            }
        });
    });
    thread::sleep(Duration::from_millis(50));
    let response = client::tether_with_path(&socket_path, "bus1", "dev42").unwrap();
    assert!(response.contains("Tethered: tether bus1 dev42"));
    let _ = fs::remove_file(&socket_path);
    let _ = handle.join();
}

#[test]
fn test_ipc_severe_command() {
    let socket_path = unique_socket_path();
    if Path::new(&socket_path).exists() {
        let _ = fs::remove_file(&socket_path);
    }
    let socket_path_clone = socket_path.clone();
    let handle = thread::spawn(move || {
        server::start_ipc_server_once_with_path(&socket_path_clone, |msg| {
            if msg == "severe" {
                Ok("Severe mode enabled".to_string())
            } else {
                Err("Unknown command".to_string())
            }
        });
    });
    thread::sleep(Duration::from_millis(50));
    let response = client::severe_with_path(&socket_path).unwrap();
    assert_eq!(response, "Severe mode enabled");
    let _ = fs::remove_file(&socket_path);
    let _ = handle.join();
}
