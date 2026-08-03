"""
Strata RPC types
"""

from typing import TypedDict

HexBytes32 = str  # TODO(STR-3692): stricter
HexBytes = str


class OLBlockCommitment(TypedDict):
    slot: int
    blkid: HexBytes32


class OLBlockInfo(TypedDict):
    blkid: HexBytes32
    slot: int
    epoch: int
    is_terminal: bool


class EpochCommitment(TypedDict):
    epoch: int
    last_slot: int
    last_blkid: HexBytes32


class ChainSyncStatus(TypedDict):
    tip: OLBlockInfo
    confirmed: EpochCommitment
    finalized: EpochCommitment
    latest: EpochCommitment


class RpcProofState(TypedDict):
    inner_state: HexBytes32
    next_inbox_msg_idx: int


class MsgPayload(TypedDict):
    value: int  # sats
    data: HexBytes


class MessageEntry(TypedDict):
    source: HexBytes32
    incl_epoch: int
    payload: MsgPayload


class ProofState(TypedDict):
    inner_state: HexBytes32
    next_inbox_msg_idx: int


class EpochUpdateData(TypedDict):
    seq_no: int
    next_inbox_msg_idx: int
    new_state_root: HexBytes32 | None
    extra_data: HexBytes
    messages: list[MessageEntry]


class AccountEpochSummary(TypedDict):
    epoch_commitment: EpochCommitment
    prev_epoch_commitment: EpochCommitment
    balance: int
    final_state_root: HexBytes32
    update_inputs: list[EpochUpdateData]
