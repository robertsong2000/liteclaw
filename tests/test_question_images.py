"""Question-specific selection: real PDF context, validated model choices."""
import importlib.util
from pathlib import Path
import sys
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / 'skills/manual-rag/lib'))


class QuestionImagesTests(unittest.TestCase):
    def test_warning_symbol_crop_excludes_neighboring_warning_rows(self):
        import pdfplumber
        spec = importlib.util.spec_from_file_location('builder', ROOT / 'tools/build_manual_images.py')
        builder = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(builder)
        with pdfplumber.open(ROOT.parent / 'renault_r5_manual/data/raw/official/renault-5-e-tech-2025-owner-manual-en.pdf') as pdf:
            page = pdf.pages[135]
            texts = [page.crop(box).extract_text() or '' for box in builder.symbol_regions(page)]
        label = next(text for text in texts if 'Low windscreen washer' in text)
        self.assertNotIn('airbag', label)
        self.assertNotIn('Automatic wiping', label)

    def test_same_page_figures_keep_distinct_operating_context(self):
        import pdfplumber
        spec = importlib.util.spec_from_file_location('builder', ROOT / 'tools/build_manual_images.py')
        builder = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(builder)
        with pdfplumber.open(ROOT.parent / 'renault_r5_manual/data/raw/official/renault-5-e-tech-2025-owner-manual-en.pdf') as pdf:
            page = pdf.pages[144]
            figures = sorted([i for i in page.images if i['width'] >= 60 and i['height'] >= 45], key=lambda i:i['x0'])
            contexts = [builder.figure_context(page, (i['x0'],i['top'],i['x1'],i['bottom'])) for i in figures]
        self.assertIn('movement A', contexts[0])
        self.assertNotIn('camera', contexts[0])
        self.assertIn('camera', contexts[1])

    def test_invalid_duplicate_or_excess_choices_never_become_featured(self):
        from image_select import validate_choices
        candidates = [{'id':str(i), 'context':'manual context', 'kind':'illustration'} for i in range(5)]
        selected = validate_choices({'images':[{'id':'unknown'}, {'id':'1'}, {'id':'1'}, {'id':'2'}, {'id':'3'}, {'id':'4'}]}, candidates)
        self.assertEqual([i['id'] for i in selected], ['1','2','3'])
        self.assertEqual([i['role'] for i in selected], ['primary','supporting','supporting'])
        self.assertEqual(validate_choices({'images':[]}, candidates), [])

    def test_shortlist_does_not_drop_specific_symbol_after_many_generic_figures(self):
        from image_select import shortlist
        candidates = [{'id':str(i),'kind':'illustration','context':'Warning light brake system'} for i in range(70)]
        candidates.append({'id':'washer','kind':'illustration','context':'Low windscreen washer level warning light'})
        self.assertEqual(shortlist('Low windscreen washer level warning light',candidates)[0]['id'],'washer')

    def test_selector_failure_returns_no_unverified_main_image(self):
        from image_select import select_images
        candidates = [{'id':'stalk','kind':'illustration','context':'Push the stalk for main beam'}]
        with patch.dict('os.environ', {'IMAGE_SELECT_BASE_URL':'https://selector.invalid/v1', 'IMAGE_SELECT_MODEL':'fixture'}), \
             patch('urllib.request.urlopen',side_effect=TimeoutError):
            self.assertEqual(select_images('How to switch on main beam?',candidates), [])

    def test_offline_selection_never_calls_a_network_endpoint(self):
        from image_select import select_images_offline
        candidates = [
            {'id':'wrong','kind':'illustration','context':'Automatic main beam camera settings'},
            {'id':'right','kind':'illustration','context':'Push the stalk 1 movement A to turn on main beam'},
        ]
        with patch('urllib.request.urlopen',side_effect=AssertionError('offline mode must not call network')):
            selected = select_images_offline('How do I manually turn on main beam?', candidates)
        self.assertEqual([item['id'] for item in selected], ['right'])

    def test_review_queue_marks_ambiguous_or_missing_context(self):
        from image_review import review_catalog
        catalog = [
            {'id':'clear','kind':'illustration','context':'Push stalk A for main beam','pdf_page':145,'source_file':'m.pdf'},
            {'id':'unclear','kind':'illustration','context':'LIGHTING AND SIGNALS','pdf_page':145,'source_file':'m.pdf'},
            {'id':'page','kind':'source_page','context':'LIGHTING AND SIGNALS','pdf_page':145,'source_file':'m.pdf'},
        ]
        annotations, queue = review_catalog(catalog)
        self.assertEqual(annotations[0]['annotation_method'], 'offline-pdf-layout')
        self.assertIn('weak-context', queue[0]['reasons'])
        self.assertIn('same-page-multiple-figures', queue[0]['reasons'])

    def test_old_generic_catalog_does_not_invent_a_question_specific_image(self):
        from image_select import select_images
        with patch('urllib.request.urlopen',side_effect=AssertionError('network must not be needed')) as request:
            self.assertEqual(select_images('How to switch on main beam?',[
                {'id':'old','kind':'illustration','alt_text':'LIGHTING AND SIGNALS'}]), [])
            request.assert_not_called()


if __name__ == '__main__':
    unittest.main()
