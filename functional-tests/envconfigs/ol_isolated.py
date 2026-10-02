"""OL environment with a genesis snark account for deposit and withdrawal tests."""

from common.config import EpochSealingConfig
from common.config.params import GenesisAccountData
from envconfigs.strata import StrataEnvConfig


class OlIsolatedEnvConfig(StrataEnvConfig):
    """Start an independent OL chain so bridge deposit indices cannot leak between tests."""

    def __init__(self):
        super().__init__(
            pre_generate_blocks=110,
            genesis_accounts={
                "00" * 31 + "42": GenesisAccountData(
                    predicate="Bip340Schnorr:4d4b6cd1361032ca9bd2aeb9d900aa4d45d9ead80ac9423374c451a7254d0766",
                    inner_state="00" * 32,
                    balance=0,
                )
            },
            epoch_sealing=EpochSealingConfig(slots_per_epoch=5),
        )
