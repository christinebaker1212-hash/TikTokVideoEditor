//! Playback engine: lockstep video decoders with frame caches, and an audio
//! output that doubles as the master clock.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::Instant;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

use combo_editor::media::{ReaderOptions, VideoInfo, VideoReader, YuvFrame};

// ---------------------------------------------------------------- video

struct DecState {
    /// Frame index (at the master frame rate) the UI wants to show next.
    want: i64,
    ahead: i64,
    cache: BTreeMap<i64, Arc<YuvFrame>>,
    stop: bool,
}

/// Decodes one video on a background thread, keeping frames around the
/// requested index cached. Index `i` is the frame at `i / fps` seconds.
pub struct Decoder {
    shared: Arc<(Mutex<DecState>, Condvar)>,
    thread: Option<JoinHandle<()>>,
}

const BEHIND: i64 = 10;
/// Jumping further ahead than this restarts ffmpeg at the target instead of decoding through.
const SKIP_FRAMES: f64 = 1.5;

impl Decoder {
    pub fn new(info: VideoInfo, fps: f64, size: (u32, u32), hwaccel: bool, repaint: egui::Context) -> Decoder {
        let shared = Arc::new((Mutex::new(DecState { want: 0, ahead: 4, cache: BTreeMap::new(), stop: false }), Condvar::new()));
        let sh = shared.clone();
        let thread = std::thread::Builder::new()
            .name("decoder".into())
            .spawn(move || decode_loop(sh, info, fps, size, hwaccel, repaint))
            .expect("spawn decoder");
        Decoder { shared, thread: Some(thread) }
    }

    /// Ask for frame `index`, decoding `ahead` frames past it.
    pub fn request(&self, index: i64, ahead: i64) {
        let (m, cv) = &*self.shared;
        let mut st = m.lock().unwrap();
        if st.want != index || st.ahead != ahead {
            st.want = index;
            st.ahead = ahead;
            cv.notify_all();
        }
    }

    pub fn get(&self, index: i64) -> Option<Arc<YuvFrame>> {
        self.shared.0.lock().unwrap().cache.get(&index).cloned()
    }

    /// The closest cached frame at or before `index` (to keep showing something while seeking).
    pub fn nearest(&self, index: i64) -> Option<(i64, Arc<YuvFrame>)> {
        let st = self.shared.0.lock().unwrap();
        st.cache.range(..=index).next_back().or_else(|| st.cache.iter().next()).map(|(k, v)| (*k, v.clone()))
    }
}

impl Drop for Decoder {
    fn drop(&mut self) {
        {
            let (m, cv) = &*self.shared;
            m.lock().unwrap().stop = true;
            cv.notify_all();
        }
        // The thread may be blocked in a read; it exits on its own once that returns.
        if let Some(t) = self.thread.take() {
            drop(t);
        }
    }
}

fn decode_loop(shared: Arc<(Mutex<DecState>, Condvar)>, info: VideoInfo, fps: f64, size: (u32, u32), hwaccel: bool, repaint: egui::Context) {
    let (m, cv) = &*shared;
    let mut reader: Option<VideoReader> = None;
    let mut next: i64 = 0;
    let skip = (SKIP_FRAMES * fps) as i64;
    let opts = ReaderOptions { width: size.0, height: size.1, fps, hwaccel };
    loop {
        // Find the first missing frame in [want, want + ahead].
        let target = {
            let mut st = m.lock().unwrap();
            loop {
                if st.stop {
                    return;
                }
                let (want, ahead) = (st.want, st.ahead);
                let missing = (want..=want + ahead).find(|i| !st.cache.contains_key(i));
                // Evict frames far from the playhead.
                let keep = (want - BEHIND)..=(want + ahead.max(4) * 2);
                st.cache.retain(|k, _| keep.contains(k));
                match missing {
                    Some(i) => break i,
                    None => st = cv.wait(st).unwrap(),
                }
            }
        };
        if reader.is_none() || target < next || target > next + skip {
            reader = VideoReader::open(&info, target as f64 / fps, &opts).ok();
            next = target;
            if reader.is_none() {
                // ffmpeg failed to start; avoid a busy loop.
                std::thread::sleep(std::time::Duration::from_millis(200));
                continue;
            }
        }
        let frame = reader.as_mut().unwrap().read();
        let mut st = m.lock().unwrap();
        if next >= st.want - BEHIND {
            st.cache.insert(next, frame);
        }
        let shown = next == st.want;
        next += 1;
        drop(st);
        if shown {
            repaint.request_repaint();
        }
    }
}

// ---------------------------------------------------------------- audio

#[derive(Clone, Default)]
pub struct MusicParams {
    pub start: f64,
    pub volume: f32,
    pub fade_in: f64,
    pub fade_out: f64,
    pub t_in: f64,
    pub t_out: f64,
    pub enabled: bool,
}

#[derive(Default)]
struct Mix {
    playing: bool,
    /// Current position on the clean timeline (seconds).
    pos: f64,
    game: Option<Arc<Vec<f32>>>, // stereo interleaved at the device rate
    game_offset: f64,           // source time = clean time + offset
    game_volume: f32,
    music: Option<Arc<Vec<f32>>>,
    music_params: MusicParams,
}

pub struct Audio {
    _stream: Option<cpal::Stream>,
    mix: Arc<Mutex<Mix>>,
    /// Bits of the f64 position written by the audio callback, and when.
    clock: Arc<(AtomicU64, Mutex<Instant>)>,
    pub rate: u32,
    pub available: bool,
    /// Fallback clock when there is no audio device.
    started: Option<(Instant, f64)>,
}

impl Audio {
    pub fn new() -> Audio {
        let mix = Arc::new(Mutex::new(Mix { game_volume: 1.0, ..Default::default() }));
        let clock = Arc::new((AtomicU64::new(0f64.to_bits()), Mutex::new(Instant::now())));
        match build_stream(mix.clone(), clock.clone()) {
            Ok((stream, rate)) => Audio { _stream: Some(stream), mix, clock, rate, available: true, started: None },
            Err(e) => {
                combo_editor::paths::log(&format!("audio output unavailable: {e}"));
                Audio { _stream: None, mix, clock, rate: 48000, available: false, started: None }
            }
        }
    }

    pub fn set_game(&self, samples: Option<Arc<Vec<f32>>>, offset: f64, volume: f32) {
        let mut m = self.mix.lock().unwrap();
        m.game = samples;
        m.game_offset = offset;
        m.game_volume = volume;
    }

    pub fn set_game_params(&self, offset: f64, volume: f32) {
        let mut m = self.mix.lock().unwrap();
        m.game_offset = offset;
        m.game_volume = volume;
    }

    pub fn set_music(&self, samples: Option<Arc<Vec<f32>>>) {
        self.mix.lock().unwrap().music = samples;
    }

    pub fn set_music_params(&self, p: MusicParams) {
        self.mix.lock().unwrap().music_params = p;
    }

    pub fn play(&mut self, from: f64) {
        let mut m = self.mix.lock().unwrap();
        m.pos = from;
        m.playing = true;
        self.clock.0.store(from.to_bits(), Ordering::Relaxed);
        *self.clock.1.lock().unwrap() = Instant::now();
        self.started = Some((Instant::now(), from));
    }

    pub fn pause(&mut self) {
        self.mix.lock().unwrap().playing = false;
        self.started = None;
    }

    /// Current playback time on the clean timeline.
    pub fn time(&self) -> f64 {
        if !self.available {
            return self.started.map_or(0.0, |(at, t0)| t0 + at.elapsed().as_secs_f64());
        }
        let pos = f64::from_bits(self.clock.0.load(Ordering::Relaxed));
        // Interpolate between callbacks so video advances smoothly.
        let since = self.clock.1.lock().unwrap().elapsed().as_secs_f64().min(0.05);
        pos + since
    }
}

fn build_stream(mix: Arc<Mutex<Mix>>, clock: Arc<(AtomicU64, Mutex<Instant>)>) -> anyhow::Result<(cpal::Stream, u32)> {
    let host = cpal::default_host();
    let device = host.default_output_device().ok_or_else(|| anyhow::anyhow!("no output device"))?;
    let config = device.default_output_config()?;
    let rate = config.sample_rate();
    let channels = config.channels() as usize;
    let fmt = config.sample_format();
    let cfg: cpal::StreamConfig = config.into();
    macro_rules! build {
        ($t:ty) => {{
            let mix = mix.clone();
            let clock = clock.clone();
            device.build_output_stream(
                cfg.clone(),
                move |out: &mut [$t], _| {
                    let mut tmp = vec![0f32; out.len()];
                    fill(&mix, &clock, &mut tmp, channels, rate);
                    for (o, s) in out.iter_mut().zip(tmp) {
                        *o = <$t as cpal::FromSample<f32>>::from_sample_(s);
                    }
                },
                |e: cpal::Error| eprintln!("audio stream error: {e}"),
                None,
            )?
        }};
    }
    let stream = match fmt {
        cpal::SampleFormat::F32 => build!(f32),
        cpal::SampleFormat::I16 => build!(i16),
        cpal::SampleFormat::U16 => build!(u16),
        cpal::SampleFormat::I32 => build!(i32),
        other => anyhow::bail!("unsupported sample format {other:?}"),
    };
    stream.play()?;
    Ok((stream, rate))
}

fn fill(mix: &Mutex<Mix>, clock: &(AtomicU64, Mutex<Instant>), out: &mut [f32], channels: usize, rate: u32) {
    let Ok(mut m) = mix.lock() else { return };
    out.fill(0.0);
    if !m.playing {
        return;
    }
    let dt = 1.0 / rate as f64;
    let frames = out.len() / channels.max(1);
    // The buffer being filled now becomes audible about one buffer from now, so
    // publish its start as "now" for the video clock.
    let start = m.pos;
    let mp = m.music_params.clone();
    for f in 0..frames {
        let t = m.pos + f as f64 * dt;
        let (mut l, mut r) = (0.0f32, 0.0f32);
        if let Some(g) = &m.game {
            let i = ((t + m.game_offset) * rate as f64).floor();
            if i >= 0.0 && (i as usize) * 2 + 1 < g.len() {
                let i = i as usize * 2;
                l += g[i] * m.game_volume;
                r += g[i + 1] * m.game_volume;
            }
        }
        if let (Some(mu), true) = (&m.music, mp.enabled) {
            if t >= mp.t_in && t < mp.t_out {
                let local = t - mp.t_in;
                let len = mp.t_out - mp.t_in;
                let mut gain = mp.volume as f64;
                if mp.fade_in > 0.0 {
                    gain *= (local / mp.fade_in).clamp(0.0, 1.0);
                }
                if mp.fade_out > 0.0 {
                    gain *= ((len - local) / mp.fade_out).clamp(0.0, 1.0);
                }
                let i = ((mp.start + local) * rate as f64) as usize * 2;
                if i + 1 < mu.len() {
                    l += mu[i] * gain as f32;
                    r += mu[i + 1] * gain as f32;
                }
            }
        }
        let o = &mut out[f * channels..(f + 1) * channels];
        match channels {
            1 => o[0] = ((l + r) * 0.5).clamp(-1.0, 1.0),
            _ => {
                o[0] = l.clamp(-1.0, 1.0);
                o[1] = r.clamp(-1.0, 1.0);
            }
        }
    }
    m.pos += frames as f64 * dt;
    clock.0.store(start.to_bits(), Ordering::Relaxed);
    if let Ok(mut at) = clock.1.lock() {
        *at = Instant::now();
    }
}
