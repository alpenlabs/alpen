import json
import os
from typing import Any

from envs.testenv import StrataTestBase
from utils.dbtool import extract_json_from_output, run_dbtool_command

from . import BaseMixin


class DbtoolMixin(BaseMixin):
    """Mixin providing dbtool command execution utilities"""

    def get_syncinfo(self) -> dict[str, Any]:
        """Get sync information and return parsed data"""
        return self.__run_dbtool_json_command("get-syncinfo", "-o", "json")

    def get_l2_block(self, block_id: str) -> dict[str, Any]:
        """Get L2 block data and return parsed data"""
        return self.__run_dbtool_json_command("get-l2-block", "-o", "json", block_id)

    def get_chainstate(self, block_id: str) -> dict[str, Any]:
        """Get chainstate for specific block and return parsed data"""
        return self.__run_dbtool_json_command("get-chainstate", block_id, "-o", "json")

    def get_l1_summary(self) -> dict[str, Any]:
        """Get L1 summary and return parsed data"""
        genesis_height = str(self.ctx.env.rollup_cfg().genesis_l1_view.height())
        return self.__run_dbtool_json_command("get-l1-summary", genesis_height, "-o", "json")

    def get_l2_summary(self) -> dict[str, Any]:
        """Get L2 summary and return parsed data"""
        return self.__run_dbtool_json_command("get-l2-summary", "-o", "json")

    def get_checkpoints_summary(self) -> dict[str, Any]:
        """Get checkpoints summary and return parsed data"""
        genesis_height = str(self.ctx.env.rollup_cfg().genesis_l1_view.height())
        return self.__run_dbtool_json_command(
            "get-checkpoints-summary", genesis_height, "-o", "json"
        )

    def get_checkpoint(self, checkpoint_index: int) -> dict[str, Any]:
        """Get checkpoint data for specific index and return parsed data"""
        return self.__run_dbtool_json_command("get-checkpoint", str(checkpoint_index), "-o", "json")

    def get_epoch_summary(self, epoch_index: int) -> dict[str, Any]:
        """Get epoch summary for specific epoch index and return parsed data"""
        return self.__run_dbtool_json_command("get-epoch-summary", str(epoch_index), "-o", "json")

    def revert_chainstate(self, block_id: str, *args) -> tuple[int, str, str]:
        """Run revert-chainstate command and return (return_code, stdout, stderr)"""
        datadir = self.__get_datadir()
        return run_dbtool_command(datadir, "revert-chainstate", block_id, *args)

    def __run_dbtool_json_command(self, subcommand: str, *args) -> dict[str, Any]:
        """Execute dbtool command and return parsed JSON result"""
        datadir = self.__get_datadir()

        return_code, stdout, stderr = run_dbtool_command(datadir, subcommand, *args)

        if return_code != 0:
            self.error(f"{subcommand} failed with return code {return_code}")
            self.error(f"Stderr: {stderr}")
            raise RuntimeError(f"dbtool {subcommand} failed: {stderr}")

        try:
            json_output = extract_json_from_output(stdout)
            if not json_output:
                self.error(f"No JSON found in stdout: {stdout}")
                raise ValueError(f"No JSON found in {subcommand} output")

            return json.loads(json_output)

        except json.JSONDecodeError as e:
            self.error(f"Invalid JSON from {subcommand}: {e}")
            raise ValueError(f"Invalid JSON from {subcommand}: {e}") from e

    def __get_datadir(self) -> str:
        datadir = self._get_dbtool_datadir()
        if not os.path.exists(datadir):
            self.error(f"Datadir does not exist: {datadir}")
            raise FileNotFoundError(f"Datadir does not exist: {datadir}")
        return datadir

    def _get_dbtool_datadir(self) -> str:
        """Subclasses must implement this to return the appropriate datadir path."""
        raise NotImplementedError("Subclasses must implement _get_dbtool_datadir()")


class SequencerDbtoolMixin(DbtoolMixin):
    def _get_dbtool_datadir(self) -> str:
        return self.seq.datadir_path()


class FullnodeDbtoolMixin(DbtoolMixin):
    """Dbtool mixin for fullnode tests that uses follower_1_node datadir."""

    def premain(self, ctx):
        """Override premain to set up fullnode services instead of sequencer services."""
        StrataTestBase.premain(self, ctx)
        self._ctx = ctx

        # Set up fullnode-specific services (available in HubNetworkEnvConfig)
        self.btc = ctx.get_service("bitcoin")
        self.seq = ctx.get_service("seq_node")
        self.seq_signer = ctx.get_service("sequencer_signer")
        self.reth = ctx.get_service("seq_reth")
        self.follower_1_node = ctx.get_service("follower_1_node")
        self.follower_1_reth = ctx.get_service("follower_1_reth")

        # Create RPC connections
        self.seqrpc = self.seq.create_rpc()
        self.btcrpc = self.btc.create_rpc()
        self.rethrpc = self.reth.create_rpc()
        self.web3 = self.reth.create_web3()
        self.follower_1_rpc = self.follower_1_node.create_rpc()
        self.follower_1_reth_rpc = self.follower_1_reth.create_rpc()

    def _get_dbtool_datadir(self) -> str:
        """Get fullnode datadir for dbtool operations."""
        return self.follower_1_node.datadir_path()
