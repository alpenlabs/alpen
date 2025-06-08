import logging

def get_latest_slot(fn_rpc):
    sync_status = fn_rpc.strata_syncStatus()
    return sync_status['tip_height']


def assert_ckpt_and_seq_sync(cs_node_rpc, ss_node_rpc):
    cs_ckpt_idx = cs_node_rpc.strata_getLatestCheckpointIndex()
    ss_ckpt_idx = ss_node_rpc.strata_getLatestCheckpointIndex()
    assert ss_ckpt_idx == cs_ckpt_idx

    ckpt_sync_latest_slot = get_latest_slot(cs_node_rpc)
    assert ckpt_sync_latest_slot > 0  # ensure checkpoint sync client is not stuck at genesis
    logging.info(f"chain tip slot for checkpoint sync client: {ckpt_sync_latest_slot}")

    cs_chs = cs_node_rpc.strata_getChainstateRaw(ckpt_sync_latest_slot)
    ss_chs = ss_node_rpc.strata_getChainstateRaw(ckpt_sync_latest_slot)

    logging.info(f"comparing chainstates for latest slot: {ckpt_sync_latest_slot}")
    assert cs_chs == ss_chs

