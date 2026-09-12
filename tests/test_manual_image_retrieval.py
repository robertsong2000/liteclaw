"""Opt-in retrieval test using the configured local corpus and embedding API."""
import json
import subprocess
import unittest
import importlib.util
import io
from contextlib import redirect_stdout
from unittest.mock import patch


class RetrievalImagesTests(unittest.TestCase):
    def test_automatic_beam_candidate_includes_previous_page_control(self):
        import sys
        from pathlib import Path
        sys.path.insert(0, str(Path('skills/manual-rag/lib').resolve()))
        import image_select
        spec = importlib.util.spec_from_file_location('rag', 'skills/manual-rag/scripts/rag.py')
        rag = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(rag)
        records, _, _ = rag.load_index()
        index = next(i for i,r in enumerate(records) if (r.get('chunk_id') or '') == 'r5e-2025-topic-lighting-0005#1')
        with patch.object(rag,'rrf',return_value=[index]), patch.object(rag,'embed_query',return_value=[]), \
             patch.object(rag,'similarities',return_value=[1.0]*len(records)), \
             patch.object(image_select,'select_images',return_value=[]) as select, redirect_stdout(io.StringIO()):
            rag.cmd_search('如何启用自动远光灯功能？',1)
        self.assertIn('6869a21848e1c3f0-figure-2-145', [i['id'] for i in select.call_args.args[1]])

    def test_allocation_table_alone_links_cross_page_layout(self):
        spec = importlib.util.spec_from_file_location('rag', 'skills/manual-rag/scripts/rag.py')
        rag = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(rag)
        records, _, _ = rag.load_index()
        table = next(i for i, r in enumerate(records)
                     if (r.get('chunk_id') or '').startswith('r5e-2025-table-fuses-allocation-0001'))
        output = io.StringIO()
        # Isolate image association from network/ranking variability while using
        # the real indexed table record and generated catalog.
        with patch.object(rag, 'rrf', return_value=[table]), \
             patch.object(rag, 'embed_query', return_value=[]), \
             patch.object(rag, 'similarities', return_value=[1.0] * len(records)), \
             redirect_stdout(output):
            rag.cmd_search('Fuse allocation', 1)
        hits = json.loads(output.getvalue())
        self.assertEqual(len(hits), 1)
        self.assertTrue(hits[0]['chunk_id'].startswith('r5e-2025-table-fuses-allocation-0001'))
        self.assertIn(350, [img['pdf_page'] for img in hits[0]['images']])

    def test_low_confidence_unmatched_query_has_no_images(self):
        result = subprocess.run(['python3', 'skills/manual-rag/scripts/rag.py',
                                 'zzqxv987654321'], check=True, capture_output=True, text=True)
        self.assertFalse(any(h['images'] for h in json.loads(result.stdout)))

    def test_fuse_query_does_not_show_adjacent_wiper_illustration(self):
        result = subprocess.run(['python3', 'skills/manual-rag/scripts/rag.py',
                                 '保险丝盒在哪里，保险丝布局图'], check=True, capture_output=True, text=True)
        hits = json.loads(result.stdout)
        ids = list(dict.fromkeys(i['id'] for h in hits for i in h['images']))[:6]
        self.assertIn('6869a21848e1c3f0-figure-1-350', ids)
        self.assertNotIn('6869a21848e1c3f0-figure-1-347', ids)
        self.assertIn('6869a21848e1c3f0-figure-1-348', ids)
        # Location and layout are complementary; either may be the primary.
        self.assertIn(ids[0], ('6869a21848e1c3f0-figure-1-348',
                              '6869a21848e1c3f0-figure-1-350'))


if __name__ == '__main__':
    unittest.main()
