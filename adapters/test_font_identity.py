import base64
import hashlib
import unittest
import zlib
from unittest.mock import patch
from pypdf._codecs._codecs import LzwCodec
from pypdf.generic import EncodedStreamObject, NameObject, ArrayObject, DictionaryObject, NumberObject
from font_identity import identify, MAX_TOTAL


def program(data, filters=None, params=None):
    stream=EncodedStreamObject();stream._data=data
    if filters is not None:
        stream[NameObject('/Filter')] = ArrayObject([NameObject(f) for f in filters]) if isinstance(filters,list) else NameObject(filters)
    if params is not None: stream[NameObject('/DecodeParms')]=params
    return stream


class FontIdentityTests(unittest.TestCase):
    def test_plain_flate_ascii_and_lzw_have_the_same_program_identity(self):
        source=bytes(range(256))*20
        cases=[program(source), program(zlib.compress(source),'/FlateDecode'),
               program(source.hex().encode()+b'>','/ASCIIHexDecode'),
               program(base64.a85encode(source)+b'~>','/ASCII85Decode'),
               program(LzwCodec().encode(source),'/LZWDecode'),
               program(base64.a85encode(zlib.compress(source))+b'~>', ['/ASCII85Decode','/FlateDecode'])]
        for stream in cases:
            record=identify(stream,MAX_TOTAL)
            self.assertEqual(record['status'],'HASHED',record)
            self.assertEqual(record['sha256'],hashlib.sha256(source).hexdigest())
            self.assertEqual(record['bytes'],len(source))
            self.assertEqual(record['encodedSha256'],hashlib.sha256(stream._data).hexdigest())
        self.assertNotEqual(identify(cases[0],MAX_TOTAL)['encodedSha256'],identify(cases[1],MAX_TOTAL)['encodedSha256'])

    def test_run_length_and_flate_predictors(self):
        source=b'ABCDEFGHabcdefgh'
        self.assertEqual(identify(program(bytes([len(source)-1])+source+b'\x80','/RunLengthDecode'),MAX_TOTAL)['sha256'],hashlib.sha256(source).hexdigest())
        encoded=b''.join(bytes([row[0]])+bytes((row[i]-row[i-1])%256 for i in range(1,len(row))) for row in [source[:8],source[8:]])
        params=DictionaryObject({NameObject('/Predictor'):NumberObject(2),NameObject('/Columns'):NumberObject(8)})
        self.assertEqual(identify(program(zlib.compress(encoded),'/FlateDecode',params),MAX_TOTAL)['sha256'],hashlib.sha256(source).hexdigest())

    def test_same_name_is_not_same_program_and_unknown_is_not_a_decoded_hash(self):
        self.assertNotEqual(identify(program(b'program A'),MAX_TOTAL)['sha256'],identify(program(b'program B'),MAX_TOTAL)['sha256'])
        for stream in [program(b'unknown','/JBIG2Decode'),program(b'bad','/FlateDecode'),program(b'bad','/LZWDecode',DictionaryObject({NameObject('/EarlyChange'):NumberObject(0)}))]:
            record=identify(stream,MAX_TOTAL)
            self.assertEqual(record['status'],'UNAVAILABLE')
            self.assertIn('reason',record)
            self.assertNotIn('sha256',record)
            self.assertNotIn('bytes',record)
            self.assertIn('encodedSha256',record)

    def test_ascii85_shorthand_and_failed_attempts_charge_budget(self):
        with patch('font_identity.MAX_PROGRAM',32):
            record=identify(program(b'z'*100+b'~>','/ASCII85Decode'),MAX_TOTAL)
            self.assertEqual(record['reason'],'DECODE_LIMIT')
            self.assertEqual(record.charged_bytes,32)
            failed=identify(program(zlib.compress(b'A'*128),'/FlateDecode'),32)
            self.assertEqual(failed.charged_bytes,32)
            self.assertEqual(identify(program(b'next'),32-failed.charged_bytes)['reason'],'TOTAL_BYTE_LIMIT')
        data=b'ABCD'*40
        compressed=zlib.compress(data)
        record=identify(program(base64.a85encode(compressed)+b'~>', ['/ASCII85Decode','/FlateDecode']),MAX_TOTAL)
        self.assertEqual(record['status'],'HASHED')
        self.assertEqual(record.charged_bytes,len(compressed)+len(data))

    def test_expansion_filter_depth_and_total_budget_are_bounded(self):
        with patch('font_identity.MAX_PROGRAM',32):
            for stream in [program(b'A'*128), program(zlib.compress(b'A'*128),'/FlateDecode'),program(b'\x81A'*2+b'\x80','/RunLengthDecode'),program(LzwCodec().encode(b'A'*128),'/LZWDecode')]:
                record=identify(stream,MAX_TOTAL)
                self.assertEqual(record['status'],'UNAVAILABLE',record)
                self.assertNotIn('sha256',record)
        self.assertEqual(identify(program(b'data'),0)['reason'],'TOTAL_BYTE_LIMIT')
        self.assertEqual(identify(program(b'data'),2)['reason'],'TOTAL_BYTE_LIMIT')
        self.assertEqual(identify(program(b'data',['/FlateDecode']*5),MAX_TOTAL)['reason'],'FILTER_CHAIN_LIMIT')


if __name__=='__main__': unittest.main()
