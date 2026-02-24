/// Automatic Tor Hidden Service Generation for LOS Node
///
/// Per spec: "los-node MUST automatically generate a unique Tor Hidden Service
/// (.onion) upon startup."
///
/// This module uses the Tor Control Port protocol (RFC-like, documented at
/// https://spec.torproject.org/control-spec/) to create ephemeral hidden
/// services at runtime — no torrc editing, no Tor restart, no sudo.
///
/// Architecture:
///   1. Connect to Tor control port (default 127.0.0.1:9051)
///   2. Authenticate (cookie-based or HASHEDPASSWORD)
///   3. ADD_ONION to create a v3 hidden service mapping ports
///   4. Persist the ED25519-V3 key to disk for stable .onion across restarts
///   5. Return the .onion address for network registration
///
/// The hidden service is "detached" (Flags=Detach) so it survives if the
/// control connection drops, and is only removed on explicit DEL_ONION or
/// Tor restart.
use std::fmt;
use std::path::{Path, PathBuf};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;

/// Errors from Tor hidden service operations
#[derive(Debug)]
#[allow(dead_code)]
pub enum TorServiceError {
    /// Cannot connect to Tor control port
    ControlPortUnreachable(String),
    /// Authentication failed
    AuthFailed(String),
    /// ADD_ONION command rejected
    OnionCreationFailed(String),
    /// Cookie file unreadable
    CookieReadError(String),
    /// Key persistence error
    KeyPersistError(String),
    /// Protocol error (unexpected response)
    ProtocolError(String),
}

impl fmt::Display for TorServiceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ControlPortUnreachable(e) => write!(f, "Tor control port unreachable: {}", e),
            Self::AuthFailed(e) => write!(f, "Tor authentication failed: {}", e),
            Self::OnionCreationFailed(e) => write!(f, "Tor ADD_ONION failed: {}", e),
            Self::CookieReadError(e) => write!(f, "Tor cookie file error: {}", e),
            Self::KeyPersistError(e) => write!(f, "Tor key persistence error: {}", e),
            Self::ProtocolError(e) => write!(f, "Tor control protocol error: {}", e),
        }
    }
}

impl std::error::Error for TorServiceError {}

/// Result of a successful hidden service creation
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct TorHiddenService {
    /// The v3 .onion address (e.g., "abc...xyz.onion")
    pub onion_address: String,
    /// The ED25519-V3 private key blob (for persistence)
    pub private_key: String,
    /// Port mappings that were configured
    pub port_mappings: Vec<(u16, u16)>,
}

/// Configuration for Tor hidden service auto-generation
#[derive(Debug, Clone)]
pub struct TorServiceConfig {
    /// Tor control port address (default: 127.0.0.1:9051)
    pub control_addr: String,
    /// Path to Tor cookie file for authentication (auto-detected if None)
    pub cookie_path: Option<PathBuf>,
    /// Control port password (alternative to cookie auth)
    pub control_password: Option<String>,
    /// Directory to persist the hidden service key
    pub data_dir: PathBuf,
    /// Port mappings: (virtual_port, local_port)
    /// e.g., [(3030, 3030), (4030, 4030)] for API + P2P
    pub port_mappings: Vec<(u16, u16)>,
}

impl TorServiceConfig {
    /// Load config from environment variables and provided data dir.
    ///
    /// Env vars:
    ///   - LOS_TOR_CONTROL     = control port addr (default: 127.0.0.1:9051)
    ///   - LOS_TOR_COOKIE_PATH = path to Tor cookie file
    ///   - LOS_TOR_CONTROL_PWD = control port password
    pub fn from_env(data_dir: &Path, api_port: u16, p2p_port: u16) -> Self {
        let control_addr =
            std::env::var("LOS_TOR_CONTROL").unwrap_or_else(|_| "127.0.0.1:9051".to_string());

        let cookie_path = std::env::var("LOS_TOR_COOKIE_PATH")
            .ok()
            .map(PathBuf::from)
            .or_else(auto_detect_cookie_path);

        let control_password = std::env::var("LOS_TOR_CONTROL_PWD").ok();

        TorServiceConfig {
            control_addr,
            cookie_path,
            control_password,
            data_dir: data_dir.to_path_buf(),
            port_mappings: vec![(api_port, api_port), (p2p_port, p2p_port)],
        }
    }
}

/// Auto-detect Tor cookie file in common locations
fn auto_detect_cookie_path() -> Option<PathBuf> {
    let candidates = [
        // macOS (Homebrew Tor)
        "/usr/local/var/lib/tor/control_auth_cookie",
        "/opt/homebrew/var/lib/tor/control_auth_cookie",
        // Linux (system Tor)
        "/var/run/tor/control.authcookie",
        "/var/lib/tor/control_auth_cookie",
        // Linux (user Tor Browser)
        "/tmp/tor-browser/Data/Tor/control_auth_cookie",
    ];

    for path in &candidates {
        let p = PathBuf::from(path);
        if p.exists() {
            return Some(p);
        }
    }
    None
}

/// Key file path within the data directory
fn key_file_path(data_dir: &Path) -> PathBuf {
    data_dir.join("tor_hidden_service_key")
}

/// Onion address file path within the data directory
fn onion_file_path(data_dir: &Path) -> PathBuf {
    data_dir.join("tor_onion_address")
}

/// Send a command to the Tor control port and read the response.
///
/// Tor control protocol uses line-based messages:
///   - Replies start with 3-digit status code
///   - "250 " (space) = final line of OK response
///   - "250-" (dash) = continuation line
///   - "5xx " = error
async fn send_command(
    reader: &mut BufReader<tokio::io::ReadHalf<TcpStream>>,
    writer: &mut tokio::io::WriteHalf<TcpStream>,
    command: &str,
) -> Result<Vec<String>, TorServiceError> {
    writer
        .write_all(format!("{}\r\n", command).as_bytes())
        .await
        .map_err(|e| TorServiceError::ProtocolError(format!("Write failed: {}", e)))?;

    let mut lines = Vec::new();
    loop {
        let mut line = String::new();
        let n = reader
            .read_line(&mut line)
            .await
            .map_err(|e| TorServiceError::ProtocolError(format!("Read failed: {}", e)))?;

        if n == 0 {
            return Err(TorServiceError::ProtocolError(
                "Connection closed unexpectedly".to_string(),
            ));
        }

        let trimmed = line.trim_end().to_string();

        // Check for error response
        if trimmed.len() >= 4 {
            let code = &trimmed[..3];
            let separator = &trimmed[3..4];

            if code.starts_with('5') || code.starts_with('4') {
                return Err(TorServiceError::ProtocolError(trimmed));
            }

            lines.push(trimmed.clone());

            // Space after code means final line
            if separator == " " {
                break;
            }
            // Dash means continuation, keep reading
        } else {
            lines.push(trimmed);
        }
    }

    Ok(lines)
}

/// Create or restore a Tor hidden service.
///
/// If a key file exists in `data_dir`, reuses the existing key (stable .onion).
/// Otherwise, generates a new ED25519-V3 key and persists it.
///
/// # Port Mappings
/// Maps `(virtual_port, local_port)` pairs. For a LOS node:
///   - API port (e.g., 3030) → localhost:3030
///   - P2P port (e.g., 4030) → localhost:4030
///
/// The .onion address is the same for both ports.
pub async fn ensure_hidden_service(
    config: &TorServiceConfig,
) -> Result<TorHiddenService, TorServiceError> {
    // 1. Connect to Tor control port
    let stream = TcpStream::connect(&config.control_addr)
        .await
        .map_err(|e| {
            TorServiceError::ControlPortUnreachable(format!(
                "{} (is Tor running with ControlPort enabled?)",
                e
            ))
        })?;

    let (read_half, write_half) = tokio::io::split(stream);
    let mut reader = BufReader::new(read_half);
    let mut writer = write_half;

    // 2. Authenticate
    authenticate(&mut reader, &mut writer, config).await?;
    println!("🧅 Tor control port: authenticated");

    // 3. Check for existing key file → stable .onion across restarts
    let key_path = key_file_path(&config.data_dir);
    let existing_key = if key_path.exists() {
        match tokio::fs::read_to_string(&key_path).await {
            Ok(key) if !key.trim().is_empty() => {
                println!("🧅 Reusing existing Tor hidden service key");
                Some(key.trim().to_string())
            }
            _ => None,
        }
    } else {
        None
    };

    // 4. Build ADD_ONION command
    let key_spec = match &existing_key {
        Some(key) => format!("ED25519-V3:{}", key),
        None => "NEW:ED25519-V3".to_string(),
    };

    let port_args: String = config
        .port_mappings
        .iter()
        .map(|(virt, local)| format!(" Port={},127.0.0.1:{}", virt, local))
        .collect();

    // Flags:
    //   Detach = service persists if control connection drops
    //   DiscardPK = don't echo key back (we already have it or will read from response)
    let flags = if existing_key.is_some() {
        "Flags=Detach,DiscardPK"
    } else {
        "Flags=Detach"
    };

    let command = format!("ADD_ONION {} {}{}", key_spec, flags, port_args);

    let response = send_command(&mut reader, &mut writer, &command)
        .await
        .map_err(|e| {
            TorServiceError::OnionCreationFailed(format!("ADD_ONION command failed: {}", e))
        })?;

    // 5. Parse response
    let mut onion_address = String::new();
    let mut private_key = existing_key.unwrap_or_default();

    for line in &response {
        if let Some(svc_id) = line.strip_prefix("250-ServiceID=") {
            onion_address = format!("{}.onion", svc_id);
        } else if let Some(key) = line.strip_prefix("250-PrivateKey=ED25519-V3:") {
            private_key = key.to_string();
        }
    }

    if onion_address.is_empty() {
        return Err(TorServiceError::OnionCreationFailed(format!(
            "No ServiceID in response: {:?}",
            response
        )));
    }

    // 6. Persist key for stable .onion across restarts
    if !private_key.is_empty() {
        // Ensure data directory exists
        if let Err(e) = tokio::fs::create_dir_all(&config.data_dir).await {
            return Err(TorServiceError::KeyPersistError(format!(
                "Cannot create data dir {:?}: {}",
                config.data_dir, e
            )));
        }

        // Write key file with restrictive permissions
        tokio::fs::write(&key_path, &private_key)
            .await
            .map_err(|e| {
                TorServiceError::KeyPersistError(format!("Cannot write key file: {}", e))
            })?;

        // Set file permissions to owner-only (Unix)
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = std::fs::Permissions::from_mode(0o600);
            if let Err(e) = std::fs::set_permissions(&key_path, perms) {
                eprintln!(
                    "⚠️ Warning: Could not set key file permissions to 0600: {}",
                    e
                );
            }
        }
    }

    // 7. Also persist the onion address for easy reference
    let onion_path = onion_file_path(&config.data_dir);
    let _ = tokio::fs::write(&onion_path, &onion_address).await;

    println!("🧅 Tor hidden service active: {}", onion_address);
    for (virt, local) in &config.port_mappings {
        println!("   {}:{} → 127.0.0.1:{}", onion_address, virt, local);
    }

    Ok(TorHiddenService {
        onion_address,
        private_key,
        port_mappings: config.port_mappings.clone(),
    })
}

/// Authenticate with the Tor control port.
///
/// Tries in order:
///   1. Cookie authentication (if cookie path provided or auto-detected)
///   2. Password authentication (if LOS_TOR_CONTROL_PWD set)
///   3. Null authentication (if Tor is configured with no auth)
async fn authenticate(
    reader: &mut BufReader<tokio::io::ReadHalf<TcpStream>>,
    writer: &mut tokio::io::WriteHalf<TcpStream>,
    config: &TorServiceConfig,
) -> Result<(), TorServiceError> {
    // Try cookie auth first
    if let Some(cookie_path) = &config.cookie_path {
        match std::fs::read(cookie_path) {
            Ok(cookie) => {
                let hex_cookie = hex::encode(&cookie);
                let resp =
                    send_command(reader, writer, &format!("AUTHENTICATE {}", hex_cookie)).await;
                match resp {
                    Ok(lines) if lines.iter().any(|l| l.starts_with("250 ")) => {
                        return Ok(());
                    }
                    Ok(lines) => {
                        eprintln!("⚠️ Cookie auth failed (trying alternatives): {:?}", lines);
                    }
                    Err(e) => {
                        eprintln!("⚠️ Cookie auth error (trying alternatives): {}", e);
                    }
                }
            }
            Err(e) => {
                eprintln!(
                    "⚠️ Cannot read cookie file {:?}: {} (trying alternatives)",
                    cookie_path, e
                );
            }
        }
    }

    // Try password auth
    if let Some(password) = &config.control_password {
        let resp = send_command(reader, writer, &format!("AUTHENTICATE \"{}\"", password)).await;
        match resp {
            Ok(lines) if lines.iter().any(|l| l.starts_with("250 ")) => {
                return Ok(());
            }
            Ok(lines) => {
                eprintln!("⚠️ Password auth failed: {:?}", lines);
            }
            Err(e) => {
                eprintln!("⚠️ Password auth error: {}", e);
            }
        }
    }

    // Try null auth (Tor configured without authentication)
    let resp = send_command(reader, writer, "AUTHENTICATE").await;
    match resp {
        Ok(lines) if lines.iter().any(|l| l.starts_with("250 ")) => {
            return Ok(());
        }
        _ => {}
    }

    Err(TorServiceError::AuthFailed(
        "All authentication methods failed. Configure Tor with CookieAuthentication 1 \
         or HashedControlPassword, then set LOS_TOR_COOKIE_PATH or LOS_TOR_CONTROL_PWD."
            .to_string(),
    ))
}

/// Check if Tor control port is reachable (non-blocking probe).
///
/// Returns true if a TCP connection to the control port succeeds within 1 second.
/// Does NOT authenticate — just checks reachability.
pub async fn is_control_port_available(addr: &str) -> bool {
    matches!(
        tokio::time::timeout(std::time::Duration::from_secs(1), TcpStream::connect(addr),).await,
        Ok(Ok(_))
    )
}

/// Remove a previously created hidden service.
///
/// Sends DEL_ONION to the control port. The service_id is the .onion
/// address WITHOUT the ".onion" suffix.
#[allow(dead_code)]
pub async fn remove_hidden_service(
    control_addr: &str,
    cookie_path: Option<&Path>,
    control_password: Option<&str>,
    service_id: &str,
) -> Result<(), TorServiceError> {
    let stream = TcpStream::connect(control_addr)
        .await
        .map_err(|e| TorServiceError::ControlPortUnreachable(e.to_string()))?;

    let (read_half, write_half) = tokio::io::split(stream);
    let mut reader = BufReader::new(read_half);
    let mut writer = write_half;

    let config = TorServiceConfig {
        control_addr: control_addr.to_string(),
        cookie_path: cookie_path.map(|p| p.to_path_buf()),
        control_password: control_password.map(|s| s.to_string()),
        data_dir: PathBuf::from("/tmp"), // unused for DEL_ONION
        port_mappings: vec![],
    };
    authenticate(&mut reader, &mut writer, &config).await?;

    // Strip .onion suffix if present
    let svc_id = service_id.trim_end_matches(".onion");
    send_command(&mut reader, &mut writer, &format!("DEL_ONION {}", svc_id))
        .await
        .map_err(|e| TorServiceError::OnionCreationFailed(format!("DEL_ONION failed: {}", e)))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_auto_detect_cookie_path_returns_option() {
        // Should return Some if any candidate path exists, None otherwise.
        // This is environment-dependent, so just verify it doesn't panic.
        let result = auto_detect_cookie_path();
        // On CI or systems without Tor, this will be None — that's fine.
        println!("auto_detect_cookie_path: {:?}", result);
    }

    #[test]
    fn test_key_file_path() {
        let dir = PathBuf::from("/tmp/los-test");
        assert_eq!(
            key_file_path(&dir),
            PathBuf::from("/tmp/los-test/tor_hidden_service_key")
        );
    }

    #[test]
    fn test_onion_file_path() {
        let dir = PathBuf::from("/tmp/los-test");
        assert_eq!(
            onion_file_path(&dir),
            PathBuf::from("/tmp/los-test/tor_onion_address")
        );
    }

    #[test]
    fn test_tor_service_config_defaults() {
        // SAFETY: Test runs single-threaded (cargo test default)
        unsafe {
            std::env::remove_var("LOS_TOR_CONTROL");
            std::env::remove_var("LOS_TOR_COOKIE_PATH");
            std::env::remove_var("LOS_TOR_CONTROL_PWD");
        }

        let config = TorServiceConfig::from_env(Path::new("/tmp/los-test"), 3030, 4030);
        assert_eq!(config.control_addr, "127.0.0.1:9051");
        assert_eq!(config.port_mappings, vec![(3030, 3030), (4030, 4030)]);
        assert!(config.control_password.is_none());
    }

    #[test]
    fn test_tor_service_config_from_env() {
        // SAFETY: Test runs single-threaded (cargo test default)
        unsafe {
            std::env::set_var("LOS_TOR_CONTROL", "127.0.0.1:9151");
            std::env::set_var("LOS_TOR_CONTROL_PWD", "mypassword");
        }

        let config = TorServiceConfig::from_env(Path::new("/tmp/los-data"), 3031, 4031);
        assert_eq!(config.control_addr, "127.0.0.1:9151");
        assert_eq!(config.control_password.as_deref(), Some("mypassword"));
        assert_eq!(config.port_mappings, vec![(3031, 3031), (4031, 4031)]);

        // Clean up
        unsafe {
            std::env::remove_var("LOS_TOR_CONTROL");
            std::env::remove_var("LOS_TOR_CONTROL_PWD");
        }
    }

    #[tokio::test]
    async fn test_control_port_unreachable() {
        // Port 19999 should not have a Tor control port
        let available = is_control_port_available("127.0.0.1:19999").await;
        assert!(!available);
    }

    #[test]
    fn test_tor_service_error_display() {
        let err = TorServiceError::ControlPortUnreachable("connection refused".to_string());
        assert!(err.to_string().contains("connection refused"));

        let err = TorServiceError::AuthFailed("bad cookie".to_string());
        assert!(err.to_string().contains("bad cookie"));

        let err = TorServiceError::OnionCreationFailed("552 Invalid key".to_string());
        assert!(err.to_string().contains("552"));

        let err = TorServiceError::KeyPersistError("permission denied".to_string());
        assert!(err.to_string().contains("permission denied"));

        let err = TorServiceError::ProtocolError("unexpected EOF".to_string());
        assert!(err.to_string().contains("unexpected EOF"));

        let err = TorServiceError::CookieReadError("not found".to_string());
        assert!(err.to_string().contains("not found"));
    }

    #[test]
    fn test_hidden_service_result_clone() {
        let hs = TorHiddenService {
            onion_address: "abc123.onion".to_string(),
            private_key: "testkey".to_string(),
            port_mappings: vec![(3030, 3030)],
        };
        let cloned = hs.clone();
        assert_eq!(cloned.onion_address, "abc123.onion");
        assert_eq!(cloned.port_mappings.len(), 1);
    }
}
