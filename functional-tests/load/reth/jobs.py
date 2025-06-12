from locust import task

from utils.transaction import EthTransactions, TransactionType, TransferTransaction

from .reth import BaseRethLoadJob


class BasicRethBlockJob(BaseRethLoadJob):
    """
    Basic job that displays block information - number of transactions.
    """

    def before_start(self):
        super().before_start()
        self._block_number = 0

    @task
    def get_block(self):
        block_number = self.w3.eth.get_block_number()
        for i in range(self._block_number + 1, block_number + 1):
            try:
                block = self.w3.eth.get_block(hex(i))
            except Exception:
                break
            self._logger.info(f"NEW BLOCK: num={i} => tx_count={len(block['transactions'])}")
            self._block_number = i


class BasicRethTxJob(BaseRethLoadJob):
    """
    Basic job that generates the load - transfers, ERC20 mints, smart contract calls.
    """

    def before_start(self):
        super().before_start()

        self._acc = self.new_account()

        self.tx = EthTransactions(self._acc, self._logger.info)
        self.transfer = TransferTransaction(self._acc, self._logger.info)

    def after_start(self):
        super().after_start()

        tx = self.tx

        # Deploy Counter.
        tx.deploy_contract("Counter.sol", "Counter", "Counter")

        # Deploy EGM and SUSD
        tx.deploy_contract("ERC20.sol", "ERC20", "EGM", "EndGameMoney", "EGM")
        tx.deploy_contract("ERC20.sol", "ERC20", "SUSD", "StrataUSD", "SUSD")

        # Deploy Uniswap.
        tx.deploy_contract("Uniswap.sol", "UniswapFactory", "UniswapFactory")
        tx.deploy_contract(
            "Uniswap.sol",
            "UniswapRouter",
            "Uniswap",
            tx.get_contract_address("UniswapFactory"),
        )

        # Mint some EGM and SUSD tokens.
        tx.mint_erc20("EGM", 1_000_000)
        tx.mint_erc20("SUSD", 1_000_000)

        uniswap_addr = tx.get_contract_address("Uniswap")
        egm_token_addr = tx.get_contract_address("EGM")
        susd_token_addr = tx.get_contract_address("SUSD")

        # Approve spending tokens to Uniswap (standard ERC20 approve).
        tx.approve_spend("EGM", uniswap_addr, 1_000_000)
        tx.approve_spend("SUSD", uniswap_addr, 1_000_000)

        # Add liquidity to uniswap liquidity pair (since we approved spending).
        tx.add_liquidity(egm_token_addr, 100_000, susd_token_addr, 100_000)

        # Swap SUSD to EGM (FOMO IS REAL).
        tx.swap(susd_token_addr, egm_token_addr, 500)

    @task
    def transactions_task(self):
        target_address = self.tx.w3.eth.account.create().address

        # Couple of transfers with different tx types.
        self.transfer.transfer(target_address, 0.1, TransactionType.LEGACY)
        self.transfer.transfer(target_address, 0.1, TransactionType.EIP2930)
        self.transfer.transfer(target_address, 0.1, TransactionType.EIP1559)

        # Increment Counter.
        self.tx.call_contract("Counter", "increment")

        # Mint some SUSD.
        self.tx.mint_erc20("SUSD", 100)

        self._logger.info("task completed successfully.")
