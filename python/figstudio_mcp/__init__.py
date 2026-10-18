"""Thin `uvx` entry point that delegates to the native `figstudio-mcp` Rust binary."""
from __future__ import annotations

import os
import pathlib
import shutil
import subprocess
import sys


def _find_binary() -> str:
    self_dir = pathlib.Path(sys.argv[0]).resolve().parent
    for directory in os.environ.get("PATH", "").split(os.pathsep):
        if not directory:
            continue
        p = pathlib.Path(directory)
        try:
            if p.resolve() == self_dir:
                continue
        except OSError:
            continue
        candidate = shutil.which("figstudio-mcp", path=directory)
        if candidate:
            return candidate
    cargo_bin = pathlib.Path.home() / ".cargo" / "bin" / "figstudio-mcp"
    if cargo_bin.is_file() and os.access(cargo_bin, os.X_OK):
        return str(cargo_bin)
    repo_root = pathlib.Path(__file__).resolve().parents[2]
    if (repo_root / "Cargo.toml").is_file():
        subprocess.run(
            ["cargo", "install", "--path", str(repo_root / "apps" / "figstudio-mcp")],
            check=True,
            stdin=subprocess.DEVNULL,
            stdout=sys.stderr,
        )
        if cargo_bin.is_file():
            return str(cargo_bin)
    raise SystemExit("figstudio-mcp binary not found; run: cargo install --git https://github.com/pelvo/figstudio figstudio-mcp")


def main() -> None:
    binary = _find_binary()
    args = sys.argv[1:] or ["--browser-proxy"]
    os.execv(binary, [binary, *args])
