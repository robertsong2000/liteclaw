"""Test the offline visual annotation/review generator with no network."""
import importlib.util
from pathlib import Path
import unittest
import tempfile
import json
import hashlib

ROOT = Path(__file__).resolve().parents[1]


class ImageReviewTests(unittest.TestCase):
    def test_visual_review_requires_exact_image_and_keeps_human_review_pending(self):
        import sys
        sys.path.insert(0, str(ROOT / 'skills/manual-rag/lib'))
        from image_review import apply_visual_reviews, review_catalog
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'a.png').write_bytes(b'original image fixture')
            row = {'id':'a','file':'a.png','source_sha256':'pdf-hash',
                   'kind':'illustration','alt_text':'old','context':'short','pdf_page':1}
            review = {'id':'a','status':'model-verified','source_sha256':'pdf-hash',
                      'image_sha256':hashlib.sha256((root/'a.png').read_bytes()).hexdigest(),
                      'caption':'verified caption','description':'visible content',
                      'association':'page evidence','method':'conversation-vision-page-and-crop'}
            (root/'visual-reviews.jsonl').write_text(json.dumps(review)+'\n')
            updated = apply_visual_reviews([row], root)
            self.assertEqual(updated[0]['alt_text'], 'verified caption')
            self.assertEqual(updated[0]['original_context'], 'short')
            self.assertEqual(review_catalog(updated)[0][0]['visual_review_status'],
                             'model-verified-not-human-reviewed')
            (root/'a.png').write_bytes(b'changed image')
            stale = apply_visual_reviews(updated, root)[0]
            self.assertEqual(stale['alt_text'], 'old')
            self.assertNotIn('visual_review_status', stale)

    def test_unresolved_crop_is_queued_and_excluded_from_selection(self):
        import sys
        sys.path.insert(0, str(ROOT / 'skills/manual-rag/lib'))
        from image_review import apply_visual_reviews, review_catalog
        from image_select import shortlist, select_images
        from unittest.mock import patch
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root/'a.png').write_bytes(b'crop')
            item = {'id':'a','file':'a.png','source_sha256':'pdf','kind':'illustration',
                    'alt_text':'charging port','context':'charging port','pdf_page':1}
            row = {'id':'a','status':'needs-review','source_sha256':'pdf',
                   'image_sha256':hashlib.sha256(b'crop').hexdigest(),
                   'method':'conversation-vision-page-and-crop','issues':['truncated-symbol']}
            (root/'visual-reviews.jsonl').write_text(json.dumps(row))
            updated = apply_visual_reviews([item], root)
            self.assertEqual(updated[0]['visual_review_status'], 'needs-human-review')
            self.assertIn('truncated-symbol', review_catalog(updated)[1][0]['reasons'])
            self.assertEqual(shortlist('charging', updated), [])
            with patch('urllib.request.urlopen', side_effect=AssertionError('No call expected')):
                self.assertEqual(select_images('charging', updated), [])
            (root/'visual-reviews.jsonl').unlink()
            self.assertNotIn('visual_review_status', apply_visual_reviews(updated, root)[0])

    def test_page_evidence_change_invalidates_review(self):
        import sys
        sys.path.insert(0, str(ROOT / 'skills/manual-rag/lib'))
        from image_review import apply_visual_reviews
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root/'a.png').write_bytes(b'crop')
            (root/'page.png').write_bytes(b'page')
            row = {'id':'a','status':'model-verified','source_sha256':'pdf',
                   'image_sha256':hashlib.sha256(b'crop').hexdigest(),
                   'evidence_sha256':{'page.png':hashlib.sha256(b'old-page').hexdigest()},
                   'method':'conversation-vision-page-and-crop', 'caption':'bad',
                   'description':'bad', 'association':'bad'}
            (root/'visual-reviews.jsonl').write_text(json.dumps(row))
            item = {'id':'a','file':'a.png','source_sha256':'pdf','alt_text':'original'}
            self.assertEqual(apply_visual_reviews([item], root)[0]['alt_text'], 'original')

    def test_catalog_outputs_annotation_and_review_files(self):
        spec = importlib.util.spec_from_file_location('review', ROOT / 'tools/build_manual_image_review.py')
        review = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(review)
        catalog = [{'id':'a','kind':'illustration','context':'Push stalk A','pdf_page':1,'source_file':'m.pdf'}]
        annotations, queue = review.review_catalog(catalog)
        self.assertEqual(annotations[0]['annotation_method'], 'offline-pdf-layout')
        self.assertIsInstance(queue, list)


if __name__ == '__main__':
    unittest.main()
