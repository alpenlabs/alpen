from typing import List, Optional, TypedDict


HexBytes32 = str  # TODO: stricter
HexBytes = str


class OLBlockCommitment(TypedDict):
    slot: int
    blkid: HexBytes32


class EpochCommitment(TypedDict):
    epoch: int
    last_slot: int
    last_blkid: HexBytes32


class ChainSyncStatus(TypedDict):
    latest: OLBlockCommitment
    confirmed: EpochCommitment
    finalized: EpochCommitment


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


class UpdateInputData(TypedDict):
    seq_no: int
    proof_state: ProofState
    extra_data: HexBytes
    messages: List[MessageEntry]


class AccountEpochSummary(TypedDict):
    epoch_commitment: EpochCommitment
    prev_epoch_commitment: EpochCommitment
    balance: int
    update_input: Optional[UpdateInputData]
