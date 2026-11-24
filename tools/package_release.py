"""Packs the release zips the launcher's updater downloads (launcher/src/update.rs).

    python tools/package_release.py windows <server.exe> <launcher.exe> <out.zip>
    python tools/package_release.py linux <server> <launcher> <out.zip>

Each zip holds only the two binaries, under `echo-rust-server/`, marked
executable (Linux keeps the mode from the zip). Never settings, accounts or
logs: `launcher-setup.json`, `server-data/` and `balance-settings.json` hold
passwords and keys.
"""

import sys
import zipfile


def main() -> None:
    if len(sys.argv) != 5 or sys.argv[1] not in ("windows", "linux"):
        sys.exit(__doc__)
    platform, server, launcher, out = sys.argv[1:]
    ext = ".exe" if platform == "windows" else ""
    with zipfile.ZipFile(out, "w", zipfile.ZIP_DEFLATED) as z:
        for src, name in ((server, "echo-rust-server"), (launcher, "echo-server-launcher")):
            info = zipfile.ZipInfo(f"echo-rust-server/{name}{ext}", (1980, 1, 1, 0, 0, 0))
            info.compress_type = zipfile.ZIP_DEFLATED
            info.external_attr = 0o100755 << 16
            with open(src, "rb") as f:
                z.writestr(info, f.read())
    with zipfile.ZipFile(out) as z:
        for i in z.infolist():
            print(f"{out}: {i.filename} ({i.file_size} bytes)")


if __name__ == "__main__":
    main()
