use libp2p::{
    gossipsub, identify, kad, noise, tcp, yamux, Multiaddr, PeerId, Swarm,
};
use libp2p::swarm::{NetworkBehaviour, SwarmEvent};
use futures_util::StreamExt;
use std::time::Duration;
use tokio::sync::mpsc;

// Global Default Seed Nodes (Automatic Peer Mesh Entrypoint)
pub const DEFAULT_GLOBAL_BOOTSTRAP_NODES: &[&str] = &[
    "/ip4/32.236.238.232/tcp/9000", // CoreByte Mainnet Master Seed Node
];

#[derive(NetworkBehaviour)]
pub struct CoreByteBehaviour {
    pub gossipsub: gossipsub::Behaviour,
    pub kademlia: kad::Behaviour<kad::store::MemoryStore>,
    pub identify: identify::Behaviour,
}

pub struct P2PCommand {
    pub topic: String,
    pub message: Vec<u8>,
}

pub async fn start_libp2p_node(
    port: u16,
    custom_bootstraps: Vec<String>,
    mut cmd_rx: mpsc::Receiver<P2PCommand>,
    event_tx: mpsc::Sender<String>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut swarm = libp2p::SwarmBuilder::with_new_identity()
        .with_tokio()
        .with_tcp(
            tcp::Config::default(),
            noise::Config::new,
            yamux::Config::default,
        )?
        .with_behaviour(|key| {
            let local_peer_id = PeerId::from(key.public());

            // Gossipsub setup
            let gossipsub_config = gossipsub::ConfigBuilder::default()
                .heartbeat_interval(Duration::from_millis(500))
                .validation_mode(gossipsub::ValidationMode::Permissive)
                .build()
                .map_err(|msg| std::io::Error::new(std::io::ErrorKind::Other, msg))?;

            let mut gossipsub = gossipsub::Behaviour::new(
                gossipsub::MessageAuthenticity::Signed(key.clone()),
                gossipsub_config,
            )?;

            let block_topic = gossipsub::IdentTopic::new("block_planetary_gossip");
            let tx_topic = gossipsub::IdentTopic::new("tx_planetary_gossip");
            gossipsub.subscribe(&block_topic)?;
            gossipsub.subscribe(&tx_topic)?;

            // Kademlia DHT setup
            let store = kad::store::MemoryStore::new(local_peer_id);
            let mut kad_config = kad::Config::default();
            kad_config.set_query_timeout(Duration::from_secs(5));
            let mut kademlia = kad::Behaviour::with_config(local_peer_id, store, kad_config);
            kademlia.set_mode(Some(kad::Mode::Server));

            // Identify setup
            let identify = identify::Behaviour::new(identify::Config::new(
                "/corebyte/kad/1.0.0".to_string(),
                key.public(),
            ));

            Ok(CoreByteBehaviour {
                gossipsub,
                kademlia,
                identify,
            })
        })?
        .with_swarm_config(|c| c.with_idle_connection_timeout(Duration::from_secs(60)))
        .build();

    let listen_addr: Multiaddr = format!("/ip4/0.0.0.0/tcp/{}", port).parse()?;
    swarm.listen_on(listen_addr)?;

    // Auto-Connect: Seed nodes ko routing table me register karein
    let mut target_bootstraps = custom_bootstraps;
    if target_bootstraps.is_empty() {
        for &seed in DEFAULT_GLOBAL_BOOTSTRAP_NODES {
            target_bootstraps.push(seed.to_string());
        }
    }

    for b in &target_bootstraps {
        if let Ok(addr) = b.parse::<Multiaddr>() {
            let _ = swarm.dial(addr.clone());
        }
    }

    let _ = swarm.behaviour_mut().kademlia.bootstrap();
    println!("[LIBP2P AUTO-DISCOVERY]: DHT Bootstrap Triggered across global mesh.");

    loop {
        tokio::select! {
            Some(cmd) = cmd_rx.recv() => {
                let topic = gossipsub::IdentTopic::new(cmd.topic);
                let _ = swarm.behaviour_mut().gossipsub.publish(topic, cmd.message);
            }
            event = swarm.select_next_some() => {
                match event {
                    SwarmEvent::Behaviour(CoreByteBehaviourEvent::Identify(identify::Event::Received { peer_id, info, .. })) => {
                        println!("[P2P AUTO-PEER]: Discovered Global Node: {} (Protocol: {})", peer_id, info.protocol_version);
                        for addr in info.listen_addrs {
                            swarm.behaviour_mut().kademlia.add_address(&peer_id, addr);
                        }
                    }
                    SwarmEvent::Behaviour(CoreByteBehaviourEvent::Gossipsub(gossipsub::Event::Message { message, .. })) => {
                        if let Ok(msg_str) = String::from_utf8(message.data) {
                            let _ = event_tx.send(msg_str).await;
                        }
                    }
                    SwarmEvent::NewListenAddr { address, .. } => {
                        println!("[LIBP2P LISTENER]: Listening on {}", address);
                    }
                    _ => {}
                }
            }
        }
    }
}
