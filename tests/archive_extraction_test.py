import importlib.util
import io
from pathlib import Path
import tarfile
import tempfile
import unittest

spec = importlib.util.spec_from_file_location("extractor", Path(__file__).resolve().parents[1] / "scripts/extract-pinned-archive.py")
extractor = importlib.util.module_from_spec(spec)
spec.loader.exec_module(extractor)


class ArchiveExtractionTests(unittest.TestCase):
    def test_supported_compression_and_deferred_links(self):
        for compression in ("xz", "zst"):
            with self.subTest(compression=compression), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                archive = root / "package.tar"
                with tarfile.open(archive, f"w:{compression}") as output:
                    member = tarfile.TarInfo("usr/bin/program")
                    member.size = 7
                    output.addfile(member, io.BytesIO(b"payload"))
                    link = tarfile.TarInfo("usr/bin/alias")
                    link.type = tarfile.LNKTYPE
                    link.linkname = member.name
                    output.addfile(link)
                links = extractor.extract(archive, root / "out")
                self.assertEqual((root / "out/usr/bin/program").read_bytes(), b"payload")
                self.assertFalse((root / "out/usr/bin/alias").exists())
                self.assertTrue(links[0]["HardLink"])

    def test_paths_and_links_cannot_escape_destination(self):
        for name, target in (("../escape", None), ("/absolute", None), ("C:/escape", None), ("usr/alias", "../../escape")):
            with self.subTest(name=name), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                archive = root / "package.tar"
                with tarfile.open(archive, "w") as output:
                    member = tarfile.TarInfo(name)
                    if target:
                        member.type = tarfile.SYMTYPE
                        member.linkname = target
                    output.addfile(member)
                with self.assertRaises(ValueError):
                    extractor.extract(archive, root / "out")


if __name__ == "__main__":
    unittest.main()
