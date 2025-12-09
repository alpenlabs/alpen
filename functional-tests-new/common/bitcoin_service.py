"""
Bitcoin service wrapper with Bitcoin-specific health checks.
"""

from common.service import ServiceWrapper


class BitcoinServiceWrapper(ServiceWrapper):
    """
    ServiceWrapper for Bitcoin with health check via `getblockchaininfo`.
    """

    def check_health(self) -> bool:
        """
        Check if Bitcoin RPC is ready by calling getblockchaininfo.

        Returns:
            True if Bitcoin is running and RPC responds, False otherwise
        """
        if not self.check_status():
            return False

        try:
            rpc = self.create_rpc()
            # Try calling a simple RPC method
            rpc.proxy.getblockchaininfo()
            return True
        except Exception:
            return False
