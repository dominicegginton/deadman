pub const SOCKET_PATH: &str = "/tmp/deadman-ipc.sock";

pub mod server {
    use super::SOCKET_PATH;
    use std::fs;
    use std::io::{self, Read, Write};
    use std::os::fd::AsRawFd;
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::sync::Arc;
    use std::thread;
    use tracing::{debug, error, info, warn};

    pub fn start_ipc_server<F>(handler: F)
    where
        F: Fn(&str) -> Result<String, String> + Send + Sync + 'static,
    {
        let _ = fs::remove_file(SOCKET_PATH);
        let listener = UnixListener::bind(SOCKET_PATH).expect("Failed to bind to socket");
        info!("IPC server listening on {SOCKET_PATH}");

        let handler = Arc::new(handler);

        for stream in listener.incoming() {
            match stream {
                Ok(stream) => {
                    let handler = Arc::clone(&handler);
                    thread::spawn(move || {
                        handle_client(stream, handler);
                    });
                }
                Err(err) => {
                    error!("Failed to accept connection: {err}");
                }
            }
        }
    }

    fn handle_client(
        mut stream: UnixStream,
        handler: Arc<dyn Fn(&str) -> Result<String, String> + Send + Sync>,
    ) {
        if let Err(err) = ensure_same_user(&stream) {
            warn!("Rejected client: {err}");
            return;
        }

        let mut buffer = [0; 512];
        match stream.read(&mut buffer) {
            Ok(size) => {
                let message = String::from_utf8_lossy(&buffer[..size]);
                debug!("Received IPC message: {message}");

                let response = match handler(message.trim()) {
                    Ok(body) => body,
                    Err(err) => {
                        warn!("Handler reported error: {err}");
                        format!("ERR: {err}")
                    }
                };

                if let Err(err) = stream.write_all(response.as_bytes()) {
                    error!("Failed to send response: {err}");
                }
            }
            Err(err) => {
                error!("Failed to read from client: {err}");
            }
        }
    }

    fn ensure_same_user(stream: &UnixStream) -> io::Result<()> {
        let fd = stream.as_raw_fd();
        let mut credentials = libc::ucred { pid: 0, uid: 0, gid: 0 };
        let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;

        let rc = unsafe {
            libc::getsockopt(
                fd,
                libc::SOL_SOCKET,
                libc::SO_PEERCRED,
                &mut credentials as *mut _ as *mut _,
                &mut len,
            )
        };

        if rc != 0 {
            return Err(io::Error::last_os_error());
        }

        if len as usize != std::mem::size_of::<libc::ucred>() {
            return Err(io::Error::new(
                io::ErrorKind::Other,
                "Unexpected credential size from socket",
            ));
        }

        let current_uid = unsafe { libc::geteuid() };
        if credentials.uid != current_uid {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "Client UID does not match daemon UID",
            ));
        }

        Ok(())
    }
}

pub mod client {
    use super::SOCKET_PATH;
    use std::io::{self, Read, Write};
    use std::net::Shutdown;
    use std::os::unix::net::UnixStream;

    fn send_ipc_message(message: &str) -> io::Result<String> {
        let mut stream = UnixStream::connect(SOCKET_PATH)?;
        stream.write_all(message.as_bytes())?;
        let _ = stream.shutdown(Shutdown::Write);

        let mut buffer = Vec::new();
        stream.read_to_end(&mut buffer)?;

        Ok(String::from_utf8_lossy(&buffer).trim().to_string())
    }

    pub fn get_status() -> io::Result<String> {
        send_ipc_message("status")
    }

    pub fn tether(bus: &str, device_id: &str) -> io::Result<String> {
        let message = format!("{} {} {}", "tether", bus, device_id);
        send_ipc_message(&message)
    }

    pub fn severe() -> io::Result<String> {
        send_ipc_message("severe")
    }
}

