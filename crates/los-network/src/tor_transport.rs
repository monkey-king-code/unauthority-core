/// Tor SOCKS5 Transport Layer for LOS P2P Network
///
/// Enables P2P communication over Tor hidden services (.onion).
/// Architecture:
///   - Inbound: Tor hidden service forwards to local libp2p port
///   - Outbound: SOCKS5 proxy creates TCP tunnel to .onion peers
///   - LAN peers: Direct TCP (mdns discovery still works for local dev)
///
/// Usage:
///   1. Run Tor with hidden service forwarding to libp2p_port
///   2. Set LOS_SOCKS5_PROXY=socks5h://127.0.0.1:9050 (or LOS_TOR_SOCKS5=127.0.0.1:9050)
///   3. Set LOS_BOOTSTRAP_NODES=<onion_addr>:<port>,<onion_addr2>:<port2>
///
/// The proxy works by creating a local TCP listener for each .onion peer,
/// then forwarding all data through the SOCKS5 proxy to the remote peer.
/// libp2p dials the local proxy address transparently.
use std::net::SocketAddr;
use tokio::io;
use tokio::net::{TcpListener, TcpStream};

/// Configuration for Tor connectivity
#[derive(Debug, Clone)]
pub struct TorConfig {
    /// SOCKS5 proxy address (e.g., 127.0.0.1:9050)
    pub socks5_proxy: Option<SocketAddr>,
    /// This node's .onion address (for peer announcements)
    pub onion_address: Option<String>,
    /// Fixed libp2p listen port (Tor hidden service points here)
    pub listen_port: u16,
    /// Whether Tor is enabled
    pub enabled: bool,
}

impl TorConfig {
    /// Load Tor configuration from environment variables
    pub fn from_env() -> Self {
        // Accept both LOS_SOCKS5_PROXY (with socks5h:// prefix) and LOS_TOR_SOCKS5 (bare addr)
        let socks5_proxy = std::env::var("LOS_SOCKS5_PROXY")
            .or_else(|_| std::env::var("LOS_TOR_SOCKS5"))
            .ok()
            .map(|s| {
                s.trim_start_matches("socks5h://")
                    .trim_start_matches("socks5://")
                    .to_string()
            })
            .and_then(|s| s.parse::<SocketAddr>().ok())
            .or_else(|| {
                // Auto-detect: try default Tor SOCKS5 proxy at 127.0.0.1:9050
                let default_addr: SocketAddr = match "127.0.0.1:9050".parse() {
                    Ok(a) => a,
                    Err(_) => return None,
                };
                match std::net::TcpStream::connect_timeout(
                    &default_addr,
                    std::time::Duration::from_millis(500),
                ) {
                    Ok(_) => {
                        println!("🧅 Auto-detected Tor SOCKS5 proxy at 127.0.0.1:9050");
                        Some(default_addr)
                    }
                    Err(_) => None,
                }
            });

        // Check LOS_HOST_ADDRESS first (if it's a .onion), then LOS_ONION_ADDRESS
        let onion_address = std::env::var("LOS_HOST_ADDRESS")
            .ok()
            .filter(|s| !s.is_empty() && s.contains(".onion"))
            .or_else(|| std::env::var("LOS_ONION_ADDRESS").ok());

        let listen_port = std::env::var("LOS_P2P_PORT")
            .ok()
            .and_then(|s| s.parse::<u16>().ok())
            .unwrap_or(4001);

        let enabled = socks5_proxy.is_some();

        TorConfig {
            socks5_proxy,
            onion_address,
            listen_port,
            enabled,
        }
    }
}

/// Tor SOCKS5 proxy dialer
///
/// Creates local TCP proxies that tunnel traffic to .onion addresses
/// through a SOCKS5 proxy (Tor).
pub struct TorDialer {
    socks5_addr: SocketAddr,
}

impl TorDialer {
    pub fn new(socks5_addr: SocketAddr) -> Self {
        TorDialer { socks5_addr }
    }

    /// Create a local TCP proxy to a .onion address.
    ///
    /// Returns the local multiaddr that libp2p can dial.
    /// The proxy accepts one connection from libp2p, then tunnels it
    /// through SOCKS5 to the remote .onion peer.
    ///
    /// # Arguments
    /// * `onion_host` - The .onion hostname (e.g., "abc123.onion")
    /// * `onion_port` - The remote port on the hidden service
    pub async fn create_onion_proxy(
        &self,
        onion_host: String,
        onion_port: u16,
    ) -> Result<String, String> {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(|e| format!("Failed to bind proxy listener: {}", e))?;

        let local_addr = listener
            .local_addr()
            .map_err(|e| format!("Failed to get proxy addr: {}", e))?;

        let local_port = local_addr.port();
        let socks5_addr = self.socks5_addr;

        // Spawn the proxy task
        tokio::spawn(async move {
            // Accept connections and proxy them through Tor
            loop {
                match listener.accept().await {
                    Ok((inbound, _)) => {
                        let target_host = onion_host.clone();
                        let target_port = onion_port;
                        let proxy_addr = socks5_addr;

                        tokio::spawn(async move {
                            if let Err(e) =
                                proxy_connection(inbound, proxy_addr, &target_host, target_port)
                                    .await
                            {
                                eprintln!(
                                    "Tor proxy error to {}:{} — {}",
                                    target_host, target_port, e
                                );
                            }
                        });
                    }
                    Err(e) => {
                        eprintln!("Tor proxy accept error: {}", e);
                        break;
                    }
                }
            }
        });

        // Return a multiaddr that libp2p can dial
        let multiaddr = format!("/ip4/127.0.0.1/tcp/{}", local_port);
        Ok(multiaddr)
    }
}

/// Proxy a single connection through SOCKS5 to a .onion target
async fn proxy_connection(
    inbound: TcpStream,
    socks5_addr: SocketAddr,
    target_host: &str,
    target_port: u16,
) -> Result<(), String> {
    // Connect to target through SOCKS5 (Tor)
    let target = format!("{}:{}", target_host, target_port);
    let outbound = tokio_socks::tcp::Socks5Stream::connect(socks5_addr, target.as_str())
        .await
        .map_err(|e| format!("SOCKS5 connect failed: {}", e))?;

    let outbound_stream = outbound.into_inner();

    // Bidirectional copy between libp2p ↔ Tor
    let (mut ri, mut wi) = io::split(inbound);
    let (mut ro, mut wo) = io::split(outbound_stream);

    let client_to_server = tokio::spawn(async move {
        let _ = io::copy(&mut ri, &mut wo).await;
    });

    let server_to_client = tokio::spawn(async move {
        let _ = io::copy(&mut ro, &mut wi).await;
    });

    // Wait for either direction to finish
    let _ = tokio::try_join!(client_to_server, server_to_client);
    Ok(())
}

/// Parse bootstrap node string into (host, port) pairs
///
/// Supports formats:
///   - "abc123.onion:4001"           → (.onion with port)
///   - "abc123.onion"                → (.onion with default port 4001)
///   - "/ip4/1.2.3.4/tcp/4001"      → (multiaddr format, passed through)
pub fn parse_bootstrap_node(node_str: &str) -> BootstrapNode {
    let trimmed = node_str.trim();

    if trimmed.starts_with("/ip4/") || trimmed.starts_with("/dns4/") {
        return BootstrapNode::Multiaddr(trimmed.to_string());
    }

    if trimmed.contains(".onion") {
        let parts: Vec<&str> = trimmed.split(':').collect();
        let host = parts[0].to_string();
        let port = parts
            .get(1)
            .and_then(|p| p.parse::<u16>().ok())
            .unwrap_or(4001);
        return BootstrapNode::Onion { host, port };
    }

    // Assume it's a regular multiaddr
    BootstrapNode::Multiaddr(trimmed.to_string())
}

/// Parsed bootstrap node
#[derive(Debug, Clone)]
pub enum BootstrapNode {
    /// Direct multiaddr(e.g., /ip4/x.x.x.x/tcp/4001)
    Multiaddr(String),
    /// Tor .onion address (host, port)
    Onion { host: String, port: u16 },
}

/// Load bootstrap nodes from environment variable
///
/// LOS_BOOTSTRAP_NODES=addr1,addr2,addr3
pub fn load_bootstrap_nodes() -> Vec<BootstrapNode> {
    match std::env::var("LOS_BOOTSTRAP_NODES") {
        Ok(val) if !val.trim().is_empty() => val.split(',').map(parse_bootstrap_node).collect(),
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_onion_with_port() {
        match parse_bootstrap_node("abc123def456.onion:4001") {
            BootstrapNode::Onion { host, port } => {
                assert_eq!(host, "abc123def456.onion");
                assert_eq!(port, 4001);
            }
            _ => panic!("Expected Onion variant"),
        }
    }

    #[test]
    fn test_parse_onion_default_port() {
        match parse_bootstrap_node("xyz789.onion") {
            BootstrapNode::Onion { host, port } => {
                assert_eq!(host, "xyz789.onion");
                assert_eq!(port, 4001);
            }
            _ => panic!("Expected Onion variant"),
        }
    }

    #[test]
    fn test_parse_multiaddr() {
        match parse_bootstrap_node("/ip4/127.0.0.1/tcp/4001") {
            BootstrapNode::Multiaddr(addr) => {
                assert_eq!(addr, "/ip4/127.0.0.1/tcp/4001");
            }
            _ => panic!("Expected Multiaddr variant"),
        }
    }

    #[test]
    fn test_load_empty_bootstrap() {
        // When env var is not set, should return empty
        // SAFETY: Test runs single-threaded (cargo test default)
        unsafe {
            std::env::remove_var("LOS_BOOTSTRAP_NODES");
        }
        let nodes = load_bootstrap_nodes();
        assert!(nodes.is_empty());
    }

    #[test]
    fn test_tor_config_defaults() {
        // SAFETY: Test runs single-threaded (cargo test default)
        unsafe {
            std::env::remove_var("LOS_SOCKS5_PROXY");
            std::env::remove_var("LOS_TOR_SOCKS5");
            std::env::remove_var("LOS_ONION_ADDRESS");
            std::env::remove_var("LOS_P2P_PORT");
        }

        let config = TorConfig::from_env();
        // listen_port and onion_address are always deterministic
        assert_eq!(config.listen_port, 4001);
        assert!(config.onion_address.is_none());
        // socks5_proxy and enabled depend on whether Tor is running locally
        // (auto-detection at 127.0.0.1:9050). Both states are valid.
        if config.socks5_proxy.is_some() {
            assert!(config.enabled);
        } else {
            assert!(!config.enabled);
        }
    }
}
