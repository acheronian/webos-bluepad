//! Diagnostics export delivery (spec: Diagnostics and observability;
//! user story 36): the bundle assembled by the daemon crosses the seam,
//! the UI adds its own warn/error log, and delivery is screen-bounded
//! token-URL LAN HTTP (primary, both tiers) plus USB write on the root
//! tier.

use bluepad_core::diagnostics::{bundle_tar, scrub_macs, DiagnosticsBundle, DiagnosticsFile};
use std::io::{Read, Write};
use std::net::{IpAddr, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// The delivered archive's file name.
pub const BUNDLE_FILENAME: &str = "bluepad-diagnostics.tar";
/// How long the LAN export stays live while the screen is open.
pub const EXPORT_LIFETIME: Duration = Duration::from_secs(10 * 60);

/// Finalize the bundle for delivery: add the UI's own warn/error log
/// (MACs scrubbed — no address survives in the clear), then pack the
/// versioned manifest + logs as a tar archive.
pub fn finalize_bundle(bundle: &DiagnosticsBundle, ui_log: &[u8]) -> Vec<u8> {
    let mut bundle = bundle.clone();
    bundle.files.push(DiagnosticsFile {
        name: "logs/bluepad-ui.log".into(),
        content: scrub_macs(ui_log, &[]),
    });
    bundle_tar(&bundle)
}

/// The TV's LAN address, as a browser on the same network would reach it:
/// the local address of a socket routed off-host (no packet is sent).
/// Falls back to loopback when there is no route (offline TV).
pub fn local_lan_ip() -> IpAddr {
    use std::net::UdpSocket;
    let fallback = IpAddr::V4(std::net::Ipv4Addr::LOCALHOST);
    let Ok(socket) = UdpSocket::bind("0.0.0.0:0") else {
        return fallback;
    };
    // A connect on a UDP socket sends nothing; it only fixes the route.
    if socket.connect("8.8.8.8:80").is_err() {
        return fallback;
    }
    socket.local_addr().map(|a| a.ip()).unwrap_or(fallback)
}

/// A screen-bounded token-URL LAN HTTP server: it serves the bundle at a
/// URL only the screen knows, shuts down on its timer, and stops when
/// the screen closes (the exporter is dropped).
pub struct LanExporter {
    url: String,
    stop: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
}

struct SharedExport {
    tar: Vec<u8>,
    token: String,
    deadline: std::time::Instant,
}

impl LanExporter {
    /// Bind and start serving. `bind` is the interface to listen on
    /// (usually unspecified = all interfaces); `display` is the address
    /// the URL shows (the TV's LAN address).
    pub fn start(
        bind: IpAddr,
        display: IpAddr,
        port: u16,
        tar: Vec<u8>,
        lifetime: Duration,
    ) -> std::io::Result<LanExporter> {
        let listener = TcpListener::bind(SocketAddr::new(bind, port))?;
        let addr = listener.local_addr()?;
        let token = generate_token();
        let url = format!("http://{display}:{port}/{token}/", port = addr.port());
        let stop = Arc::new(AtomicBool::new(false));
        let shared = Arc::new(SharedExport {
            tar,
            token,
            deadline: std::time::Instant::now() + lifetime,
        });

        let accept_stop = stop.clone();
        let handle = std::thread::spawn(move || {
            listener
                .set_nonblocking(true)
                .expect("nonblocking listener");
            loop {
                if accept_stop.load(Ordering::Relaxed)
                    || std::time::Instant::now() > shared.deadline
                {
                    break; // the screen closed or the timer ran out
                }
                match listener.accept() {
                    Ok((stream, _)) => {
                        let shared = shared.clone();
                        std::thread::spawn(move || serve_connection(stream, &shared));
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(250));
                    }
                    Err(_) => break,
                }
            }
        });

        Ok(LanExporter {
            url,
            stop,
            handle: Some(handle),
        })
    }

    /// The URL the export screen shows — token included, bounded lifetime.
    pub fn url(&self) -> &str {
        &self.url
    }
}

/// Serve one HTTP request: the token gates everything, the two routes
/// are the index page and the bundle download. No filesystem is touched
/// — unknown paths are a plain 404.
fn serve_connection(mut stream: TcpStream, shared: &SharedExport) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let mut request = Vec::new();
    let mut chunk = [0u8; 1024];
    // Read until the end of the request headers (bounded).
    'read: loop {
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                request.extend_from_slice(&chunk[..n]);
                if request.windows(4).any(|w| w == b"\r\n\r\n") || request.len() > 8192 {
                    break 'read;
                }
            }
            Err(_) => return,
        }
    }
    let request_line = String::from_utf8_lossy(&request);
    let path = request_line.split_whitespace().nth(1).unwrap_or("").to_string();

    let index_path = format!("/{}/", shared.token);
    let bundle_path = format!("/{}/{}", shared.token, BUNDLE_FILENAME);
    if path == bundle_path {
        respond(
            &mut stream,
            "200 OK",
            "application/x-tar",
            &shared.tar,
        );
    } else if path == index_path {
        let page = format!(
            "<!doctype html>\n<title>BluePad diagnostics</title>\n\
             <h1>BluePad diagnostics</h1>\n\
             <p>BluePad's anonymized diagnostics bundle — logs, environment\n\
             snapshot, health counters, and a pseudonymized pad registry\n\
             summary.</p>\n\
             <p><a href=\"{BUNDLE_FILENAME}\">Download {BUNDLE_FILENAME}</a></p>\n"
        );
        respond(&mut stream, "200 OK", "text/html; charset=utf-8", page.as_bytes());
    } else {
        respond(
            &mut stream,
            "404 Not Found",
            "text/plain; charset=utf-8",
            b"not found\n",
        );
    }
}

fn respond(stream: &mut TcpStream, status: &str, content_type: &str, body: &[u8]) {
    let _ = write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(body);
    let _ = stream.flush();
}

impl Drop for LanExporter {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

/// Where webOS mounts USB drives (env-overridable for dev and tests).
pub fn usb_mount_root() -> std::path::PathBuf {
    std::env::var("BLUEPAD_USB_MOUNTS")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::path::PathBuf::from("/tmp/usbmounts"))
}

/// The mounted USB drives under a mount root: one directory per drive.
pub fn find_usb_mounts(root: &std::path::Path) -> Vec<std::path::PathBuf> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut mounts: Vec<std::path::PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    mounts.sort();
    mounts
}

/// Write the bundle archive onto a mounted USB drive. Returns the file
/// written.
pub fn write_bundle_to_usb(
    tar: &[u8],
    mount: &std::path::Path,
    now_unix: u64,
) -> std::io::Result<std::path::PathBuf> {
    let path = mount.join(format!("bluepad-diagnostics-{now_unix}.tar"));
    std::fs::write(&path, tar)?;
    Ok(path)
}

/// The export token: 32 hex chars from the system's randomness, so the
/// URL is unguessable while the screen is up. Falls back to hashing
/// pid + time when no system randomness is reachable.
fn generate_token() -> String {
    let mut bytes = [0u8; 16];
    if std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut bytes))
        .is_err()
    {
        // Fallback: mix pid and nanos through FNV-1a.
        let mut hash = 0x2166_1362_u64;
        for byte in format!(
            "{}{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        )
        .bytes()
        {
            hash ^= byte as u64;
            hash = hash.wrapping_mul(0x0100_0000_01b3);
        }
        bytes[..8].copy_from_slice(&hash.to_le_bytes());
        bytes[8..].copy_from_slice(&(hash ^ 0x9e37_79b9_7f4a_7c15).to_le_bytes());
    }
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use bluepad_core::bdaddr::BdAddr;
    use bluepad_core::diagnostics::pseudonymize;
    use std::net::Ipv4Addr;

    fn sample_addr() -> BdAddr {
        "a0:fa:9c:62:7d:1a".parse().unwrap()
    }

    fn sample_bundle() -> DiagnosticsBundle {
        DiagnosticsBundle {
            manifest: bluepad_core::diagnostics::BundleManifest {
                schema_version: 1,
                app_version: "0.1.0".into(),
                protocol_version: 2,
                generated_unix: 0,
                tier: bluepad_core::protocol::PrivilegeTier::Root,
                capabilities: vec![],
                health: Default::default(),
                pads: vec![],
            },
            files: vec![DiagnosticsFile {
                name: "logs/bluepad-daemon.log".into(),
                content: b"daemon log line\n".to_vec(),
            }],
        }
    }

    #[test]
    fn finalize_adds_the_ui_log_and_packs_the_tar() {
        let tar = finalize_bundle(
            &sample_bundle(),
            b"[1] [WARN] ui log line, pad a0:fa:9c:62:7d:1a failed\n",
        );
        // The tar carries the manifest, the daemon log, and the UI log.
        let hay = String::from_utf8_lossy(&tar).to_string();
        assert!(hay.contains("manifest.json"));
        assert!(hay.contains("logs/bluepad-daemon.log"));
        assert!(hay.contains("logs/bluepad-ui.log"), "the UI log rides along");
        assert!(tar.windows(16).any(|w| w == b"daemon log line\n"));
        let needle = b"[1] [WARN] ui log line, pad ";
        assert!(tar.windows(needle.len()).any(|w| w == needle));
        // No MAC in the clear: the UI log is scrubbed like the rest.
        assert!(!hay.contains("a0:fa:9c:62:7d:1a"), "{hay}");
        let pseudonym = pseudonymize(&sample_addr());
        assert!(hay.contains(&pseudonym), "the pseudonym keeps the OUI: {pseudonym}");
    }

    // -- The fake HTTP client: minimal GET over loopback TCP ------------

    fn http_get(addr: SocketAddr, path: &str) -> std::io::Result<(u16, Vec<u8>)> {
        let mut stream = TcpStream::connect(addr)?;
        stream.write_all(format!("GET {path} HTTP/1.1\r\nHost: tv\r\n\r\n").as_bytes())?;
        let mut response = Vec::new();
        stream.read_to_end(&mut response)?;
        let text = String::from_utf8_lossy(&response);
        let status: u16 = text
            .split_whitespace()
            .nth(1)
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        // Body: everything after the blank line ending the headers.
        let header_end = response
            .windows(4)
            .position(|w| w == b"\r\n\r\n")
            .map(|i| i + 4)
            .unwrap_or(response.len());
        let body = response[header_end..].to_vec();
        Ok((status, body))
    }

    #[allow(clippy::type_complexity)]
    fn start_test_exporter() -> (LanExporter, SocketAddr, String) {
        let tar = finalize_bundle(&sample_bundle(), b"");
        let exporter = LanExporter::start(
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            0,
            tar.clone(),
            Duration::from_secs(60),
        )
        .expect("exporter starts");
        let url = exporter.url().to_string();
        assert!(url.starts_with("http://127.0.0.1:"), "{url}");
        let (host, path) = url
            .trim_start_matches("http://")
            .split_once('/')
            .expect("host and path");
        let addr: SocketAddr = host.parse().expect("the URL carries host and port");
        let base = format!("/{}", path.trim_end_matches('/'));
        (exporter, addr, base)
    }

    #[test]
    fn the_token_url_serves_the_bundle() {
        let (_exporter, addr, base) = start_test_exporter();
        let (status, body) =
            http_get(addr, &format!("{base}/{BUNDLE_FILENAME}")).expect("request served");
        assert_eq!(status, 200);
        assert!(
            body.windows(5).any(|w| w == b"ustar"),
            "the delivered file is the tar archive"
        );
    }

    #[test]
    fn the_index_page_links_the_download() {
        let (_exporter, addr, base) = start_test_exporter();
        let (status, body) = http_get(addr, &format!("{base}/")).expect("index served");
        assert_eq!(status, 200);
        let page = String::from_utf8_lossy(&body);
        assert!(page.contains("BluePad diagnostics"), "{page}");
        assert!(page.contains(BUNDLE_FILENAME), "the download is linked");
    }

    #[test]
    fn a_wrong_or_missing_token_is_refused() {
        let (_exporter, addr, _base) = start_test_exporter();
        let (status, _) = http_get(addr, "/no-such-token/bluepad-diagnostics.tar").unwrap();
        assert_eq!(status, 404);
        let (status, _) = http_get(addr, "/").unwrap();
        assert_eq!(status, 404);
        // Path traversal never reaches the filesystem: it is a 404.
        let (status, _) = http_get(addr, "/../../etc/passwd").unwrap();
        assert_eq!(status, 404);
    }

    #[test]
    fn dropping_the_exporter_shuts_the_server_down() {
        let (exporter, addr, _base) = start_test_exporter();
        drop(exporter);
        assert!(TcpStream::connect(addr).is_err(), "the screen closed");
    }

    #[test]
    fn the_exporter_shuts_down_on_its_timer() {
        let tar = finalize_bundle(&sample_bundle(), b"");
        let exporter = LanExporter::start(
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            0,
            tar,
            Duration::from_millis(150),
        )
        .expect("exporter starts");
        let addr: SocketAddr = exporter
            .url()
            .trim_start_matches("http://")
            .trim_end_matches('/')
            .split('/')
            .next()
            .unwrap()
            .parse()
            .unwrap();
        // The server stops accepting once the bounded lifetime is spent.
        let mut served = false;
        for _ in 0..50 {
            if TcpStream::connect(addr).is_err() {
                served = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(served, "the timer shut the export down");
        drop(exporter);
    }

    #[test]
    fn tokens_are_hex_and_unique_per_export() {
        let a = generate_token();
        let b = generate_token();
        assert_eq!(a.len(), 32);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, b);
    }

    // -- USB seam: a fake mount root ------------------------------------

    fn fake_mount_root() -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!("bluepad-usb-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("USB Drive 1")).unwrap();
        std::fs::create_dir_all(root.join("usb2")).unwrap();
        std::fs::write(root.join("not-a-mount.txt"), "").unwrap();
        root
    }

    #[test]
    fn usb_mounts_are_the_directories_under_the_mount_root() {
        let root = fake_mount_root();
        let mounts = find_usb_mounts(&root);
        let names: Vec<String> = mounts
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into())
            .collect();
        assert_eq!(names.len(), 2, "{names:?}");
        assert!(names.contains(&"USB Drive 1".to_string()));
        assert!(names.contains(&"usb2".to_string()));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_bundle_lands_on_the_usb_drive_with_a_dated_name() {
        let root = fake_mount_root();
        let mount = find_usb_mounts(&root).remove(0);
        let tar = b"tar-bytes".to_vec();
        let written = write_bundle_to_usb(&tar, &mount, 1_700_000_123).unwrap();
        assert_eq!(
            written,
            mount.join("bluepad-diagnostics-1700000123.tar"),
            "the file lands in the mount with the timestamp in its name"
        );
        assert_eq!(std::fs::read(&written).unwrap(), tar);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_mount_root_is_env_overridable() {
        // The default is webOS's usbmounts dir; dev and tests override it.
        std::env::set_var("BLUEPAD_USB_MOUNTS", "/tmp/bluepad-test-mounts");
        assert_eq!(usb_mount_root(), std::path::PathBuf::from("/tmp/bluepad-test-mounts"));
        std::env::remove_var("BLUEPAD_USB_MOUNTS");
        assert_eq!(usb_mount_root(), std::path::PathBuf::from("/tmp/usbmounts"));
    }

    #[test]
    fn the_lan_ip_is_some_routable_address() {
        let ip = local_lan_ip();
        assert!(!ip.is_unspecified(), "the URL needs a real address");
    }
}
