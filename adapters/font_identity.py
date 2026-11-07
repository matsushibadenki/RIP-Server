"""Bounded identities of embedded PDF font program streams; never glyph proof."""
import hashlib
from pypdf import apply_configuration
from pypdf.errors import LimitReachedError
from pypdf.filters import decode_stream_data
from pypdf.generic import ArrayObject, DictionaryObject, NullObject, EncodedStreamObject, NameObject

PROFILE = 'org.jrip.embedded-font-program-v1'
MAX_PROGRAM = 16 * 1024 * 1024
MAX_TOTAL = 64 * 1024 * 1024
FILTERS = frozenset(('/FlateDecode','/Fl','/LZWDecode','/LZW','/ASCIIHexDecode','/AHx',
                     '/ASCII85Decode','/A85','/RunLengthDecode','/RL'))


class IdentityResult(dict):
    # Internal accounting only; not serialized into public identity metadata.
    charged_bytes = 0


def identify(stream, remaining):
    raw = stream._data  # Pinned pypdf API: do not populate its unbounded decoded cache.
    result = IdentityResult(profile=PROFILE, status='UNAVAILABLE',
                            encodedSha256=hashlib.sha256(raw).hexdigest(), encodedBytes=len(raw))
    def unavailable(reason):
        result['reason'] = reason
        return result
    if remaining <= 0:
        return unavailable('TOTAL_BYTE_LIMIT')
    filters = stream.get('/Filter', ArrayObject())
    if hasattr(filters, 'get_object'): filters = filters.get_object()
    if isinstance(filters, str): filters = [filters]
    if not isinstance(filters, (list, ArrayObject)) or len(filters) > 4:
        return unavailable('FILTER_CHAIN_LIMIT')
    names=[]
    for item in filters:
        item=item.get_object() if hasattr(item,'get_object') else item
        if not isinstance(item,str) or item not in FILTERS:
            return unavailable('UNSUPPORTED_FILTER')
        names.append(str(item))
    params=stream.get('/DecodeParms')
    if hasattr(params,'get_object'): params=params.get_object()
    if params is None or isinstance(params, NullObject): params=[None]*len(names)
    elif isinstance(params,DictionaryObject) and len(names)==1: params=[params]
    if not isinstance(params,(list,ArrayObject)) or len(params)!=len(names):
        return unavailable('UNSUPPORTED_PARAMETERS')
    for name, value in zip(names, params):
        if hasattr(value,'get_object'): value=value.get_object()
        if value is None or isinstance(value,NullObject): continue
        if not isinstance(value,DictionaryObject): return unavailable('UNSUPPORTED_PARAMETERS')
        if name in ('/LZWDecode','/LZW'):
            # Pinned pypdf's LZW decoder handles the default EarlyChange=1 only,
            # and does not apply LZW predictors. Do not hash a wrong interpretation.
            if value.get('/EarlyChange',1)!=1 or value.get('/Predictor',1)!=1:
                return unavailable('UNSUPPORTED_PARAMETERS')
        elif name not in ('/FlateDecode','/Fl') and value:
            return unavailable('UNSUPPORTED_PARAMETERS')
    decoded=raw
    for name, parameters in zip(names or [None], params or [None]):
        limit=min(MAX_PROGRAM, remaining-result.charged_bytes)
        if limit<=0: return unavailable('TOTAL_BYTE_LIMIT')
        try:
            if name in ('/ASCIIHexDecode','/AHx','/ASCII85Decode','/A85'):
                # ASCII85's z shorthand expands four bytes from one input byte.
                # Bound before calling the library, which otherwise allocates first.
                clean=b''.join(decoded.split())
                if name in ('/ASCIIHexDecode','/AHx'):
                    maximum=(len(clean.rstrip(b'>'))+1)//2
                else:
                    if clean.startswith(b'<~'): clean=clean[2:]
                    if clean.endswith(b'~>'): clean=clean[:-2]
                    zeros=clean.count(b'z'); digits=len(clean)-zeros
                    maximum=zeros*4+(digits//5)*4+max(0,digits%5-1)
                if maximum>limit:
                    result.charged_bytes+=limit
                    return unavailable('DECODE_LIMIT')
            if name is not None:
                stage=EncodedStreamObject();stage._data=decoded
                stage[NameObject('/Filter')]=NameObject(name)
                if hasattr(parameters,'get_object'): parameters=parameters.get_object()
                if isinstance(parameters,DictionaryObject): stage[NameObject('/DecodeParms')]=parameters
                with apply_configuration(zlib_maximum_output_length=limit,
                                         zlib_maximum_recovery_input_length=0,
                                         lzw_maximum_output_length=limit,
                                         run_length_maximum_output_length=limit,
                                         stream_filters_maximum_length=1,
                                         stream_decoding_work_maximum_length=len(decoded)+2*limit,
                                         flate_maximum_columns=4096,
                                         flate_maximum_row_length=16384):
                    decoded=decode_stream_data(stage)
            if len(decoded)>limit:
                result.charged_bytes+=limit
                return unavailable('TOTAL_BYTE_LIMIT' if remaining-result.charged_bytes==0 else 'PROGRAM_BYTE_LIMIT')
            result.charged_bytes+=len(decoded)
        except LimitReachedError:
            result.charged_bytes+=limit
            return unavailable('DECODE_LIMIT')
        except Exception:
            result.charged_bytes+=limit
            return unavailable('DECODE_FAILED')
    if not decoded: return unavailable('EMPTY_PROGRAM')
    result.update(status='HASHED',sha256=hashlib.sha256(decoded).hexdigest(),bytes=len(decoded))
    return result
