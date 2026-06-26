"""Helpers for OL account and inbox assertions in functional tests."""

import time

from common.wait import wait_until_with_value


def get_ol_balance(rpc, account_id_hex: str) -> int:
    status = rpc.strata_getChainStatus()
    tip_slot = status["tip"]["slot"]
    summaries = rpc.strata_getBlocksSummaries(account_id_hex, tip_slot, tip_slot)
    return summaries[0]["balance"] if summaries else 0


def wait_for_ol_balance(
    rpc,
    account_id_hex: str,
    expected_sats: int,
    timeout: int = 180,
    btc_rpc=None,
    miner_addr: str | None = None,
) -> None:
    def poll_balance() -> int:
        if btc_rpc is not None and miner_addr is not None:
            btc_rpc.proxy.generatetoaddress(2, miner_addr)
        return get_ol_balance(rpc, account_id_hex)

    wait_until_with_value(
        poll_balance,
        lambda balance: balance == expected_sats,
        error_with=f"account {account_id_hex} did not reach {expected_sats} sats",
        timeout=timeout,
        step=1,
    )


def wait_for_account_update_seq(
    rpc,
    account_id_hex: str,
    min_seq_no: int,
    start_epoch: int,
    btc_rpc,
    miner_addr: str,
    timeout: int = 600,
) -> int:
    deadline = time.time() + timeout
    last_terminal_epoch = start_epoch
    last_seen_seq_no = -1
    while time.time() < deadline:
        btc_rpc.proxy.generatetoaddress(4, miner_addr)
        time.sleep(1)
        status = rpc.strata_getChainStatus()
        last_terminal_epoch = int(status["latest"]["epoch"])
        for epoch in range(start_epoch, last_terminal_epoch + 1):
            try:
                summary = rpc.strata_getAccountEpochSummary(account_id_hex, epoch)
            except Exception:
                continue
            for update in summary.get("update_inputs") or []:
                seq_no = int(update.get("seq_no", -1))
                last_seen_seq_no = max(last_seen_seq_no, seq_no)
                if seq_no >= min_seq_no:
                    return epoch
    raise AssertionError(
        f"account {account_id_hex} update seq_no >= {min_seq_no} not found from epoch "
        f"{start_epoch}; last_terminal_epoch={last_terminal_epoch}, "
        f"last_seen_seq_no={last_seen_seq_no}"
    )


def wait_for_account_update_exact_seq(
    rpc,
    account_id_hex: str,
    expected_seq_no: int,
    start_epoch: int,
    btc_rpc,
    miner_addr: str,
    timeout: int = 600,
) -> int:
    deadline = time.time() + timeout
    last_terminal_epoch = start_epoch
    last_seen_seq_no = -1
    while time.time() < deadline:
        btc_rpc.proxy.generatetoaddress(4, miner_addr)
        time.sleep(1)
        status = rpc.strata_getChainStatus()
        last_terminal_epoch = int(status["latest"]["epoch"])
        for epoch in range(start_epoch, last_terminal_epoch + 1):
            try:
                summary = rpc.strata_getAccountEpochSummary(account_id_hex, epoch)
            except Exception:
                continue
            for update in summary.get("update_inputs") or []:
                seq_no = int(update.get("seq_no", -1))
                last_seen_seq_no = max(last_seen_seq_no, seq_no)
                if seq_no == expected_seq_no:
                    return epoch
    raise AssertionError(
        f"account {account_id_hex} update seq_no {expected_seq_no} not found from epoch "
        f"{start_epoch}; last_terminal_epoch={last_terminal_epoch}, "
        f"last_seen_seq_no={last_seen_seq_no}"
    )


def wait_for_next_ol_epoch(rpc, btc_rpc, miner_addr: str, timeout: int = 180) -> int:
    current_epoch = int(rpc.strata_getChainStatus()["latest"]["epoch"])

    def poll_epoch() -> int:
        btc_rpc.proxy.generatetoaddress(2, miner_addr)
        return int(rpc.strata_getChainStatus()["latest"]["epoch"])

    return wait_until_with_value(
        poll_epoch,
        lambda epoch: epoch > current_epoch,
        error_with=f"OL epoch did not advance past {current_epoch}",
        timeout=timeout,
        step=1,
    )


def count_new_inbox_messages(rpc, account_id_hex: str, start_slot: int) -> int:
    tip_slot = rpc.strata_getChainStatus()["tip"]["slot"]
    summaries = rpc.strata_getBlocksSummaries(account_id_hex, start_slot, tip_slot)
    return sum(len(summary.get("new_inbox_messages") or []) for summary in summaries)


def wait_for_inbox_message_delta(
    rpc,
    account_id_hex: str,
    start_slot: int,
    start_count: int,
    delta: int,
    error_with: str,
) -> None:
    wait_until_with_value(
        lambda: count_new_inbox_messages(rpc, account_id_hex, start_slot),
        lambda count: count >= start_count + delta,
        error_with=error_with,
        timeout=120,
        step=1,
    )


def build_gam_tx(account_id_hex: str, payload_hex: str) -> dict:
    return {
        "payload": {
            "type": "generic_account_message",
            "target": account_id_hex,
            "payload": payload_hex,
        },
        "constraints": {
            "min_slot": None,
            "max_slot": None,
        },
    }
