"""
Tests mesh topology formation via discv5 discovery.

Verifies that nodes form a mesh network where fullnodes connect to each other,
not just a wheel-and-spoke topology where all connect only to sequencer.
"""

import logging

import flexitest

from common.base_test import AlpenClientTest

logger = logging.getLogger(__name__)

FULLNODE_COUNT = 5
MIN_MESH_PEERS = 2


@flexitest.register
class TestMeshDiscovery(AlpenClientTest):
    """Test that nodes form a mesh topology via discv5 discovery."""

    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env("alpen_client_mesh")

    def main(self, ctx):
        sequencer = self.get_service("sequencer")
        fullnodes = [self.get_service(f"fullnode_{i}") for i in range(FULLNODE_COUNT)]

        # Get node IDs for topology analysis
        seq_id = sequencer.get_enode().split("@")[0].replace("enode://", "")
        fn_ids = set()
        for fn in fullnodes:
            fn_id = fn.get_enode().split("@")[0].replace("enode://", "")
            fn_ids.add(fn_id)

        # Wait for mesh formation
        logger.info("Waiting for mesh discovery...")
        for fn in fullnodes:
            fn.wait_for_peers(MIN_MESH_PEERS, timeout=120)

        # Analyze topology
        mesh_connections = 0
        for i, fn in enumerate(fullnodes):
            peers = fn.get_peers()
            fn_peer_count = 0
            for peer in peers:
                peer_enode = peer.get("enode", "")
                if peer_enode:
                    peer_id = peer_enode.split("@")[0].replace("enode://", "")
                else:
                    peer_id = peer.get("id", "").removeprefix("0x")

                if peer_id != seq_id and peer_id in fn_ids:
                    fn_peer_count += 1

            mesh_connections += fn_peer_count
            logger.info(f"Fullnode {i}: {len(peers)} peers, {fn_peer_count} fullnodes")

        # Verify mesh (not wheel-and-spoke)
        assert mesh_connections > 0, "Wheel-and-spoke detected, expected mesh topology"

        max_connections = FULLNODE_COUNT * (FULLNODE_COUNT - 1)
        mesh_density = mesh_connections / max_connections * 100
        logger.info(f"Mesh density: {mesh_density:.0f}% ({mesh_connections}/{max_connections})")

        # Verify block propagation
        seq_block = sequencer.get_block_number()
        target_block = seq_block + 3

        sequencer.wait_for_block(target_block, timeout=60)
        seq_hash = sequencer.get_block_by_number(target_block)["hash"]

        for i, fn in enumerate(fullnodes):
            fn.wait_for_block(target_block, timeout=60)
            fn_hash = fn.get_block_by_number(target_block)["hash"]
            assert seq_hash == fn_hash, f"Fullnode {i} hash mismatch"

        logger.info(f"Block {target_block} propagated through mesh")
        return True
