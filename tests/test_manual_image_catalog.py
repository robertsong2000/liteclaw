"""Opt-in checks against the generated R5 catalog (no network or PDF edits)."""
import json
import os
from pathlib import Path
import unittest


class ManualCatalogTests(unittest.TestCase):
    def test_warning_light_icons_keep_a_page_preview(self):
        root = Path(os.environ.get('MANUAL_IMAGE_DIR', str(Path.home()/'.liteclaw/manual-images')))
        catalog = json.loads((root/'catalog.json').read_text())
        pages = [i for i in catalog if i['pdf_page'] == 136]
        self.assertTrue(pages, 'small warning symbols must not be discarded')
        self.assertTrue(any(i['kind'] == 'source_page' for i in pages))

    def test_cross_section_chunk_does_not_mislabel_fuse_page(self):
        root = Path(os.environ.get('MANUAL_IMAGE_DIR', str(Path.home()/'.liteclaw/manual-images')))
        catalog = json.loads((root/'catalog.json').read_text())
        item = next(i for i in catalog if i['pdf_page'] == 348 and i['kind'] == 'illustration')
        self.assertIn('FUSE BOX A', item['alt_text'].upper())
        self.assertNotIn('WIPER', item['alt_text'].upper())


if __name__ == '__main__':
    unittest.main()
