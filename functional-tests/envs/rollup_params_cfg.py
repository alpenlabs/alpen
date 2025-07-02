from typing import Annotated, Literal, Union

from pydantic import BaseModel, StringConstraints

# A string that optionally starts with 0x, followed by exactly 64 hex characters
StrBuf32 = Annotated[str, StringConstraints(pattern=r"^(0x)?[0-9A-Fa-f]{64}$")]


class CredRule(BaseModel):
    schnorr_key: StrBuf32


class OperatorConfigItem(BaseModel):
    signing_pk: StrBuf32
    wallet_pk: StrBuf32


class OperatorConfig(BaseModel):
    static: list[OperatorConfigItem]

    def get_operators_pubkeys(self) -> list[str]:
        return [operator.wallet_pk for operator in self.static]


class SP1Groth16Verifier(BaseModel):
    model_config = {"extra": "allow"} # Allow any additional fields


class Risc0Groth16Verifier(BaseModel):
    model_config = {"extra": "allow"} # Allow any additional fields

class Sp1RollupVk(BaseModel):
    sp1: SP1Groth16Verifier

class Risc0RollupVk(BaseModel):
    risc0: Risc0Groth16Verifier


RollupVk = Union[Sp1RollupVk, Risc0RollupVk, Literal["native"]]


class ProofPublishModeTimeout(BaseModel):
    timeout: int


ProofPublishMode = Union[Literal["strict"], ProofPublishModeTimeout]


class RollupConfig(BaseModel):
    """
    A rollup params config data-class.
    Can be used to work with config values conveniently.
    """

    rollup_name: str
    block_time: int
    cred_rule: CredRule
    horizon_l1_height: int
    genesis_l1_height: int
    operator_config: OperatorConfig
    evm_genesis_block_hash: StrBuf32
    evm_genesis_block_state_root: StrBuf32
    l1_reorg_safe_depth: int
    target_l2_batch_size: int
    address_length: int
    deposit_amount: int
    rollup_vk: RollupVk
    dispatch_assignment_dur: int
    proof_publish_mode: ProofPublishMode
    max_deposits_in_block: int
    network: str

    # Additional fields that aren't coming from datatool config generation (yet)
    # and has to be supplied manually.
    # TODO(STR-816): make datatool return OPERATOR_FEE from bridge-tx-builder/src/constants.rs
    operator_fee: int = 50_000_000
    # TODO(STR-816): this is currently an inconsistent mess, figure it out.
    # ANYONE_CAN_SPEND_OUTPUT_VALUE (330) in `bridge-tx-builder/src/constants.rs`
    # + 5.5 sats/vB (200 vbytes) according to `MIN_RELAY_FEE`
    # in `bridge-tx-builder/src/constants.rs`
    withdraw_extra_fee: int = int(330 + 5.5 * 200)
