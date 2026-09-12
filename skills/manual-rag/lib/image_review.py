"""Offline image annotation and human-review queue generation."""
import hashlib
import json


def apply_visual_reviews(catalog, root):
    """Apply pixel-verified descriptions only to the exact reviewed assets."""
    path = root / 'visual-reviews.jsonl'
    reviews = {r['id']: r for r in (json.loads(line) for line in path.read_text().splitlines() if line.strip())} if path.exists() else {}
    result = []
    for item in catalog:
        row = dict(item)
        for key in ('visual_review_status', 'visual_review_method', 'visual_review_issues'):
            row.pop(key, None)
        if 'original_alt_text' in row:
            row['alt_text'] = row['original_alt_text']
            row['context'] = row['original_context']
        review = reviews.get(row['id'])
        if review and review.get('status') in ('model-verified', 'needs-review'):
            png = (root / row['file']).resolve()
            evidence_valid = all(
                (root / name).resolve().is_relative_to(root.resolve())
                and (root / name).is_file()
                and hashlib.sha256((root / name).read_bytes()).hexdigest() == digest
                for name, digest in review.get('evidence_sha256', {}).items())
            if (png.is_relative_to(root.resolve()) and png.is_file()
                    and review.get('source_sha256') == row.get('source_sha256')
                    and evidence_valid
                    and hashlib.sha256(png.read_bytes()).hexdigest() == review.get('image_sha256')):
                row['visual_review_method'] = review['method']
                row['visual_review_issues'] = review.get('issues', [])
                if review['status'] == 'needs-review':
                    row['visual_review_status'] = 'needs-human-review'
                    result.append(row)
                    continue
                row.setdefault('original_alt_text', row.get('alt_text', ''))
                row.setdefault('original_context', row.get('context', ''))
                row['alt_text'] = review['caption']
                row['context'] = review['description'] + '\nPage-grounded association: ' + review['association']
                row['visual_review_status'] = 'model-verified-not-human-reviewed'
        result.append(row)
    return result


def review_catalog(catalog):
    annotations = []
    queue = []
    page_counts = {}
    for item in catalog:
        key = (item.get('source_file'), item.get('pdf_page'))
        page_counts[key] = page_counts.get(key, 0) + (item.get('kind') == 'illustration')
    for item in catalog:
        context = (item.get('context') or '').strip()
        reasons = []
        if item.get('visual_review_status') == 'needs-human-review':
            reasons.append('visual-association-unresolved')
        reasons.extend(item.get('visual_review_issues', []))
        if item.get('kind') == 'illustration' and len(context) < 40:
            reasons.append('weak-context')
        if item.get('kind') == 'illustration' and page_counts.get((item.get('source_file'), item.get('pdf_page')), 0) > 1:
            reasons.append('same-page-multiple-figures')
        if item.get('kind') == 'illustration' and not item.get('bbox'):
            reasons.append('missing-region')
        confidence = 'high' if len(context) >= 120 and not reasons else ('medium' if context else 'low')
        annotation = {
            'id': item.get('id'), 'source_file': item.get('source_file'),
            'pdf_page': item.get('pdf_page'), 'kind': item.get('kind'),
            'annotation_method': item.get('visual_review_method', 'offline-pdf-layout'),
            'visual_label': item.get('alt_text') or 'unlabeled manual figure',
            'nearby_text': context, 'confidence': confidence,
            'review_status': 'needs-human-review' if reasons else 'offline-generated',
            'visual_review_status': item.get('visual_review_status', 'not-visually-reviewed'),
        }
        annotations.append(annotation)
        if reasons:
            queue.append({**annotation, 'reasons': reasons})
    queue.sort(key=lambda item: (item['confidence'] != 'low', item['pdf_page'] or 0, item['id'] or ''))
    return annotations, queue
