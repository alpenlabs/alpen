"""Bail tag identifiers for debug crash injection.

The Rust constants in ``crates/common/src/bail_tags.rs`` are the single source
of truth for bail point names. The functional test framework discovers them at
runtime via the ``debug_listBailTags`` RPC, so there is no Python-side mirror to
keep in sync.

Typical usage:

    from common.bail_tags import require_known_bail_tag

    tag = require_known_bail_tag(rpc, "fcm_new_block")
    rpc.debug_bail(tag)

If a tag is misspelled or not registered in the running ``strata`` binary, the
helper raises ``AssertionError`` with the list of known tags so the failure
points at the source-of-truth file.
"""

from common.rpc import JsonRpcClient


def list_known_bail_tags(rpc: JsonRpcClient) -> list[str]:
    """Return the bail tags registered by the running strata binary.

    Wraps the ``debug_listBailTags`` RPC, which is only registered when strata
    is built with the ``debug-utils`` feature.
    """
    return list(rpc.debug_listBailTags())


def require_known_bail_tag(rpc: JsonRpcClient, tag: str) -> str:
    """Validate ``tag`` against the live bail tag list and return it.

    Fails fast on typos or stale references rather than letting the test hang
    waiting for a bail point that will never trip.
    """
    known = list_known_bail_tags(rpc)
    if tag not in known:
        raise AssertionError(
            f"bail tag {tag!r} is not registered in strata. "
            f"check crates/common/src/bail_tags.rs::KNOWN_BAIL_TAGS. "
            f"known tags: {sorted(known)}"
        )
    return tag
