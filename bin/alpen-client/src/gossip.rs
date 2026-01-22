//! Gossip event handling task for managing peer connections and broadcasting blocks.

use std::collections::HashMap;

use alpen_ee_common::BlockNumHash;
#[cfg(feature = "sequencer")]
use alpen_reth_node::AlpenGossipMessage;
use alpen_reth_node::{AlpenGossipCommand, AlpenGossipEvent, AlpenGossipPackage};
use reth_network_api::PeerId;
use reth_primitives::Header;
use reth_provider::CanonStateNotification;
use strata_acct_types::Hash;
use strata_primitives::buf::Buf32;
use tokio::{
    select,
    sync::{broadcast, mpsc, watch},
};
use tracing::{debug, error, info, warn};

/// Configuration for the gossip task.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct GossipConfig {
    /// Sequencer's public key for signature validation.
    pub sequencer_pubkey: Buf32,

    /// Whether the local node should produce and sign gossip messages.
    pub sequencer_enabled: bool,

    /// Sequencer's private key for signing (only in sequencer mode).
    #[cfg(feature = "sequencer")]
    pub sequencer_privkey: Option<Buf32>,
}

/// Handles a gossip event (connection established/closed or package received).
fn handle_gossip_event(
    event: AlpenGossipEvent,
    connections: &mut HashMap<PeerId, mpsc::UnboundedSender<AlpenGossipCommand>>,
    highest_seq_no: &mut u64,
    preconf_tx: &watch::Sender<BlockNumHash>,
    config: &GossipConfig,
) -> bool {
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
            true
        }
        AlpenGossipEvent::Closed { peer_id } => {
            debug!(
                target: "alpen-gossip",
                %peer_id,
                "Gossip connection closed"
            );
            connections.remove(&peer_id);
            true
        }
        AlpenGossipEvent::Package { peer_id, package } => handle_gossip_package(
            peer_id,
            package,
            connections,
            highest_seq_no,
            preconf_tx,
            config,
        ),
    }
}

/// Handles a received gossip package.
fn handle_gossip_package(
    peer_id: PeerId,
    package: AlpenGossipPackage,
    connections: &HashMap<PeerId, mpsc::UnboundedSender<AlpenGossipCommand>>,
    highest_seq_no: &mut u64,
    preconf_tx: &watch::Sender<BlockNumHash>,
    config: &GossipConfig,
) -> bool {
    // Validate signature before processing
    if !package.validate_signature() {
        error!(
            target: "alpen-gossip",
            %peer_id,
            "Received gossip package with invalid signature"
        );
        return true; // Continue loop
    }

    // Verify the public key matches the expected sequencer public key
    if package.public_key() != &config.sequencer_pubkey {
        error!(
            target: "alpen-gossip",
            %peer_id,
            "Received gossip package from unexpected public key"
        );
        return true; // Continue loop
    }

    let seq_no = package.message().seq_no();

    // Check if already seen using sequence number (dedup).
    // Since seq_no is the block number and blocks are produced monotonically by the sequencer,
    // we only need to check if this seq_no is greater than the highest we've seen.
    // This prevents duplicate messages and replay of stale blocks.
    if seq_no <= *highest_seq_no {
        debug!(
            target: "alpen-gossip",
            %peer_id,
            seq_no,
            highest_seq_no = *highest_seq_no,
            "Package already seen or stale, skipping"
        );
        return true; // Continue loop
    }

    // Update the highest sequence number seen
    *highest_seq_no = seq_no;

    let block_hash = package.message().header().hash_slow();

    info!(
        target: "alpen-gossip",
        %peer_id,
        ?block_hash,
        seq_no,
        "Received gossip package"
    );

    // Forward the block hash and number to engine control task for fork choice update
    let hash = Hash::from(block_hash.0);
    let block_number = package.message().header().number;
    let blocknumhash = BlockNumHash::new(hash, block_number);
    if preconf_tx.send(blocknumhash).is_err() {
        warn!(
            target: "alpen-gossip",
            "Failed to forward block hash to engine control (no receivers)"
        );
    }

    // Re-broadcast to all OTHER peers (exclude sender)
    for (other_peer_id, sender) in connections {
        if other_peer_id == &peer_id {
            continue;
        }
        if sender
            .send(AlpenGossipCommand::SendPackage(package.clone()))
            .is_err()
        {
            warn!(
                target: "alpen-gossip",
                %other_peer_id,
                "Failed to re-broadcast to peer"
            );
        }
    }

    true // Continue loop
}

/// Handles a canonical state notification.
///
/// Returns `true` to continue the loop, `false` to break.
fn handle_state_event(
    res: Result<CanonStateNotification, broadcast::error::RecvError>,
    connections: &HashMap<PeerId, mpsc::UnboundedSender<AlpenGossipCommand>>,
    config: &GossipConfig,
) -> bool {
    match res {
        Ok(event) => {
            if let CanonStateNotification::Commit { new } = event {
                // Extract the last header from the new chain segment
                if let Some(tip) = new.headers().last().map(|h| h.header().clone()) {
                    broadcast_new_block(&tip, connections, config);
                }
            }
            true // Continue loop
        }
        Err(broadcast::error::RecvError::Lagged(n)) => {
            warn!(
                target: "alpen-gossip",
                lagged = n,
                "Canonical state subscription lagged"
            );
            true // Continue loop
        }
        Err(broadcast::error::RecvError::Closed) => {
            false // Break loop
        }
    }
}

/// Broadcasts a new canonical block to all connected peers.
fn broadcast_new_block(
    tip: &Header,
    connections: &HashMap<PeerId, mpsc::UnboundedSender<AlpenGossipCommand>>,
    config: &GossipConfig,
) {
    if !config.sequencer_enabled {
        return;
    }

    info!(
        target: "alpen-gossip",
        block_hash = ?tip.hash_slow(),
        block_number = tip.number,
        peer_count = connections.len(),
        "Broadcasting new block to peers"
    );

    #[cfg(feature = "sequencer")]
    {
        let Some(sequencer_privkey) = config.sequencer_privkey else {
            error!(
                target: "alpen-gossip",
                "Sequencer mode enabled but no private key configured; skipping broadcast"
            );
            return;
        };

        let msg = AlpenGossipMessage::new(
            tip.clone(),
            // NOTE: we use the block number as the sequence number
            //       because it's the block number from the header, which naturally
            //       provides monotonic, unique sequence numbers for gossip messages.
            tip.number,
        );
        let pkg = msg.into_package(config.sequencer_pubkey, sequencer_privkey);

        for (peer_id, sender) in connections {
            if sender
                .send(AlpenGossipCommand::SendPackage(pkg.clone()))
                .is_err()
            {
                warn!(
                    target: "alpen-gossip",
                    %peer_id,
                    "Failed to send message to peer"
                );
            }
        }
    }
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
    preconf_tx: watch::Sender<BlockNumHash>,
    config: GossipConfig,
) {
    let mut connections: HashMap<PeerId, mpsc::UnboundedSender<AlpenGossipCommand>> =
        HashMap::new();
    // Track the highest sequence number (block number) seen from the sequencer.
    // This prevents duplicate and stale message processing since blocks are produced
    // monotonically.
    let mut highest_seq_no: u64 = 0;

    loop {
        select! {
            Some(event) = gossip_rx.recv() => {
                handle_gossip_event(
                    event,
                    &mut connections,
                    &mut highest_seq_no,
                    &preconf_tx,
                    &config,
                );
            },
            res = state_events.recv() => {
                if !handle_state_event(res, &connections, &config) {
                    break;
                }
            },
            else => { break; }
        }
    }
}
