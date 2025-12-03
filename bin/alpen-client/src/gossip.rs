//! Gossip event handling task for managing peer connections and broadcasting blocks.

use std::collections::HashMap;

use alpen_reth_node::{AlpenGossipCommand, AlpenGossipEvent, AlpenGossipMessage};
use reth_network_api::PeerId;
use reth_primitives::Header;
use reth_provider::CanonStateNotification;
use strata_acct_types::Hash;
use strata_primitives::buf::Buf32;
use tokio::{
    select,
    sync::{broadcast, mpsc},
};
use tracing::{debug, error, info, warn};

/// Configuration for the gossip task.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct GossipConfig {
    /// Sequencer's public key for signature validation.
    pub sequencer_pubkey: Buf32,

    /// Sequencer's private key for signing (only in sequencer mode).
    #[cfg(feature = "sequencer")]
    pub sequencer_privkey: Buf32,
}

/// Creates the gossip event handling task.
///
/// This task manages:
///
/// - Connection tracking (establish/close)
/// - Receiving gossip messages and forwarding block hashes to engine control
/// - Broadcasting new canonical blocks to connected peers
pub(crate) async fn create_gossip_task(
    mut gossip_rx: mpsc::UnboundedReceiver<AlpenGossipEvent>,
    mut state_events: broadcast::Receiver<CanonStateNotification>,
    preconf_tx: broadcast::Sender<Hash>,
    config: GossipConfig,
) {
    let mut connections: HashMap<PeerId, mpsc::UnboundedSender<AlpenGossipCommand>> =
        HashMap::new();

    loop {
        select! {
            Some(event) = gossip_rx.recv() => {
                match event {
                    AlpenGossipEvent::Established {
                        peer_id,
                        direction,
                        to_connection,
                    } => {
                        debug!(
                            target: "alpen-gossip",
                            %peer_id,
                            ?direction,
                            "New gossip connection established"
                        );
                        connections.insert(peer_id, to_connection);
                    }
                    AlpenGossipEvent::Closed { peer_id } => {
                        debug!(
                            target: "alpen-gossip",
                            %peer_id,
                            "Gossip connection closed"
                        );
                        connections.remove(&peer_id);
                    }
                    AlpenGossipEvent::Package { peer_id, package } => {
                        // Validate signature before processing
                        if !package.validate_signature() {
                            error!(
                                target: "alpen-gossip",
                                %peer_id,
                                "Received gossip package with invalid signature"
                            );
                            continue;
                        }

                        // Verify the public key matches the expected sequencer public key
                        if package.public_key() != &config.sequencer_pubkey {
                            error!(
                                target: "alpen-gossip",
                                %peer_id,
                                "Received gossip package from unexpected public key"
                            );
                            continue;
                        }

                        let block_hash = package.message().header().hash_slow();
                        info!(
                            target: "alpen-gossip",
                            %peer_id,
                            ?block_hash,
                            seq_no = package.message().seq_no(),
                            "Received gossip package"
                        );

                        // Forward the block hash to engine control task for fork choice update
                        let hash = Hash::from(block_hash.0);
                        if preconf_tx.send(hash).is_err() {
                            warn!(
                                target: "alpen-gossip",
                                "Failed to forward block hash to engine control (no receivers)"
                            );
                        }
                    }
                }
            },
            res = state_events.recv() => {
                match res {
                    Ok(event) => {
                        if let CanonStateNotification::Commit { new } = event {
                            // Extract headers from the new chain segment
                            let headers: Vec<Header> = new
                                .headers()
                                .map(|h| h.header().clone())
                                .collect();

                            if let Some(tip) = headers.last() {
                                info!(
                                    target: "alpen-gossip",
                                    block_hash = ?tip.hash_slow(),
                                    block_number = tip.number,
                                    peer_count = connections.len(),
                                    "Broadcasting new block to peers"
                                );

                                #[cfg(feature = "sequencer")]
                                {
                                    let msg = AlpenGossipMessage::new(
                                        tip.clone(),
                                        // NOTE: we use the block number as the sequence number
                                        //       because it's the block number from the header, which naturally
                                        //       provides monotonic, unique sequence numbers for gossip messages.
                                        tip.number
                                    );
                                    let pkg = msg.into_package(config.sequencer_pubkey, config.sequencer_privkey);

                                    for (peer_id, sender) in &connections {
                                        if sender.send(AlpenGossipCommand::SendPackage(pkg.clone())).is_err() {
                                            warn!(
                                                target: "alpen-gossip",
                                                %peer_id,
                                                "Failed to send message to peer"
                                            );
                                        }
                                    }
                                }

                                #[cfg(not(feature = "sequencer"))]
                                {
                                    warn!(
                                        target: "alpen-gossip",
                                        "Cannot broadcast: sequencer feature not enabled"
                                    );
                                }
                            }
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        warn!(
                            target: "alpen-gossip",
                            lagged = n,
                            "Canonical state subscription lagged"
                        );
                        continue;
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                        warn!(
                            target: "alpen-gossip",
                            "Canonical state subscription closed"
                        );
                        break;
                    }
                }
            },
            else => { break; }
        }
    }
}
