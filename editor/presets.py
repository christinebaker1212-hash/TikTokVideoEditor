"""JSON preset storage: UI skins (layout + blend per element), blend presets and projects."""

from __future__ import annotations

import json
import re
from pathlib import Path

from .config import DEFAULT_BLEND, DEFAULT_SKIN, PRESET_DIR, WORKSPACE

KINDS = {
    "skins": (PRESET_DIR / "skins", DEFAULT_SKIN),
    "blends": (PRESET_DIR / "blends", DEFAULT_BLEND),
    "projects": (WORKSPACE / "projects", None),
}


def _slug(name: str) -> str:
    slug = re.sub(r"[^A-Za-z0-9._ -]+", "", name).strip().replace(" ", "_")
    if not slug or slug.startswith("."):
        raise ValueError("Invalid preset name")
    return slug[:80]


def _dir(kind: str) -> Path:
    if kind not in KINDS:
        raise ValueError(f"Unknown preset kind {kind!r}")
    d = KINDS[kind][0]
    d.mkdir(parents=True, exist_ok=True)
    return d


def list_presets(kind: str) -> list[str]:
    names = []
    for p in sorted(_dir(kind).glob("*.json")):
        try:
            names.append(json.loads(p.read_text()).get("name", p.stem))
        except (OSError, json.JSONDecodeError):
            continue
    default = KINDS[kind][1]
    if default and default["name"] not in names:
        names.insert(0, default["name"])
    return names


def load_preset(kind: str, name: str) -> dict:
    path = _dir(kind) / f"{_slug(name)}.json"
    if path.exists():
        return json.loads(path.read_text())
    default = KINDS[kind][1]
    if default and name == default["name"]:
        return json.loads(json.dumps(default))
    raise FileNotFoundError(name)


def save_preset(kind: str, name: str, data: dict) -> None:
    data = dict(data, name=name)
    (_dir(kind) / f"{_slug(name)}.json").write_text(json.dumps(data, indent=2))


def delete_preset(kind: str, name: str) -> None:
    path = _dir(kind) / f"{_slug(name)}.json"
    if path.exists():
        path.unlink()
