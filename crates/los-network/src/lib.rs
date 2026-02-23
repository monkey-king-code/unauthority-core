// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// UNAUTHORITY (LOS) - NETWORK MODULE
//
// Peer-to-peer networking layer using libp2p with Tor Hidden Service support.
// - Gossipsub for block/transaction propagation
// - mDNS for local development peer discovery
// - Noise Protocol for encrypted communications
// - Tor SOCKS5 proxy for .onion peer connectivity
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

use libp2p::{
    futures::StreamExt,
    gossipsub, mdns, noise,
    swarm::{behaviour::toggle::Toggle, NetworkBehaviour, SwarmEvent},
    tcp, yamux,
};
use std::error::Error;
use std::time::Duration;
use tokio::sync::mpsc;

// Public modules
pub mod fee_scaling;
pub mod p2p_encryption;
pub mod p2p_integration;
pub mod slashing_integration;
pub mod tor_transport;
pub mod validator_rewards;

pub use tor_transport::{load_bootstrap_nodes, BootstrapNode, TorConfig, TorDialer};

#[derive(Debug)]
pub enum NetworkEvent {
    NewBlock(String),
    PeerDiscovered(String),
}

#[derive(NetworkBehaviour)]
pub struct LosBehaviour {
    pub gossipsub: gossipsub::Behaviour,
    /// mDNS is disabled when Tor is enabled to prevent LAN presence leaks.
    pub mdns: Toggle<mdns::tokio::Behaviour>,
}

pub struct LosNode;

impl LosNode {
    pub async fn start(
        tx: mpsc::Sender<NetworkEvent>,
        mut rx_out: mpsc::Receiver<String>,
    ) -> Result<(), Box<dyn Error>> {
        // Load Tor configuration from environment
        let tor_config = TorConfig::from_env();
        let bootstrap_nodes = load_bootstrap_nodes();

        // Create optional Tor dialer for .onion connections
        let tor_dialer = tor_config.socks5_proxy.map(TorDialer::new);

        if tor_config.enabled {
            println!(
                "🧅 Tor transport enabled (SOCKS5: {})",
                tor_config
                    .socks5_proxy
                    .map(|a| a.to_string())
                    .unwrap_or_default()
            );
            if let Some(ref onion) = tor_config.onion_address {
                println!("🧅 This node's .onion address: {}", onion);
            }
        }

        if !bootstrap_nodes.is_empty() {
            println!("📡 Bootstrap nodes: {}", bootstrap_nodes.len());
        }

        let mut swarm = libp2p::SwarmBuilder::with_new_identity()
            .with_tokio()
            .with_tcp(
                tcp::Config::default(),
                noise::Config::new,
                yamux::Config::default,
            )?
            .with_behaviour(|key| {
                let message_id_fn = |message: &gossipsub::Message| {
                    let mut s = std::collections::hash_map::DefaultHasher::new();
                    std::hash::Hash::hash(&message.data, &mut s);
                    gossipsub::MessageId::from(std::hash::Hasher::finish(&s).to_string())
                };

                let gossipsub_config = gossipsub::ConfigBuilder::default()
                    .heartbeat_interval(Duration::from_secs(1))
                    .validation_mode(gossipsub::ValidationMode::Strict)
                    .message_id_fn(message_id_fn)
                    .max_transmit_size(10 * 1024 * 1024)
                    .build()
                    .map_err(std::io::Error::other)?;

                let gossipsub = gossipsub::Behaviour::new(
                    gossipsub::MessageAuthenticity::Signed(key.clone()),
                    gossipsub_config,
                )?;

                // SECURITY: mDNS leaks node presence on LAN via multicast UDP.
                // When Tor is enabled, disable mDNS to preserve anonymity.
                let mdns = if tor_config.enabled {
                    println!("🔒 mDNS disabled (Tor mode — prevents LAN presence leak)");
                    Toggle::from(None)
                } else {
                    Toggle::from(Some(mdns::tokio::Behaviour::new(
                        mdns::Config::default(),
                        key.public().to_peer_id(),
                    )?))
                };

                Ok(LosBehaviour { gossipsub, mdns })
            })?
            .with_swarm_config(|cfg| cfg.with_idle_connection_timeout(Duration::from_secs(300)))
            .build();

        let topic = gossipsub::IdentTopic::new("los-blocks");
        swarm.behaviour_mut().gossipsub.subscribe(&topic)?;

        // Listen on configured port
        // SECURITY: When Tor SOCKS5 is configured, bind 127.0.0.1 only to prevent IP leaks.
        // The Tor hidden service forwards external traffic to this local port.
        let bind_ip = if tor_config.socks5_proxy.is_some() {
            "127.0.0.1"
        } else {
            "0.0.0.0"
        };
        let listen_addr = format!("/ip4/{}/tcp/{}", bind_ip, tor_config.listen_port);
        swarm.listen_on(listen_addr.parse()?)?;
        println!("📡 P2P listening on port {}", tor_config.listen_port);

        // Bootstrap: dial all configured bootstrap nodes
        for node in &bootstrap_nodes {
            match node {
                BootstrapNode::Multiaddr(addr) => {
                    if let Ok(maddr) = addr.parse::<libp2p::Multiaddr>() {
                        println!("📡 Dialing bootstrap peer: {}", addr);
                        let _ = swarm.dial(maddr);
                    }
                }
                BootstrapNode::Onion { host, port } => {
                    if let Some(ref dialer) = tor_dialer {
                        match dialer.create_onion_proxy(host.clone(), *port).await {
                            Ok(local_addr) => {
                                println!("🧅 Tor proxy created for {} → {}", host, local_addr);
                                if let Ok(maddr) = local_addr.parse::<libp2p::Multiaddr>() {
                                    let _ = swarm.dial(maddr);
                                }
                            }
                            Err(e) => {
                                eprintln!("🧅 Failed to create Tor proxy for {}: {}", host, e);
                            }
                        }
                    } else {
                        eprintln!(
                            "🧅 Cannot dial .onion {} — set LOS_SOCKS5_PROXY=socks5h://127.0.0.1:9050",
                            host
                        );
                    }
                }
            }
        }

        // ── RECONNECTION TIMER ──────────────────────────────────────
        // Tor circuits break after ~10 minutes, killing gossipsub connections.
        // Without reconnection, the mesh collapses and nodes become isolated:
        // no heartbeats, no block gossip, no reward Mint propagation.
        // This timer periodically re-dials bootstrap nodes to maintain the mesh.
        let reconnect_interval_secs = if std::env::var("LOS_TESTNET_LEVEL").is_ok() {
            60
        } else {
            180
        };
        let mut reconnect_timer =
            tokio::time::interval(Duration::from_secs(reconnect_interval_secs));
        reconnect_timer.tick().await; // Consume the first immediate tick
        let mut connected_peers: std::collections::HashSet<libp2p::PeerId> =
            std::collections::HashSet::new();
        let min_peers: usize = bootstrap_nodes.len().max(1);

        loop {
            tokio::select! {
                Some(msg_to_send) = rx_out.recv() => {
                    if let Some(addr_str) = msg_to_send.strip_prefix("DIAL:") {
                        // Check if it's a .onion address
                        if addr_str.contains(".onion") {
                            if let Some(ref dialer) = tor_dialer {
                                let parsed = tor_transport::parse_bootstrap_node(addr_str);
                                if let BootstrapNode::Onion { host, port } = parsed {
                                    match dialer.create_onion_proxy(host.clone(), port).await {
                                        Ok(local_addr) => {
                                            println!("🧅 Tor proxy for {} → {}", host, local_addr);
                                            if let Ok(maddr) = local_addr.parse::<libp2p::Multiaddr>() {
                                                let _ = swarm.dial(maddr);
                                            }
                                        }
                                        Err(e) => eprintln!("🧅 Tor dial failed: {}", e),
                                    }
                                }
                            } else {
                                eprintln!("🧅 Cannot dial .onion — set LOS_SOCKS5_PROXY=socks5h://127.0.0.1:9050");
                            }
                        } else if let Ok(maddr) = addr_str.parse::<libp2p::Multiaddr>() {
                            println!("📡 Swarm: Dialing {}...", maddr);
                            let _ = swarm.dial(maddr);
                        }
                    } else {
                        let msg_prefix = if msg_to_send.len() > 20 { &msg_to_send[..20] } else { &msg_to_send };
                        match swarm.behaviour_mut().gossipsub.publish(topic.clone(), msg_to_send.as_bytes()) {
                            Ok(_mid) => {
                                if msg_to_send.starts_with("CONFIRM_REQ:") || msg_to_send.starts_with("CONFIRM_RES:") {
                                    println!("📤 Gossip PUBLISHED OK: {} ({} bytes, mesh_peers={})", msg_prefix, msg_to_send.len(), connected_peers.len());
                                }
                            }
                            Err(e) => {
                        let err_str = format!("{:?}", e);
                        if msg_to_send.starts_with("CONFIRM_REQ:") || msg_to_send.starts_with("CONFIRM_RES:") {
                            eprintln!("❌ Gossip FAILED for {}: {} (mesh_peers={})", msg_prefix, err_str, connected_peers.len());
                        }
                        if err_str.contains("InsufficientPeers") {
                            // Log periodically instead of silently swallowing
                            static LAST_WARN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
                            let now = std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs();
                            if now.saturating_sub(LAST_WARN.load(std::sync::atomic::Ordering::Relaxed)) >= 30 {
                                LAST_WARN.store(now, std::sync::atomic::Ordering::Relaxed);
                                eprintln!("⚠️ Gossipsub: InsufficientPeers — messages not being delivered (connected: {})", connected_peers.len());
                            }
                        } else {
                            eprintln!("⚠️ Broadcast Error: {:?}", e);
                        }
                    }}
                    }
                },
                // ── RECONNECTION: Re-dial bootstrap nodes when mesh is thin ──
                _ = reconnect_timer.tick() => {
                    let peer_count = connected_peers.len();
                    if peer_count < min_peers {
                        println!("🔄 P2P reconnect: only {}/{} peers connected, re-dialing bootstrap nodes...", peer_count, min_peers);
                        for node in &bootstrap_nodes {
                            match node {
                                BootstrapNode::Multiaddr(addr) => {
                                    if let Ok(maddr) = addr.parse::<libp2p::Multiaddr>() {
                                        let _ = swarm.dial(maddr);
                                    }
                                }
                                BootstrapNode::Onion { host, port } => {
                                    if let Some(ref dialer) = tor_dialer {
                                        match dialer.create_onion_proxy(host.clone(), *port).await {
                                            Ok(local_addr) => {
                                                if let Ok(maddr) = local_addr.parse::<libp2p::Multiaddr>() {
                                                    let _ = swarm.dial(maddr);
                                                }
                                            }
                                            Err(e) => eprintln!("🔄 Reconnect dial failed for {}: {}", host, e),
                                        }
                                    }
                                }
                            }
                        }
                    } else {
                        println!("📡 P2P mesh healthy: {}/{} peers connected", peer_count, min_peers);
                    }
                },
                event = swarm.select_next_some() => match event {
                    SwarmEvent::Behaviour(LosBehaviourEvent::Mdns(mdns::Event::Discovered(list))) => {
                        for (peer_id, _addr) in list {
                            swarm.behaviour_mut().gossipsub.add_explicit_peer(&peer_id);
                            let _ = tx.send(NetworkEvent::PeerDiscovered(peer_id.to_string())).await;
                        }
                    },
                    SwarmEvent::Behaviour(LosBehaviourEvent::Gossipsub(gossipsub::Event::Message { message, .. })) => {
                        let content = String::from_utf8_lossy(&message.data).to_string();
                        let _ = tx.send(NetworkEvent::NewBlock(content)).await;
                    },
                    SwarmEvent::NewListenAddr { address, .. } => {
                        println!("📍 P2P listening on: {:?}", address);
                    },
                    SwarmEvent::ConnectionEstablished { peer_id, .. } => {
                        swarm.behaviour_mut().gossipsub.add_explicit_peer(&peer_id);
                        connected_peers.insert(peer_id);
                        println!("🤝 P2P connected: {:?} (total: {})", peer_id, connected_peers.len());
                    },
                    SwarmEvent::ConnectionClosed { peer_id, cause, .. } => {
                        connected_peers.remove(&peer_id);
                        println!("🔌 P2P disconnected: {:?} (reason: {:?}, remaining: {})", peer_id, cause, connected_peers.len());
                    },
                    SwarmEvent::OutgoingConnectionError { error, .. } => {
                        eprintln!("❌ P2P dial error: {:?}", error);
                    },
                    _ => {}
                }
            }
        }
    }
}
