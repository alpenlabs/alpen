import re
import subprocess

import flexitest
from web3 import Web3

from envs import testenv
from utils import wait_until


@flexitest.register
class P2PGossipTest(testenv.StrataTestBase):
    def __init__(self, ctx: flexitest.InitContext):
        # Use P2PGossipEnvConfig to get actual RLPx P2P peering between nodes
        ctx.set_env(testenv.P2PGossipEnvConfig(pre_generate_blocks=110))

    def main(self, ctx: flexitest.RunContext):
        """
        Test for verifying the head gossip RLPx subprotocol between Reth nodes.

        This test verifies that the custom head_gossip protocol is working by:
        1. Setting up a sequencer and follower network with actual P2P peering
        2. Waiting for blocks to be produced on the sequencer
        3. Verifying the follower receives blocks
        4. Checking logs for actual P2P gossip message exchange
        """

        seq_reth = ctx.get_service("seq_reth")
        follower_reth = ctx.get_service("follower_1_reth")

        seq_web3: Web3 = seq_reth.create_web3()
        fn_web3: Web3 = follower_reth.create_web3()

        # Wait for the sequencer to produce some blocks
        # Use a longer timeout since block production depends on sequencer + signer startup
        reth_waiter = self.create_reth_waiter(seq_reth.create_rpc(), timeout=45)
        self.info("Waiting for sequencer to produce blocks...")
        reth_waiter.wait_until_eth_block_exceeds(3)

        seq_block = seq_web3.eth.block_number
        self.info(f"Sequencer is at block {seq_block}")

        # The follower should eventually sync up with the sequencer
        def follower_synced():
            fn_block = fn_web3.eth.block_number
            self.info(f"Follower is at block {fn_block}, sequencer at {seq_block}")
            return fn_block >= seq_block - 2

        wait_until(
            follower_synced,
            timeout=60,
            error_with="Follower did not sync with sequencer",
        )

        fn_block = fn_web3.eth.block_number
        self.info(f"Follower synced to block {fn_block}")

        # Verify the follower has the same block hash as sequencer for a given block
        check_block = min(seq_block, fn_block) - 1
        if check_block > 0:
            seq_hash = seq_web3.eth.get_block(check_block)["hash"].hex()
            fn_hash = fn_web3.eth.get_block(check_block)["hash"].hex()
            self.info(f"Block {check_block} hash - Sequencer: {seq_hash}, Follower: {fn_hash}")
            assert seq_hash == fn_hash, f"Block hash mismatch at block {check_block}"

        # Check logs for head gossip messages
        broadcast_count, received_count, broadcast_hashes, received_hashes = (
            self._analyze_gossip_logs(ctx)
        )

        self.info(f"Head gossip broadcasts: {broadcast_count}, received: {received_count}")
        self.info(f"Unique broadcast hashes: {len(broadcast_hashes)}")
        self.info(f"Unique received hashes: {len(received_hashes)}")

        # Verify head gossip protocol is active (broadcasts should happen)
        assert broadcast_count > 0, "No head-gossip broadcasts found in logs"
        assert received_count > 0, "No head-gossip received messages found in logs"

        # Verify that received hashes are a subset of broadcast hashes
        # (nodes receive what other nodes broadcast)
        if received_hashes and broadcast_hashes:
            # Find hashes that were received but never broadcast (would indicate corruption)
            unexpected_hashes = received_hashes - broadcast_hashes
            if unexpected_hashes:
                self.warning(f"Found {len(unexpected_hashes)} hashes received but not broadcast")
                for h in list(unexpected_hashes)[:5]:
                    self.warning(f"  Unexpected hash: {h}")

            # Find common hashes (successful gossip)
            common_hashes = received_hashes & broadcast_hashes
            self.info(f"Verified {len(common_hashes)} hashes match between broadcast and receive")

            # At least some hashes should match
            assert len(common_hashes) > 0, "No matching hashes between broadcast and receive"

            # Log a few examples of matching hashes
            for h in list(common_hashes)[:3]:
                self.info(f"  Verified hash: {h}")

        self.info("P2P gossip test passed!")
        return True

    def _analyze_gossip_logs(
        self, ctx: flexitest.RunContext
    ) -> tuple[int, int, set[str], set[str]]:
        """
        Analyze log files for head gossip activity.

        Returns:
            tuple: (broadcast_count, received_count, broadcast_hashes, received_hashes)
        """
        broadcast_count = 0
        received_count = 0
        broadcast_hashes: set[str] = set()
        received_hashes: set[str] = set()

        try:
            # Search for gossip log entries
            result = subprocess.run(
                ["grep", "-r", "head-gossip", ctx.datadir_root],
                capture_output=True,
                text=True,
            )

            if result.returncode == 0:
                log_lines = result.stdout.strip().split("\n")

                for line in log_lines:
                    self.debug(f"Gossip log: {line}")

                    # Count and extract broadcasts
                    # Format: "New block committed: 0x..., broadcasting to N peers"
                    if "broadcasting to" in line:
                        broadcast_count += 1
                        # Extract peer count from "broadcasting to N peers"
                        match = re.search(r"broadcasting to (\d+) peers", line)
                        if match:
                            peer_count = int(match.group(1))
                            if peer_count > 0:
                                self.info(f"Found broadcast to {peer_count} peers")

                        # Extract the block hash being broadcast
                        hash_match = re.search(r"New block committed: (0x[a-fA-F0-9]+)", line)
                        if hash_match:
                            broadcast_hashes.add(hash_match.group(1).lower())

                    # Count and extract received messages
                    # Format: "Received head hash from peer 0x...: 0x..."
                    if "Received head hash from peer" in line:
                        received_count += 1
                        # Extract the received block hash (last 0x... on the line)
                        hash_match = re.search(
                            r"Received head hash from peer.*: (0x[a-fA-F0-9]+)$", line
                        )
                        if hash_match:
                            received_hashes.add(hash_match.group(1).lower())

                    if "Received" in line and "head hashes from peer" in line:
                        received_count += 1

                self.info(f"Found {len(log_lines)} head-gossip log entries")
            else:
                self.warning("No head-gossip log entries found")

        except Exception as e:
            self.warning(f"Could not analyze logs: {e}")

        return broadcast_count, received_count, broadcast_hashes, received_hashes
