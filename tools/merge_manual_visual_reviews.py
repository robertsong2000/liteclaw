#!/usr/bin/env python3
"""Validate and merge authored visual review batches; never generate observations."""
import argparse
import hashlib
import json
from pathlib import Path


def read_rows(path):
    return [json.loads(line) for line in path.read_text().splitlines() if line.strip()]


def merge(root, batches):
    catalog = json.loads((root / 'catalog.json').read_text())
    by_id = {row['id']: row for row in catalog}
    pages = {(row['source_file'], row['pdf_page']): row for row in catalog
             if row['kind'] == 'source_page'}
    path = root / 'visual-reviews.jsonl'
    merged = {row['id']: row for row in read_rows(path)} if path.exists() else {}
    seen = set()
    for batch in batches:
        for row in read_rows(batch):
            image_id = row['id']
            if image_id in seen or image_id in merged:
                raise ValueError('Duplicate review: ' + image_id)
            seen.add(image_id)
            item = by_id[image_id]
            if item['kind'] != 'illustration':
                raise ValueError('Only illustration crops belong in a batch')
            if row['status'] not in ('model-verified', 'needs-review'):
                raise ValueError('Invalid review status')
            for field in ('caption', 'description', 'association', 'method', 'reviewer'):
                if not isinstance(row.get(field), str) or not row[field].strip():
                    raise ValueError('Missing review field: ' + field)
            if not isinstance(row.get('issues'), list):
                raise ValueError('issues must be a list')
            if row.get('human_review') != 'pending':
                raise ValueError('Model review cannot approve human review')
            page = pages[(item['source_file'], item['pdf_page'])]
            required = {item['file'], page['file']}
            if not required.issubset(set(row.get('evidence_files', []))):
                raise ValueError('Missing crop/page observation: ' + image_id)
            hashes = {}
            for name in row['evidence_files']:
                evidence = (root / name).resolve()
                if not evidence.is_relative_to(root.resolve()) or evidence.suffix != '.png':
                    raise ValueError('Invalid evidence path')
                hashes[name] = hashlib.sha256(evidence.read_bytes()).hexdigest()
            merged[image_id] = {**row, 'source_sha256': item['source_sha256'],
                                'image_sha256': hashes[item['file']], 'evidence_sha256': hashes}
    rows = sorted(merged.values(), key=lambda row: (by_id[row['id']]['pdf_page'], row['id']))
    temporary = path.with_suffix('.jsonl.tmp')
    temporary.write_text(''.join(json.dumps(row, ensure_ascii=False) + '\n' for row in rows))
    temporary.replace(path)
    illustrations = [row for row in catalog if row['kind'] == 'illustration']
    pending = [row['id'] for row in illustrations if row['id'] not in merged]
    report = {'total_illustrations': len(illustrations), 'visually_reviewed': len(rows),
              'model_verified': sum(row['status'] == 'model-verified' for row in rows),
              'needs_human_review': sum(row['status'] == 'needs-review' for row in rows),
              'not_yet_reviewed': pending, 'human_approved': 0}
    (root / 'visual-review-summary.json').write_text(json.dumps(report, ensure_ascii=False, indent=2))
    print(json.dumps({key: len(value) if isinstance(value, list) else value
                      for key, value in report.items()}, ensure_ascii=False))
    return report


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('root', type=Path)
    parser.add_argument('batches', type=Path, nargs='*')
    args = parser.parse_args()
    merge(args.root, args.batches)
