#!/usr/bin/env python3
"""Edit date tag in Ogg Opus files without touching audio or cover art."""
import sys
from pathlib import Path

CRC_POLY = 0x04c11db7

def ogg_crc(data: bytes) -> int:
    crc = 0
    for byte in data:
        crc ^= byte << 24
        for _ in range(8):
            if crc & 0x80000000:
                crc = ((crc << 1) ^ CRC_POLY) & 0xffffffff
            else:
                crc = (crc << 1) & 0xffffffff
    return crc


def build_page(ht, granule, serial, seq, table, payload):
    head = (b'OggS' + bytes([0, ht])
            + int(granule).to_bytes(8, 'little', signed=True)
            + int(serial).to_bytes(4, 'little')
            + int(seq).to_bytes(4, 'little')
            + b'\x00\x00\x00\x00'
            + bytes([len(table)]) + bytes(table))
    page = head + payload
    crc = ogg_crc(page)
    return page[:22] + crc.to_bytes(4, 'little') + page[26:]


def parse(data):
    pages, off = [], 0
    while off < len(data):
        assert data[off:off + 4] == b'OggS', f'bad page at {off}'
        n = data[off + 26]
        table = data[off + 27:off + 27 + n]
        plen = sum(table)
        payload = data[off + 27 + n:off + 27 + n + plen]
        pages.append({
            'ht': data[off + 5],
            'granule': int.from_bytes(data[off + 6:off + 14], 'little', signed=True),
            'serial': int.from_bytes(data[off + 14:off + 18], 'little'),
            'seq': int.from_bytes(data[off + 18:off + 22], 'little'),
            'stored_crc': int.from_bytes(data[off + 22:off + 26], 'little'),
            'table': table,
            'payload': payload,
            'raw': data[off:off + 27 + n + plen],
        })
        off += 27 + n + plen
    return pages


def packets_for(pages):
    """Return list of (packet_bytes, [page indices]) for one logical stream."""
    out, buf, owners = [], b'', []
    for i, page in enumerate(pages):
        pos = 0
        for lacing in page['table']:
            buf += page['payload'][pos:pos + lacing]
            pos += lacing
            owners.append(i)
            if lacing < 255:
                out.append((buf, owners))
                buf, owners = b'', []
    return out


def patch(data: bytes, year: str) -> bytes:
    pages = parse(data)
    # sanity check CRC implementation against the first page
    p0 = pages[0]
    check = build_page(p0['ht'], p0['granule'], p0['serial'], p0['seq'], p0['table'], p0['payload'])
    assert check == p0['raw'], 'CRC implementation mismatch'

    found = None
    for page in pages:
        # find the stream that starts with OpusHead
        if page['payload'].startswith(b'OpusHead'):
            serial = page['serial']
            break
    else:
        raise RuntimeError('no OpusHead found')

    for pkt, owners in packets_for([p for p in pages if p['serial'] == serial]):
        if pkt.startswith(b'OpusTags'):
            found = (pkt, owners)
            break
    if not found:
        raise RuntimeError('no OpusTags found')
    pkt, owners = found
    idxs = [i for i, p in enumerate(pages) if p['serial'] == serial]
    tag_pages = list(dict.fromkeys(idxs[i] for i in owners))  # one entry per segment -> dedupe
    assert len(tag_pages) > 0, 'unexpected page mapping'

    pos = 8
    vlen = int.from_bytes(pkt[pos:pos + 4], 'little'); pos += 4
    vendor = pkt[pos:pos + vlen]; pos += vlen
    count = int.from_bytes(pkt[pos:pos + 4], 'little'); pos += 4
    comments = []
    for _ in range(count):
        l = int.from_bytes(pkt[pos:pos + 4], 'little'); pos += 4
        comments.append(pkt[pos:pos + l]); pos += l
    assert pos == len(pkt), 'unparsed bytes in OpusTags'

    new_date = ('date=' + year).encode()
    replaced = False
    for i, c in enumerate(comments):
        if c.lower().startswith(b'date='):
            comments[i] = new_date
            replaced = True
    if not replaced:
        comments.append(new_date)

    new_pkt = (b'OpusTags'
               + len(vendor).to_bytes(4, 'little') + vendor
               + len(comments).to_bytes(4, 'little')
               + b''.join(len(c).to_bytes(4, 'little') + c for c in comments))

    segs, rest = [], new_pkt
    while len(rest) >= 255:
        segs.append(255); rest = rest[255:]
    if rest or not segs:
        segs.append(len(rest))
    groups = [segs[i:i + 255] for i in range(0, len(segs), 255)]

    assert len(groups) == len(tag_pages), (
        f'page count change not supported here ({len(tag_pages)} -> {len(groups)})')

    replacements = {}
    consumed = 0
    for j, group in enumerate(groups):
        payload = new_pkt[consumed:consumed + sum(group)]; consumed += sum(group)
        src = pages[tag_pages[j]]
        replacements[tag_pages[j]] = build_page(src['ht'], src['granule'], src['serial'],
                                                 src['seq'], group, payload)
    assert consumed == len(new_pkt)

    out = bytearray()
    for i, page in enumerate(pages):
        out += replacements[i] if i in replacements else page['raw']
    return bytes(out)


if __name__ == '__main__':
    src, dst, year = Path(sys.argv[1]), Path(sys.argv[2]), sys.argv[3]
    dst.write_bytes(patch(src.read_bytes(), year))
    print(f'patched {src} -> {dst} ({year})')
