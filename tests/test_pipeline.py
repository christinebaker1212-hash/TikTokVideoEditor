import json
import subprocess

import numpy as np
import pytest

from editor import camera, compositor, render, sync, tracking
from editor.config import OUT_H, OUT_W, normalize_project
from editor.media import probe

from .conftest import FPS, UI_LEAD, W, H, fighter_positions


def make_project(media, **timeline):
    p = normalize_project({"sources": {"ui": media["ui"], "clean": media["clean"]}})
    p["sync"]["offset"] = UI_LEAD
    p["timeline"].update(timeline)
    return p


def test_sync_finds_offset(media):
    result = sync.sync_files(media["ui"], media["clean"])
    assert result["offset"] == pytest.approx(UI_LEAD, abs=0.002)
    assert result["confidence"] > 0.3


def test_sync_negative_offset(media):
    # Swapping the roles must give the opposite sign.
    result = sync.sync_files(media["clean"], media["ui"])
    assert result["offset"] == pytest.approx(-UI_LEAD, abs=0.002)


def test_tracking_follows_fighters(media):
    tr = tracking.track_video(media["clean"])
    assert tr["frames"] == pytest.approx(5 * FPS, abs=2)
    cx = tracking.smooth(np.array(tr["centerX"]), 2)
    errors = []
    for i in range(5, tr["frames"] - 5):
        a, b = fighter_positions(i / FPS + UI_LEAD)
        errors.append(abs(cx[i] - (a + b) / 2))
    assert np.median(errors) < 0.02 * W
    assert np.max(errors) < 0.06 * W


def test_timeline_parts_and_keyframes(media):
    p = make_project(media, **{"in": 0.5, "superStart": 1.5, "animStart": 2.0, "postStart": 3.0, "out": 4.0})
    p["camera"]["super"]["keyframes"] = [{"t": 2.5, "cx": 300, "cy": 180, "zoom": 1.5}]
    p["camera"]["transitions"]["superToPost"] = 0.0
    tl = camera.build_timeline(p, None, FPS, W, H, 5.0)
    assert len(tl["times"]) == int(3.5 * FPS)
    parts = tl["part"]
    assert parts[0] == 0 and parts[int(1.2 * FPS)] == 1 and parts[int(1.7 * FPS)] == 2 and parts[-1] == 3
    s = camera.sample(tl, 2.5)
    assert s["zoom"] == pytest.approx(1.5)
    assert s["cx"] == pytest.approx(300)
    # Holds the last keyframe until the post section.
    assert camera.sample(tl, 2.9)["zoom"] == pytest.approx(1.5)
    assert camera.sample(tl, 3.1)["zoom"] == pytest.approx(1.0)
    # Animation part hides the UI by default.
    assert camera.sample(tl, 2.6)["alphas"]["health"] == 0.0
    assert camera.sample(tl, 1.0)["alphas"]["health"] == 1.0


def test_camera_is_clamped_inside_source():
    p = normalize_project({})
    p["camera"]["combo"]["offsetX"] = -5000
    tl = camera.build_timeline(p, None, 30, 1920, 1080, 2.0)
    cw, _ = camera.crop_size(1.0, 1080)
    assert min(tl["cx"]) == pytest.approx(cw / 2)


def test_feather_mask_and_overlay():
    m = compositor.feather_mask(100, 20, 10, 0, 0, 0)
    assert m[10, 0, 0] < 0.1 and m[10, 50, 0] == 1.0 and m[10, 99, 0] == 1.0
    out = np.zeros((OUT_H, OUT_W, 3), np.uint8)
    ui = np.zeros((1080, 1920, 3), np.uint8)
    ui[52:98, 1040:1755] = (0, 0, 255)
    el = {"src": {"x": 1040, "y": 52, "w": 715, "h": 46}, "dst": {"x": 0, "y": 165, "w": 1080, "h": 69},
          "opacity": 0.5, "feather": {}}
    compositor.draw_element(out, ui, el, 1.0)
    assert tuple(out[200, 540]) == (0, 0, 128)
    assert out[100, 540].sum() == 0


def test_export_end_to_end(media, tmp_path):
    p = make_project(media, **{"in": 0.5, "superStart": 2.0, "animStart": 2.5, "postStart": 3.0, "out": 4.0})
    p["camera"]["super"]["keyframes"] = [{"t": 2.5, "cx": 320, "cy": 150, "zoom": 1.4}]
    p["music"].update({"path": media["music"], "fadeIn": 0.5, "fadeOut": 0.5})
    p["export"]["preset"] = "ultrafast"
    out = render.export(p, str(tmp_path / "out.mp4"))
    info = probe(out)
    assert (info.width, info.height) == (OUT_W, OUT_H)
    assert info.duration == pytest.approx(3.5, abs=0.1)
    assert info.has_audio

    # The health bar from the UI video must appear at its destination in the output.
    frame = subprocess.run(["ffmpeg", "-v", "error", "-ss", "0.5", "-i", out, "-frames:v", "1", "-f", "rawvideo",
                            "-pix_fmt", "bgr24", "-"], capture_output=True).stdout
    img = np.frombuffer(frame, np.uint8).reshape(OUT_H, OUT_W, 3)
    b, g, r = img[200, 540].astype(int)
    assert r > 180 and b < 80


def test_still_matches_layout(media):
    p = make_project(media, **{"in": 0.5, "out": 4.0})
    img = render.render_still(p, 1.0)
    assert img.shape == (OUT_H, OUT_W, 3)


def test_server_api(media, tmp_path):
    from fastapi.testclient import TestClient

    from editor.server import app

    c = TestClient(app)
    assert c.post("/api/probe", json={"path": media["clean"]}).json()["fps"] == pytest.approx(FPS)
    p = make_project(media, **{"in": 0.5, "out": 4.0})
    tl = c.post("/api/timeline", json=p).json()
    assert tl["tracked"] is False
    c.post("/api/track", json=p).raise_for_status()
    tl = c.post("/api/timeline", json=p).json()
    assert tl["tracked"] is True and len(tl["cx"]) == int(3.5 * FPS)

    skin = c.get("/api/presets/skins/Default").json()
    skin["elements"][0]["opacity"] = 0.4
    c.put("/api/presets/skins/My Skin", json=skin).raise_for_status()
    assert "My Skin" in c.get("/api/presets/skins").json()
    assert c.get("/api/presets/skins/My Skin").json()["elements"][0]["opacity"] == 0.4
    c.delete("/api/presets/skins/My Skin").raise_for_status()
    assert "My Skin" not in c.get("/api/presets/skins").json()

    r = c.get("/api/media", params={"path": media["clean"]}, headers={"Range": "bytes=0-99"})
    assert r.status_code == 206 and len(r.content) == 100
