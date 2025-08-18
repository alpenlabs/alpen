#!/usr/bin/env python3

"""
Debug script to compare all key sources and identify the mismatch
"""

import flexitest
import os
import re

from mixins import bridge_mixin
from utils import wait_until, get_bridge_pubkey
from strata_utils import (test_key_aggregation_comparison, convert_to_xonly_pk)

@flexitest.register
class DepositKeyMismatchTest(bridge_mixin.BridgeMixin):
    """
    Simple Bridge Test
    """

    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env("basic")

    def main(self, ctx: flexitest.InitContext):
        seq = ctx.get_service("sequencer")
        seqrpc = seq.create_rpc()

        wait_until(
            lambda: self.seqrpc.strata_syncStatus() is not None,
            error_with="Genesis did not happen in time",
            )

        op_pks_from_seq = seqrpc.strata_getActiveOperatorChainPubkeySet()
        print(f"==================================")
        print(f"Operator pubkeys: {op_pks_from_seq}")

        # pks (not xonlypubkeys)
        op_pks = [op_pks_from_seq[str(i)] for i in range(len(op_pks_from_seq))]

        op_x_only_pks = [convert_to_xonly_pk(pk) for pk in op_pks]
        print(op_x_only_pks)

        bridge_pk = get_bridge_pubkey(self.seqrpc)
        print("---------------------------")
        print(f"Bridge PK: {bridge_pk}")
        print("---------------------------")


        # second is reading
        path = os.path.join(ctx.datadir_root, "basic" ,"_init")
        priv_keys = []
        opkeys = sorted(
                filter(lambda file: file.startswith("opkey"), os.listdir(path)),
                key=lambda x: int(''.join(filter(str.isdigit, x))))
        print(opkeys)

        for filename in opkeys:
            full_path = os.path.join(path, filename)
            with open(full_path, "r") as f:
                content = f.read().strip()
                priv_keys.append(content)


        print("=== AGGREGATION COMPARISON ===")
        print(priv_keys)

        # Test config keys vs python received keys
        test_key_aggregation_comparison(priv_keys)

