import tempfile
import zlib
import unittest
from pathlib import Path
from unittest.mock import patch
from pypdf import PdfWriter
from pypdf.generic import ArrayObject, DictionaryObject, NameObject, DecodedStreamObject, EncodedStreamObject, NumberObject
from font_policy import inspect_fonts


def dictionary(**fields):
    return DictionaryObject({NameObject('/'+key): value if not isinstance(value, str) else NameObject('/'+value) for key, value in fields.items()})


def type1(name='Helvetica'):
    return dictionary(Type='Font', Subtype='Type1', BaseFont=name)


class FontPolicyTests(unittest.TestCase):
    def inspect(self, font=None, nested=False, appearance=False, cycle=False):
        writer = PdfWriter()
        page = writer.add_blank_page(width=72, height=72)
        resources = dictionary()
        if font is not None:
            entries=font if isinstance(font,list) else [font]
            resources[NameObject('/Font')]=DictionaryObject({NameObject('/F'+str(i)):writer._add_object(value) for i,value in enumerate(entries)})
        if cycle:
            ref = writer._add_object(resources)
            resources[NameObject('/Resources')] = ref
        if nested or appearance:
            form = DecodedStreamObject()
            form.set_data(b'')
            form.update(dictionary(Type='XObject', Subtype='Form', Resources=resources))
            if appearance:
                page[NameObject('/Annots')] = ArrayObject([writer._add_object(dictionary(AP=dictionary(N=writer._add_object(form))))])
                resources = dictionary()
            else:
                resources = dictionary(XObject=dictionary(Form=writer._add_object(form)))
        page[NameObject('/Resources')] = resources
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp)/'source.pdf'
            writer.write(path)
            return inspect_fonts(path)

    def test_all_standard14_explicitly_report_substitution(self):
        from font_policy import STANDARD14
        for name in STANDARD14:
            report = self.inspect(type1(name))
            self.assertTrue(report['accepted'])
            self.assertTrue(report['standard14Substitution'])
            self.assertEqual(report['fonts'][0]['state'], 'STANDARD14_SUBSTITUTION')

    def test_missing_nonstandard_and_subset_alias_are_rejected(self):
        for name in ('Arial', 'MissingJapaneseFont', 'ABCDEF+Helvetica'):
            with self.assertRaisesRegex(ValueError, 'unembedded_font'):
                self.inspect(type1(name))

    def test_nested_form_and_appearance_cannot_hide_missing_font(self):
        for options in ({'nested': True}, {'appearance': True}):
            with self.assertRaisesRegex(ValueError, 'unembedded_font'):
                self.inspect(type1('Missing'), **options)

    def test_type3_and_empty_document(self):
        self.assertEqual(self.inspect()['fonts'], [])
        proc = DecodedStreamObject()
        proc.set_data(b'0 0 d0')
        report = self.inspect(dictionary(Type='Font', Subtype='Type3', CharProcs=dictionary(A=proc)))
        self.assertEqual(report['fonts'][0]['state'], 'SELF_CONTAINED_TYPE3')
        self.assertFalse(report['standard14Substitution'])

    def test_embedded_stream_and_external_program_rejection(self):
        for external in (False, True):
            stream = DecodedStreamObject()
            stream.set_data(b'program validated later by MuPDF')
            if external:
                stream[NameObject('/F')] = NameObject('/external')
            font = dictionary(Type='Font', Subtype='TrueType', BaseFont='Owned', FontDescriptor=dictionary(FontFile2=stream))
            if external:
                with self.assertRaisesRegex(ValueError, 'invalid_program'):
                    self.inspect(font)
            else:
                self.assertEqual(self.inspect(font)['fonts'][0]['state'], 'EMBEDDED')

    def test_cid_requires_embedded_descendant(self):
        font = dictionary(Type='Font', Subtype='Type0', BaseFont='Helvetica', DescendantFonts=ArrayObject([dictionary(Subtype='CIDFontType2')]))
        with self.assertRaisesRegex(ValueError, 'unembedded_font'):
            self.inspect(font)
        font[NameObject('/DescendantFonts')] = ArrayObject([])
        with self.assertRaisesRegex(ValueError, 'invalid_descendant'):
            self.inspect(font)

    def test_cycles_are_bounded_and_not_duplicated(self):
        self.assertEqual(len(self.inspect(type1(), cycle=True)['fonts']), 1)

    def test_limits_and_program_type_mismatch_fail_closed(self):
        with patch('font_policy.MAX_NODES', 2):
            with self.assertRaisesRegex(ValueError, 'object_limit'):
                self.inspect(type1())
        with patch('font_policy.MAX_FONTS', 0):
            with self.assertRaises(ValueError):
                self.inspect(type1())
        stream = DecodedStreamObject()
        stream.set_data(b'not a TrueType program')
        with self.assertRaisesRegex(ValueError, 'program_type_mismatch'):
            self.inspect(dictionary(Type='Font', Subtype='TrueType', BaseFont='Owned', FontDescriptor=dictionary(FontFile=stream)))
        with self.assertRaisesRegex(ValueError, 'invalid_type3'):
            self.inspect(dictionary(Type='Font', Subtype='Type3'))

    def test_font_width_arrays_do_not_consume_resource_graph_budget(self):
        font = type1()
        font[NameObject('/Widths')] = ArrayObject([NumberObject(500) for _ in range(10000)])
        self.assertTrue(self.inspect(font)['accepted'])

    def test_failed_program_decompression_consumes_the_shared_budget(self):
        bomb=EncodedStreamObject();bomb._data=zlib.compress(b'A'*128);bomb[NameObject('/Filter')]=NameObject('/FlateDecode')
        plain=DecodedStreamObject();plain.set_data(b'later program')
        fonts=[dictionary(Type='Font',Subtype='TrueType',BaseFont='Owned',FontDescriptor=dictionary(FontFile2=stream)) for stream in (bomb,plain)]
        with patch('font_identity.MAX_PROGRAM',32), patch('font_policy.MAX_TOTAL',32):
            report=self.inspect(fonts)
        self.assertTrue(report['accepted'])
        self.assertEqual(report['fonts'][0]['programIdentity']['reason'],'DECODE_LIMIT')
        self.assertEqual(report['fonts'][1]['programIdentity']['reason'],'TOTAL_BYTE_LIMIT')

    def test_same_named_fonts_keep_distinct_program_identities(self):
        fonts=[]
        for data in (b'program A',b'program B'):
            stream=DecodedStreamObject();stream.set_data(data)
            fonts.append(dictionary(Type='Font',Subtype='TrueType',BaseFont='SameName',FontDescriptor=dictionary(FontFile2=stream)))
        report=self.inspect(fonts)
        self.assertEqual([f['baseFont'] for f in report['fonts']],['SameName','SameName'])
        self.assertNotEqual(report['fonts'][0]['programIdentity']['sha256'],report['fonts'][1]['programIdentity']['sha256'])

    def test_shared_program_is_identified_once_and_does_not_consume_budget_twice(self):
        path=Path(__file__).resolve().parents[1]/'tests/corpus/fonts/punctuation-quality.pdf'
        with patch('font_policy.identify', wraps=__import__('font_identity').identify) as measured:
            report=inspect_fonts(path)
        self.assertEqual(measured.call_count,2)
        identities=[font['programIdentity'] for font in report['fonts']]
        self.assertEqual(len(identities),4)
        unique={value['sha256']:value['bytes'] for value in identities}
        self.assertEqual(len(unique),2)
        with patch('font_policy.MAX_TOTAL',sum(unique.values())):
            self.assertTrue(all(font['programIdentity']['status']=='HASHED' for font in inspect_fonts(path)['fonts']))

    def test_owned_corpora_are_embedded(self):
        for name in ('font-quality.pdf', 'punctuation-quality.pdf'):
            report = inspect_fonts(Path(__file__).resolve().parents[1]/'tests/corpus/fonts'/name)
            self.assertTrue(report['fonts'])
            self.assertFalse(report['standard14Substitution'])
            self.assertTrue(all(font['state'] == 'EMBEDDED' for font in report['fonts']))


if __name__ == '__main__':
    unittest.main()
