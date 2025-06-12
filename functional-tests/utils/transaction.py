from enum import Enum
from typing import Callable, TypeAlias

import solcx
import web3
from eth_typing import HexStr
from hexbytes import HexBytes
from web3.types import TxReceipt

from load.reth.log_helper import log_metadata_var, tx_caller

from .evm_account import AbstractAccount


class TransactionType(Enum):
    LEGACY = 1
    EIP2930 = 2
    EIP1559 = 3


# We don't use (a stricter) Tx from web3 because our transactions are
# created partially and filled later according to the transaction type.
Tx: TypeAlias = dict[str, int | str]


class _TransactionFaucet:
    """
    Base class for all transaction mixins that equips those with account and w3.
    Also, plugs in the structured logger.
    """

    def __init__(self, acc: AbstractAccount, logger=None):
        self._acc: AbstractAccount = acc
        self._logger: Callable[[str], None] | None = logger

    @property
    def w3(self) -> web3.Web3:
        return self._acc.w3

    def log(self, msg):
        if self._logger is not None:
            self._logger(msg)


class TransactionBuilder(_TransactionFaucet):
    """
    A transaction mixin responsible for basic transaction payload building.
    Supports Legacy, EIP-2930 and EIP-1559 transaction types.
    """

    def fill_tx_fields(
        self, tx: Tx, tx_type: TransactionType = TransactionType.LEGACY, from_rpc: bool = False
    ):
        """
        Fills all the necessary transaction fields according to the transaction type provided.
        """

        tx.setdefault("from", self._acc.address)
        tx.setdefault("nonce", self._acc.nonce)

        if tx_type == TransactionType.LEGACY:
            tx.setdefault(
                "gasPrice", self.w3.eth.gas_price if not from_rpc else self.w3.to_wei("1", "gwei")
            )
        elif tx_type == TransactionType.EIP2930:
            tx.setdefault("type", "0x1")
            tx.setdefault("chainId", self.w3.eth.chain_id)
            # Define an empty access_list for simplicity for now.
            tx.setdefault("accessList", [{"address": tx["to"], "storageKeys": []}])

            tx.setdefault(
                "gasPrice", self.w3.eth.gas_price if from_rpc else self.w3.to_wei("1", "gwei")
            )

        elif tx_type == TransactionType.EIP1559:
            tx.setdefault("type", "0x2")
            tx.setdefault("chainId", self.w3.eth.chain_id)

            # TODO: use from_rpc to fetch the current fee market if needed.
            # Currently hardcoded.
            tx.setdefault("maxPriorityFeePerGas", self.w3.to_wei("1", "gwei"))
            tx.setdefault("maxFeePerGas", self.w3.to_wei("2", "gwei"))

    @classmethod
    def new_with_gas(cls, gas: int) -> Tx:
        return {"gas": gas}


class TransactionSender(TransactionBuilder):
    """
    A helper that's capable of sending transaction and waiting for the receipt.
    Also, logs all the transactions sent in a nice manner.
    """

    def _tx_fmt(self, tx: Tx):
        gas = tx.get("gasPrice", tx.get("maxFeePerGas"))
        return f"from={tx['from']}, nonce={tx['nonce']}, gas={gas}, gasLimit={tx['gas']}"

    def send_tx(self, tx: Tx) -> HexStr | None:
        logs_caller = log_metadata_var.get()
        self.log(f"Sending tx=[{logs_caller}]: {self._tx_fmt(tx)}")

        try:
            tx_hash = self.w3.eth.send_transaction(tx)
            hash = self.w3.to_hex(tx_hash)
            self.log(f"Transaction sent with hash={hash}")

            return hash
        except Exception as e:
            self.log(f"An exception during transaction send: {e}")
            return None

    def send_tx_and_wait(self, tx: Tx, timeout: int = 10) -> TxReceipt | None:
        logs_caller = log_metadata_var.get()
        self.log(f"Sending tx=[{logs_caller}] with timeout={timeout}: {self._tx_fmt(tx)}")

        try:
            tx_hash = self.w3.eth.send_transaction(tx)

            hash = self.w3.to_hex(tx_hash)
            self.log(f"Transaction sent with hash={hash}")

            receipt = self.w3.eth.wait_for_transaction_receipt(tx_hash, timeout=timeout)
            self.log(f"Transaction hash={hash} was processed, receipt={receipt}")

            return receipt
        except Exception as e:
            self.log(f"An exception during transaction send: {e}")
            return None

    def send_ensured_tx(self, tx: Tx, tx_type: TransactionType) -> HexStr | None:
        self.fill_tx_fields(tx, tx_type)
        return self.send_tx(tx)

    def send_ensured_tx_and_wait(
        self, tx: Tx, tx_type: TransactionType, timeout: int = 10
    ) -> TxReceipt | None:
        self.fill_tx_fields(tx, tx_type)
        return self.send_tx_and_wait(tx, timeout=timeout)

    def wait_tx(self, tx_hash: HexBytes, timeout: int = 10) -> TxReceipt | None:
        try:
            return self.w3.eth.wait_for_transaction_receipt(tx_hash, timeout=timeout)
        except Exception as e:
            print(e)
            return None


class TransferTransaction(TransactionSender):
    """
    A transaction mixin that's capable of transferring funds to another account.
    """

    @tx_caller("TRANSFERRING [2] TO [1]")
    def transfer(
        self, to, value, tx_type: TransactionType, wait=False
    ) -> TxReceipt | HexStr | None:
        tx: Tx = TransactionBuilder.new_with_gas(25000)
        tx.update(
            {
                "to": to,
                "value": self.w3.to_wei(value, "ether"),
            }
        )

        if wait:
            return self.send_ensured_tx_and_wait(tx, tx_type)
        return self.send_ensured_tx(tx, tx_type)


class SmartContracts(TransactionSender):
    """
    Mixin to work with smart contracts.
    """

    CONTRACTS_DIR = "contracts/"
    SOL_VERSION = "0.8.7"

    _smart_contracts_storage = dict()

    @staticmethod
    def extract_compiled_contract(compiled_sol, contract_name: str):
        for ct_id, ct_interface in compiled_sol.items():
            if ct_id.endswith(contract_name):
                return ct_interface["abi"], ct_interface["bin"]
        return None

    @staticmethod
    def compile_contract(filename, contract_name=None):
        if contract_name is None:
            contract_name = filename.split(".")[0]

        solcx.install_solc(SmartContracts.SOL_VERSION)

        compiled_sol = solcx.compile_files(
            [f"{SmartContracts.CONTRACTS_DIR}{filename}"],
            output_values=["abi", "bin"],
            solc_version=SmartContracts.SOL_VERSION,
        )
        return SmartContracts.extract_compiled_contract(compiled_sol, contract_name)

    @tx_caller("DEPLOYING CONTRACT [3]")
    def deploy_contract(self, contract_path, contract_name, contract_id, *args):
        abi, bytecode = SmartContracts.compile_contract(contract_path, contract_name)
        contract = self.w3.eth.contract(abi=abi, bytecode=bytecode)

        tx: Tx = TransactionBuilder.new_with_gas(5_000_000)
        self.fill_tx_fields(tx)

        tx = contract.constructor(*args).build_transaction(tx)
        receipt: TxReceipt = self.send_tx_and_wait(tx)
        self._smart_contracts_storage[contract_id] = (receipt.contractAddress, abi)
        return receipt.contractAddress, abi

    def _contract_abi(self, contract_id):
        return self._smart_contracts_storage.get(contract_id, (None, None))[1]

    def _contract_address(self, contract_id):
        return self._smart_contracts_storage.get(contract_id, (None, None))[0]

    @tx_caller("CALLING CONTRACT [1]")
    def call_contract(
        self, contract_id, function_name, *args, wait=True
    ) -> HexBytes | TxReceipt | None:
        try:
            contract = self.w3.eth.contract(
                address=self._contract_address(contract_id), abi=self._contract_abi(contract_id)
            )
            tx: Tx = TransactionBuilder.new_with_gas(1_000_000)
            self.fill_tx_fields(tx)

            function_tx = contract.functions[function_name](*args).build_transaction(tx)
            if wait:
                return self.send_tx_and_wait(function_tx)
            else:
                return self.send_tx(function_tx)
        except Exception as e:
            print(e)
            return None

    def get_contract_address(self, contract_id):
        return self.w3.to_checksum_address(self._contract_address(contract_id))

    def get_contract_instance(self, contract_id):
        return self.w3.eth.contract(
            address=self._contract_address(contract_id), abi=self._contract_abi(contract_id)
        )


class ERC20(SmartContracts):
    """
    Allows to call ERC20 contract methods.
    """

    @tx_caller("MINTING [2] [1] tokens")
    def mint_erc20(self, token_name, amount, wait=True):
        return self.call_contract(token_name, "mint", self._acc.address, amount, wait=wait)

    @tx_caller("APPROVING SPEND [3] [1] TOKENS FOR [2]")
    def approve_spend(self, token_name, spender, amount, wait=True):
        return self.call_contract(token_name, "approve", spender, amount, wait=wait)


class Uniswap(SmartContracts):
    """
    Allows to call basic uniswap methods.
    """

    @tx_caller("SWAPPING [1] FOR [2], AMOUNT [3]")
    def swap(self, token_in, token_out, amount, wait=True):
        return self.call_contract("Uniswap", "swap", token_in, token_out, amount, wait=wait)

    @tx_caller("ADDING LIQUIDITY FOR [1]/[3] PAIR, AMOUNT [2]/[4]")
    def add_liquidity(self, token_a, amount_a, token_b, amount_b, wait=True):
        return self.call_contract(
            "Uniswap", "addLiquidity", token_a, token_b, amount_a, amount_b, wait=wait
        )


class EthTransactions(
    Uniswap, ERC20, SmartContracts, TransferTransaction, TransactionSender, _TransactionFaucet
):
    """
    A convenient wrapper that enables all mixins altogether.
    """

    pass
