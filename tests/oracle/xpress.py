#!/usr/bin/env python3
"""Writes the plain LZ77 ("Xpress") test vectors of src/xpress.rs, none of
the test databases holding an Xpress-compressed value.

Each input is compressed here following [MS-XCA] 2.3 (greedy matching,
lengths in the token, a shared half byte, a byte and 16 bits), and the
result is checked with libfwnt's decoder before it is printed, so the
vectors don't rest on this crate's reading of the format. (libfwnt doesn't
decode 32-bit lengths, which ESE values, at most 65,535 bytes, can't hold.)

    sudo apt-get install python3-libfwnt
    python3 tests/oracle/xpress.py
"""

import struct

import pyfwnt

WINDOW = 8192
MIN_MATCH = 3
CANDIDATES = 64

# The inputs, as the Rust tests rebuild them.
VECTORS = {
    "text": b"In ESE, compressed values: " + b"abc" * 20 + b"xyz",
    "shared_half_byte": b"0123456789" * 3 + b"ABCDEFGHIJ" + b"0123456789" * 4 + b"ABCDEFGHIJ" * 3,
    "long_run": b"\x00" * 5000,
}


def longest_match(data, position, chains):
    """The (offset, length) of the longest earlier match at position."""
    best = (0, 0)
    for start in reversed(chains.get(data[position : position + MIN_MATCH], [])[-CANDIDATES:]):
        if position - start > WINDOW:
            break
        length = 0
        while position + length < len(data) and data[start + length] == data[position + length]:
            length += 1
        if length > best[1]:
            best = (position - start, length)
    return best


def compress(data):
    out = bytearray(4)
    flags, flag_count, flag_position, half_byte = 0, 0, 0, None
    chains = {}
    position = 0

    def remember(index):
        chains.setdefault(data[index : index + MIN_MATCH], []).append(index)

    while position < len(data):
        offset, length = longest_match(data, position, chains)
        if length < MIN_MATCH:
            out.append(data[position])
            remember(position)
            position += 1
            flags <<= 1
        else:
            for index in range(position, position + length):
                remember(index)
            position += length
            length -= MIN_MATCH
            token = (offset - 1) << 3
            if length < 7:
                out += struct.pack("<H", token | length)
            else:
                out += struct.pack("<H", token | 7)
                length -= 7
                nibble = min(length, 15)
                if half_byte is None:
                    half_byte = len(out)
                    out.append(nibble)
                else:
                    out[half_byte] |= nibble << 4
                    half_byte = None
                if length >= 15:
                    length -= 15
                    if length < 255:
                        out.append(length)
                    else:
                        out.append(255)
                        length += 15 + 7
                        if length < 1 << 16:
                            out += struct.pack("<H", length)
                        else:
                            out += struct.pack("<HI", 0, length)
            flags = (flags << 1) | 1
        flag_count += 1
        if flag_count == 32:
            out[flag_position : flag_position + 4] = struct.pack("<I", flags & 0xFFFFFFFF)
            flags, flag_count, flag_position = 0, 0, len(out)
            out += bytes(4)
    unused = 32 - flag_count
    flags = (flags << unused) | ((1 << unused) - 1)
    out[flag_position : flag_position + 4] = struct.pack("<I", flags & 0xFFFFFFFF)
    return bytes(out)


def main():
    for name, plain in VECTORS.items():
        compressed = compress(plain)
        assert pyfwnt.lzxpress_decompress(compressed, len(plain)) == plain, name
        print("%s (%d bytes): %s" % (name, len(plain), compressed.hex()))


if __name__ == "__main__":
    main()
