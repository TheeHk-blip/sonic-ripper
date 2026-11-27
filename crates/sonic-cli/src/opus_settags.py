#!/usr/bin/env python3
"""Set arbitrary tags in an Ogg Opus file without touching audio or cover art."""
import sys
from collections import Counter
from pathlib import Path
from opus_retag import parse, packets_for, build_page

def read_comments(data: bytes):
    """Return the existing OpusTags comments without requiring a clean layout.

    This is used by the ffmpeg fallback, which rewrites the file from an
    ffmetadata file so cover art and every existing tag are preserved.
    """
    pages = parse(data)
    serial = pages[0]['serial']
    for pkt, _owners in packets_for([p for p in pages if p['serial'] == serial]):
        if not pkt.startswith(b'OpusTags'):
            continue
        pos = 8
        vlen = int.from_bytes(pkt[pos:pos + 4], 'little'); pos += 4 + vlen
        count = int.from_bytes(pkt[pos:pos + 4], 'little'); pos += 4
        out = []
        for _ in range(count):
            length = int.from_bytes(pkt[pos:pos + 4], 'little'); pos += 4
            raw = pkt[pos:pos + length]; pos += length
            out.append(raw.decode('utf-8', 'replace'))
        return out
    raise RuntimeError('no OpusTags')


def set_tags(data: bytes, updates: dict) -> bytes:
    pages = parse(data)
    serial = pages[0]['serial']
    idxs = [i for i, p in enumerate(pages) if p['serial'] == serial]
    found = None
    for pkt, owners in packets_for([p for p in pages if p['serial'] == serial]):
        if pkt.startswith(b'OpusTags'):
            found = (pkt, owners)
            break
    if not found:
        raise RuntimeError('no OpusTags')
    pkt, owners = found
    tag_pages = list(dict.fromkeys(idxs[i] for i in owners))

    pos = 8
    vlen = int.from_bytes(pkt[pos:pos+4], 'little'); pos += 4
    vendor = pkt[pos:pos+vlen]; pos += vlen
    count = int.from_bytes(pkt[pos:pos+4], 'little'); pos += 4
    comments = []
    for _ in range(count):
        l = int.from_bytes(pkt[pos:pos+4], 'little'); pos += 4
        comments.append(pkt[pos:pos+l]); pos += l
    if pos != len(pkt):
        raise RuntimeError(f'unparsed bytes in OpusTags ({len(pkt) - pos})')

    # A tag page must contain only the OpusTags packet. If a page also holds
    # audio segments, replacing it here would drop that audio; let the caller
    # use ffmpeg for files with this layout.
    per_page = Counter(owners)
    for j, page_idx in enumerate(tag_pages):
        segs = per_page.get(j, 0)
        if segs < len(pages[page_idx]['table']):
            raise RuntimeError(f'OpusTags shares page {page_idx} with other packets')

    for key, value in updates.items():
        k = key.upper().encode()
        new = key.lower().encode() + b'=' + value.encode()
        hit = False
        for i, c in enumerate(comments):
            if c.split(b'=', 1)[0].upper() == k:
                comments[i] = new
                hit = True
        if not hit:
            comments.append(new)

    new_pkt = (b'OpusTags'
               + len(vendor).to_bytes(4, 'little') + vendor
               + len(comments).to_bytes(4, 'little')
               + b''.join(len(c).to_bytes(4, 'little') + c for c in comments))
    segs, rest = [], new_pkt
    while len(rest) >= 255:
        segs.append(255); rest = rest[255:]
    if rest or not segs:
        segs.append(len(rest))
    groups = [segs[i:i+255] for i in range(0, len(segs), 255)]
    if len(groups) < len(tag_pages):
        raise RuntimeError(f'tag packet shrink unsupported ({len(tag_pages)} -> {len(groups)})')

    repl, consumed = {}, 0
    for j, group in enumerate(groups[:len(tag_pages)]):
        payload = new_pkt[consumed:consumed+sum(group)]; consumed += sum(group)
        src = pages[tag_pages[j]]
        repl[tag_pages[j]] = build_page(src['ht'], src['granule'], src['serial'], src['seq'], group, payload)

    extra = []
    last_tag = pages[tag_pages[-1]]
    if len(groups) > len(tag_pages):
        for j, group in enumerate(groups[len(tag_pages):]):
            payload = new_pkt[consumed:consumed+sum(group)]; consumed += sum(group)
            extra.append(build_page(0, last_tag['granule'], serial,
                                    last_tag['seq'] + 1 + j, group, payload))
    assert consumed == len(new_pkt)

    delta = len(extra)
    out = bytearray()
    for i, p in enumerate(pages):
        if i > tag_pages[-1] and delta and p['serial'] == serial:
            out += build_page(p['ht'], p['granule'], p['serial'], p['seq'] + delta,
                              p['table'], p['payload'])
        else:
            out += repl[i] if i in repl else p['raw']
        if i == tag_pages[-1]:
            out += b''.join(extra)
    return bytes(out)

if __name__ == '__main__':
    src, dst = Path(sys.argv[1]), Path(sys.argv[2])
    updates = dict(item.split('=', 1) for item in sys.argv[3:])
    dst.write_bytes(set_tags(src.read_bytes(), updates))
    print('patched', dst)
