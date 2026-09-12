#!/usr/bin/env python3
"""Render source-page references without changing the source PDF or text corpus."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
from concurrent.futures import ThreadPoolExecutor
import math
import pdfplumber
import sys


def figure_context(page, box):
    """Keep the figure's column, not unrelated text in neighboring columns."""
    if box is None:
        return (page.extract_text() or '')[:2400]
    x0, top, x1, bottom = box
    region = (max(0, x0-8), max(35, top-150), min(page.width, x1+8),
              min(page.height-20, bottom+170))
    return (page.crop(region).extract_text() or '')[:1800]


def symbol_regions(page):
    """R5 three-column warning rows: preserve each symbol and its adjacent label."""
    symbols = sorted([i for i in page.images if i['width'] >= 12 and i['height'] >= 12
                      and not (i['width'] >= 60 and i['height'] >= 45)],
                     key=lambda i:(i['x0'], i['top']))
    regions = []
    for item in symbols:
        column = min(2, int(item['x0'] / (page.width/3)))
        next_top = min([i['top'] for i in symbols if i['top'] > item['bottom']
                        and int(i['x0']/(page.width/3)) == column] or [page.height-20])
        right = min(page.width, max(item['x1']+3, (column+1)*page.width/3-7))
        regions.append((max(0,item['x0']-3), max(0,item['top']-3), right,
                        min(page.height, item['bottom']+18, next_top-2)))
    return regions


def build(source_root, output):
    source_root = source_root.resolve()
    output.mkdir(parents=True, exist_ok=True)
    manifest = source_root / 'data/manifests/image-reference-manifest.jsonl'
    references = []
    for line in manifest.read_text().splitlines():
        if not line.strip():
            continue
        references.append(json.loads(line))
    chunks = [json.loads(line) for path in (source_root / 'data/chunks').glob('*.jsonl')
              for line in path.read_text().splitlines() if line.strip()]
    catalog, jobs = [], []
    for source_file in sorted({item['source_file'] for item in references}):
        item = {'source_file': source_file}
        pdf = (source_root / item['source_file']).resolve()
        if not pdf.is_relative_to(source_root) or pdf.suffix.lower() != '.pdf':
            raise ValueError('Invalid source PDF')
        digest = hashlib.sha256(pdf.read_bytes()).hexdigest()
        with pdfplumber.open(pdf) as document:
            for page in document.pages:
                number = page.page_number
                related = [c for c in chunks if c.get('page_start') and c.get('page_end')
                           and c['page_start'] <= number <= c['page_end']
                           and (not c.get('source_file') or c['source_file'] == source_file)]
                explicit = [r for r in references if r['source_file'] == source_file and r['pdf_page'] == number]
                # Ignore tiny repeated glyphs/icons. Render composed PDF pixels,
                # not raw image streams, so masks and numbered overlays survive.
                boxes = sorted({(max(0, i['x0']-4), max(0, i['top']-4),
                                 min(page.width, i['x1']+4), min(page.height, i['bottom']+4))
                                for i in page.images if i['width'] >= 60 and i['height'] >= 45})
                # Small dashboard pictograms are useful with their explanations,
                # but poor standalone crops. Keep a full-page view for those and
                # meaningful vector artwork; ignore rectangular frames/tables.
                has_symbols = any(i['width'] >= 12 and i['height'] >= 12 for i in page.images)
                has_vectors = any(
                    c['width'] >= 12 and c['height'] >= 12
                    and c['x0'] >= 0 and c['top'] >= 0
                    and (len({round(p[0], 1) for p in c['pts']}) > 2
                         or len({round(p[1], 1) for p in c['pts']}) > 2)
                    for c in page.curves
                )
                if not explicit and (not related or not (boxes or has_symbols or has_vectors)):
                    page.close()
                    continue
                # JSONL chunks can straddle section boundaries; the PDF heading
                # describes the actual illustrated page more accurately.
                page_lines = (page.extract_text() or '').splitlines()
                page_title = page_lines[0].strip() if page_lines else 'Manual illustration'
                description = explicit[0]['alt_text'] if explicit else page_title
                base = {**(explicit[0] if explicit else {}), 'source_file': source_file,
                        'pdf_page': number, 'source_sha256': digest,
                        'alt_text': description, 'page_title': page_title,
                        'chunk_ids': list(dict.fromkeys(
                            [cid for ref in explicit for cid in ref.get('chunk_ids', [])]
                            + [c['chunk_id'] for c in related])),
                        'review_status': explicit[0].get('review_status') if explicit else 'page-linked-unreviewed'}
                regions = [('page', None)] + [(f'figure-{i+1}', b) for i,b in enumerate(boxes)]
                regions += [(f'symbol-{i+1}', b) for i,b in enumerate(symbol_regions(page))]
                for suffix, box in regions:
                    image_id = f'{digest[:16]}-{suffix}-{number}'
                    target = output / image_id
                    catalog.append({**base, 'id': image_id, 'file': image_id+'.png',
                                    'context': (page.crop(box).extract_text() or '') if suffix.startswith('symbol-') else figure_context(page, box),
                                    'kind': 'source_page' if box is None else 'illustration', 'bbox': box})
                    if not target.with_suffix('.png').exists():
                        command = ['pdftoppm', '-f', str(number), '-l', str(number), '-r', '160', '-singlefile', '-png']
                        if box:
                            x0,y0,x1,y1 = box
                            scale = 160/72
                            command += ['-x', str(math.floor(x0*scale)), '-y', str(math.floor(y0*scale)),
                                        '-W', str(math.ceil((x1-x0)*scale)), '-H', str(math.ceil((y1-y0)*scale))]
                        jobs.append(command + [str(pdf), str(target)])
                page.close()
    with ThreadPoolExecutor(max_workers=4) as pool:
        for i, _ in enumerate(pool.map(lambda cmd: subprocess.run(cmd, check=True, stdout=subprocess.DEVNULL), jobs), 1):
            if i % 50 == 0:
                print(f'Rendered {i}/{len(jobs)} assets', flush=True)
    temporary = output / 'catalog.json.tmp'
    temporary.write_text(json.dumps(catalog, ensure_ascii=False, indent=2))
    temporary.replace(output / 'catalog.json')
    from build_manual_image_review import build as build_review
    build_review(output / 'catalog.json', output)
    print(f'Cataloged {len(catalog)} page/illustration assets into {output}')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('source_root', type=Path)
    parser.add_argument('output', type=Path)
    args = parser.parse_args()
    build(args.source_root, args.output)
