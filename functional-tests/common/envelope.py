"""SPS-51 envelope decoding.

Protocol payloads are revealed inside a taproot script-path spend rather than
in an OP_RETURN, so reading one back off L1 means pulling the pushes out of
the reveal tapscript.
"""


def extract_envelope_payload(script: bytes) -> bytes | None:
    """Extract the payload from a reveal tapscript.

    Tapscript shape:
        <pubkey(32)> OP_CHECKSIG OP_FALSE OP_IF <payload_bytes> OP_ENDIF

    The pubkey + OP_CHECKSIG prefix is ignored; we read every push between
    OP_FALSE OP_IF and OP_ENDIF and concatenate them.
    """
    OP_FALSE, OP_IF, OP_ENDIF = 0x00, 0x63, 0x68
    OP_PUSHDATA1, OP_PUSHDATA2 = 0x4C, 0x4D

    i = 0
    while i < len(script) - 1:
        if script[i] == OP_FALSE and script[i + 1] == OP_IF:
            i += 2
            break
        i += 1
    else:
        return None

    chunks: list[bytes] = []
    while i < len(script) and script[i] != OP_ENDIF:
        opcode = script[i]
        if 0x01 <= opcode <= 0x4B:
            i += 1
            if i + opcode > len(script):
                return None
            chunks.append(script[i : i + opcode])
            i += opcode
        elif opcode == OP_PUSHDATA1:
            i += 1
            if i >= len(script):
                return None
            length = script[i]
            i += 1
            if i + length > len(script):
                return None
            chunks.append(script[i : i + length])
            i += length
        elif opcode == OP_PUSHDATA2:
            i += 1
            if i + 2 > len(script):
                return None
            length = int.from_bytes(script[i : i + 2], "little")
            i += 2
            if i + length > len(script):
                return None
            chunks.append(script[i : i + length])
            i += length
        else:
            i += 1

    return b"".join(chunks) if chunks else None
