import json
import os
from pathlib import Path, PurePosixPath
import shutil
import sys
import tarfile


def safe_path(root, name):
    path = PurePosixPath(name)
    if path.is_absolute() or "\\" in name or ":" in name:
        raise ValueError(f"Unsafe archive path: {name}")
    if any(os.path.isreserved(part) for part in path.parts if part not in (".", "..")):
        raise ValueError(f"Reserved archive path: {name}")
    result = (root / path).resolve()
    if not result.is_relative_to(root):
        raise ValueError(f"Archive path escapes destination: {name}")
    return result


def extract(archive, directory):
    root = Path(directory).resolve()
    root.mkdir(parents=True, exist_ok=True)
    links = []
    with tarfile.open(archive, "r:*") as source:
        for member in source:
            # Manual pages are not bundled and can contain Windows-invalid names.
            if member.name.startswith("usr/share/man/"):
                continue
            destination = safe_path(root, member.name)
            if member.issym() or member.islnk():
                target = member.linkname
                relative = target.lstrip("/") if member.islnk() or target.startswith("/") else str(PurePosixPath(member.name).parent / target)
                safe_path(root, relative)
                links.append({"Name": member.name, "Target": target, "HardLink": member.islnk()})
            elif member.isdir():
                destination.mkdir(parents=True, exist_ok=True)
            elif member.isfile():
                destination.parent.mkdir(parents=True, exist_ok=True)
                with source.extractfile(member) as data, destination.open("wb") as output:
                    shutil.copyfileobj(data, output)
            else:
                raise ValueError(f"Unsupported archive member: {member.name}")
    return links


if __name__ == "__main__":
    try:
        if sys.version_info < (3, 14):
            raise RuntimeError("Packaging requires Python 3.14 or later")
        print(json.dumps(extract(sys.argv[1], sys.argv[2])))
    except Exception as error:
        print(str(error), file=sys.stderr)
        sys.exit(1)
