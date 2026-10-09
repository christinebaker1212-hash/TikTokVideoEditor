"""Align the UI and clean recordings using audio transients.

Both recordings carry the same game audio (no music), so hits, whiffs and
announcer lines line up exactly once the clips are offset correctly. We build
an onset-strength envelope (spectral flux) for each, cross-correlate the
envelopes for a coarse offset, then refine to sample precision with GCC-PHAT
on the raw audio in a small window around the coarse result.

Convention: ``ui_time = clean_time + offset``.
"""

from __future__ import annotations

import numpy as np

from .media import load_audio

SR = 16000
HOP = 160  # 10 ms
N_FFT = 1024


def onset_envelope(x: np.ndarray, sr: int = SR, hop: int = HOP, n_fft: int = N_FFT) -> np.ndarray:
    """Spectral-flux onset strength, one value per ``hop`` samples."""
    if len(x) < n_fft:
        x = np.pad(x, (0, n_fft - len(x)))
    n_frames = 1 + (len(x) - n_fft) // hop
    idx = np.arange(n_fft)[None, :] + hop * np.arange(n_frames)[:, None]
    frames = x[idx] * np.hanning(n_fft)[None, :].astype(np.float32)
    mag = np.log1p(100.0 * np.abs(np.fft.rfft(frames, axis=1)))
    flux = np.maximum(0.0, np.diff(mag, axis=0, prepend=mag[:1])).sum(axis=1)
    # Remove the slowly varying floor so only sharp transients remain.
    k = max(1, int(0.25 * sr / hop))
    floor = np.convolve(flux, np.ones(k) / k, mode="same")
    env = np.maximum(0.0, flux - floor)
    peak = env.max()
    return (env / peak).astype(np.float32) if peak > 0 else env.astype(np.float32)


def _xcorr(a: np.ndarray, b: np.ndarray) -> tuple[np.ndarray, np.ndarray]:
    """Full cross-correlation c[k] = sum a[n+k] b[n]; returns (lags, c)."""
    n = len(a) + len(b) - 1
    size = 1 << (n - 1).bit_length()
    c = np.fft.irfft(np.fft.rfft(a, size) * np.conj(np.fft.rfft(b, size)), size)
    c = np.concatenate([c[-(len(b) - 1):], c[: len(a)]]) if len(b) > 1 else c[: len(a)]
    lags = np.arange(-(len(b) - 1), len(a))
    return lags, c


def estimate_offset(ui: np.ndarray, clean: np.ndarray, sr: int = SR, max_lag: float | None = 20.0) -> dict:
    env_ui, env_clean = onset_envelope(ui, sr), onset_envelope(clean, sr)
    lags, c = _xcorr(env_ui - env_ui.mean(), env_clean - env_clean.mean())
    if max_lag is not None:
        keep = np.abs(lags) <= max_lag * sr / HOP
        lags, c = lags[keep], c[keep]
    best = int(np.argmax(c))
    coarse = lags[best] * HOP / sr

    # Confidence: how much the winning peak stands out from the runner-up
    # outside its immediate neighbourhood.
    others = c.copy()
    guard = int(0.05 * sr / HOP) + 1
    others[max(0, best - guard): best + guard + 1] = -np.inf
    second = others.max() if np.isfinite(others).any() else 0.0
    confidence = float(np.clip(1.0 - max(second, 0.0) / c[best], 0.0, 1.0)) if c[best] > 0 else 0.0

    offset = coarse + _refine(ui, clean, sr, coarse)
    return {"offset": float(offset), "coarse": float(coarse), "confidence": confidence}


def _refine(ui: np.ndarray, clean: np.ndarray, sr: int, coarse: float, window: float = 0.03) -> float:
    """GCC-PHAT on the overlapping audio, searched within +-window of ``coarse``."""
    shift = int(round(coarse * sr))
    # Overlap: ui[n + shift] corresponds to clean[n].
    start_c = max(0, -shift)
    end_c = min(len(clean), len(ui) - shift)
    if end_c - start_c < sr // 2:
        return 0.0
    end_c = min(end_c, start_c + 60 * sr)
    a = ui[start_c + shift: end_c + shift].astype(np.float64)
    b = clean[start_c:end_c].astype(np.float64)
    size = 1 << (2 * len(a) - 1).bit_length()
    cross = np.fft.rfft(a, size) * np.conj(np.fft.rfft(b, size))
    cross /= np.abs(cross) + 1e-12
    r = np.fft.irfft(cross, size)
    w = int(window * sr)
    cand = np.concatenate([r[-w:], r[: w + 1]])
    k = int(np.argmax(cand)) - w
    return k / sr


def sync_files(ui_path: str, clean_path: str, analyse: float = 60.0) -> dict:
    """Estimate the offset between two recordings from their first ``analyse`` seconds."""
    ui = load_audio(ui_path, SR, 0.0, analyse)
    clean = load_audio(clean_path, SR, 0.0, analyse)
    if len(ui) == 0 or len(clean) == 0:
        raise ValueError("Both videos need an audio track to auto-sync")
    result = estimate_offset(ui, clean)
    # Envelopes for the UI's visual sync check (first 15 s, 10 ms resolution).
    n = int(15 * SR / HOP)
    result["envelopes"] = {
        "hop": HOP / SR,
        "ui": np.round(onset_envelope(ui)[:n], 3).tolist(),
        "clean": np.round(onset_envelope(clean)[:n], 3).tolist(),
    }
    return result


def waveform(path: str, points_per_second: int = 50) -> dict:
    """Peak envelope of a file for drawing waveforms."""
    x = load_audio(path, 8000)
    hop = 8000 // points_per_second
    n = len(x) // hop
    peaks = np.abs(x[: n * hop]).reshape(n, hop).max(axis=1) if n else np.zeros(0)
    return {"rate": points_per_second, "peaks": np.round(peaks, 3).tolist()}
