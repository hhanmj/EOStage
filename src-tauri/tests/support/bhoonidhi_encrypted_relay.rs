//! Test-process-only TCP fault injection. TLS remains end-to-end and opaque.
use std::io::{Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    Arc, Mutex,
};
use std::thread::{self, JoinHandle};
use std::time::Duration;

pub struct Relay {
    pub port: u16,
    pub interrupted: Arc<AtomicBool>,
    pub bytes: Arc<AtomicU64>,
    stop: Arc<AtomicBool>,
    streams: Arc<Mutex<Vec<TcpStream>>>,
    listener: Option<JoinHandle<()>>,
}

impl Relay {
    pub fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let mut relay = Self {
            port: listener.local_addr().unwrap().port(),
            interrupted: Arc::new(AtomicBool::new(false)),
            bytes: Arc::new(AtomicU64::new(0)),
            stop: Arc::new(AtomicBool::new(false)),
            streams: Arc::new(Mutex::new(Vec::new())),
            listener: None,
        };
        let stop = relay.stop.clone();
        let streams = relay.streams.clone();
        let interrupted = relay.interrupted.clone();
        let bytes = relay.bytes.clone();
        relay.listener = Some(thread::spawn(move || {
            while !stop.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((mut client, _)) => {
                        let interrupted = interrupted.clone();
                        let bytes = bytes.clone();
                        let streams = streams.clone();
                        thread::spawn(move || {
                            let result = (|| -> std::io::Result<()> {
                                // Windows accept inherits the listener's nonblocking mode;
                                // the opaque copy loops require a blocking accepted socket.
                                client.set_nonblocking(false)?;
                                client.set_read_timeout(Some(Duration::from_secs(10)))?;
                                let mut connect = Vec::new();
                                while !connect.ends_with(b"\r\n\r\n") {
                                    let mut byte = [0];
                                    client.read_exact(&mut byte)?;
                                    connect.push(byte[0]);
                                    if connect.len() > 4096 {
                                        return Err(std::io::ErrorKind::InvalidData.into());
                                    }
                                }
                                if !connect.starts_with(
                                    b"CONNECT bhoonidhi-api.nrsc.gov.in:443 HTTP/1.1\r\n",
                                ) {
                                    return Err(std::io::ErrorKind::PermissionDenied.into());
                                }
                                // The observed owner proxy is used only by this acceptance relay.
                                let mut upstream = TcpStream::connect_timeout(
                                    &"127.0.0.1:7897".parse().unwrap(),
                                    Duration::from_secs(5),
                                )?;
                                upstream.write_all(&connect)?;
                                client.set_read_timeout(None)?;
                                streams
                                    .lock()
                                    .unwrap()
                                    .extend([client.try_clone()?, upstream.try_clone()?]);
                                let mut upload = client.try_clone()?;
                                let mut upload_target = upstream.try_clone()?;
                                thread::spawn(move || {
                                    let _ = std::io::copy(&mut upload, &mut upload_target);
                                    let _ = upload_target.shutdown(Shutdown::Write);
                                });
                                let mut buffer = [0; 64 * 1024];
                                loop {
                                    let count = upstream.read(&mut buffer)?;
                                    if count == 0 {
                                        break;
                                    }
                                    client.write_all(&buffer[..count])?;
                                    let total = bytes.fetch_add(count as u64, Ordering::SeqCst)
                                        + count as u64;
                                    if total >= 16 * 1024 * 1024
                                        && interrupted
                                            .compare_exchange(
                                                false,
                                                true,
                                                Ordering::SeqCst,
                                                Ordering::SeqCst,
                                            )
                                            .is_ok()
                                    {
                                        let _ = upstream.shutdown(Shutdown::Both);
                                        let _ = client.shutdown(Shutdown::Both);
                                        break;
                                    }
                                }
                                Ok(())
                            })();
                            let _ = result;
                            let _ = client.shutdown(Shutdown::Both);
                        });
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(10))
                    }
                    Err(_) => break,
                }
            }
        }));
        relay
    }
}

impl Drop for Relay {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(listener) = self.listener.take() {
            let _ = listener.join();
        }
        for stream in self.streams.lock().unwrap().drain(..) {
            let _ = stream.shutdown(Shutdown::Both);
        }
    }
}
