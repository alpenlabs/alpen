import json
import os
from typing import Any

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
        return self.__run_dbtool_json_command("get-l1-summary", "-o", "json")

    def get_l2_summary(self) -> dict[str, Any]:
        """Get L2 summary and return parsed data"""
        return self.__run_dbtool_json_command("get-l2-summary", "-o", "json")

    def get_checkpoints_summary(self) -> dict[str, Any]:
        """Get checkpoints summary and return parsed data"""
        return self.__run_dbtool_json_command("get-checkpoints-summary", "-o", "json")

    def get_checkpoint(self, checkpoint_index: int) -> dict[str, Any]:
        """Get checkpoint data for specific index and return parsed data"""
        return self.__run_dbtool_json_command("get-checkpoint", str(checkpoint_index), "-o", "json")

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
        """Get sequencer datadir and validate it exists"""
        datadir = self.seq.datadir_path()

        if not os.path.exists(datadir):
            self.error(f"Sequencer datadir does not exist: {datadir}")
            raise FileNotFoundError(f"Sequencer datadir does not exist: {datadir}")

        return datadir
