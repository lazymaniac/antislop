"""Archive compatibility, exact bytes, corruption, and publication tests."""
import base64
import gzip
import hashlib
import importlib.util
import json
from pathlib import Path
import unittest
import zlib
import sys
sys.dont_write_bytecode = True
sys.path.insert(0, str(Path(__file__).resolve().parent / "support"))
from resources import resources

spec = importlib.util.spec_from_file_location(
    "reference_renders", Path(__file__).resolve().parents[1] / "scripts/reference_renders.py"
)
archive = importlib.util.module_from_spec(spec)
spec.loader.exec_module(archive)


class ReferenceArchiveTest(unittest.TestCase):
    def test_roundtrip_legacy_corruption_and_existing_output(self):
        raw = json.dumps({"history": [{"text": "👩‍💻é\\t", "style": ["Rgb(1,2,3)", "DIM"]}]}, ensure_ascii=False).encode()
        compressed = gzip.compress(raw, mtime=0)
        encoded = dict(format=archive.FORMAT, encoding="gzip+base64", decoded_format="legacy JSON", decoded_bytes=len(raw),
                       decoded_sha256=hashlib.sha256(raw).hexdigest(),
                       payload=[base64.b64encode(compressed[i:i+3]).decode() for i in range(0, len(compressed), 3)])
        with resources() as (root, _diagnostic, _env):
            source, result = root / "reference.json", root / "decoded.json"
            source.write_text(json.dumps(encoded))
            archive.decode(source, result)
            self.assertEqual(result.read_bytes(), raw)
            with self.assertRaises(FileExistsError):
                archive.decode(source, result)
            self.assertEqual(result.read_bytes(), raw)
            for mutation in [dict(decoded_bytes=1), dict(decoded_sha256="bad"),
                             dict(format="antislop-reference-archive-v2"), dict(payload=["!!!"]),
                             dict(payload=[123]), dict(decoded_bytes=True), dict(extra="unrecognized"),
                             dict(decoded_sha256="0"*64),
                             dict(payload=[base64.b64encode(compressed[:-8]).decode()]),
                             dict(payload=[base64.b64encode(compressed+b"trailing").decode()])]:
                source.write_text(json.dumps(encoded | mutation))
                target = root / "invalid.json"
                with self.assertRaises((ValueError, zlib.error)):
                    archive.decode(source, target)
                self.assertFalse(target.exists())
            source.write_text('{"format":"duplicate",' + json.dumps(encoded)[1:])
            with self.assertRaises(ValueError):
                archive.decode(source, root / "duplicate.json")
            self.assertFalse((root / "duplicate.json").exists())
            source.write_bytes(raw)
            archive.decode(source, root / "legacy.json")
            self.assertEqual((root / "legacy.json").read_bytes(), raw)
            archive.pack(source, root / "repacked.json")
            archive.decode(root / "repacked.json", root / "unpacked.json")
            self.assertEqual((root / "unpacked.json").read_bytes(), raw)


if __name__ == "__main__":
    unittest.main()
