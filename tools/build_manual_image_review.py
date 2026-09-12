#!/usr/bin/env python3
"""Generate offline annotations and a human-review queue from catalog.json."""
import argparse
import json
from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent.parent / 'skills/manual-rag/lib'))
from image_review import review_catalog, apply_visual_reviews


def build(catalog_path, output_dir):
    catalog = json.loads(catalog_path.read_text())
    catalog = apply_visual_reviews(catalog, output_dir)
    temporary_catalog = catalog_path.with_suffix('.json.tmp')
    temporary_catalog.write_text(json.dumps(catalog, ensure_ascii=False, indent=2))
    temporary_catalog.replace(catalog_path)
    annotations, queue = review_catalog(catalog)
    output_dir.mkdir(parents=True, exist_ok=True)
    for name, rows in (('annotations.jsonl', annotations), ('review-queue.jsonl', queue)):
        temporary = output_dir / (name + '.tmp')
        temporary.write_text(''.join(json.dumps(row, ensure_ascii=False) + '\n' for row in rows))
        temporary.replace(output_dir / name)
    print(f'Offline annotations: {len(annotations)}; human-review queue: {len(queue)}')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('catalog', type=Path)
    parser.add_argument('output', type=Path)
    args = parser.parse_args()
    build(args.catalog, args.output)
