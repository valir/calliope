//! Audio output backends (plan 2.4): the real device (`CpalBackend`), a silent paced one for
//! GUI tests (`NullBackend`, optional float32 WAV capture) and a test-driven one
//! (`ManualBackend`). Tests never construct `CpalBackend` (a static test limits where it is
//! named), and builds with `e2e-hooks` can never select it.

#![allow(dead_code)] // used by the editor session (task 7) and the tests

use std::fs::File;
use std::io::{BufWriter, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample};

/// Fills an interleaved stereo f32 block.
pub type RenderFn = Box<dyn FnMut(&mut [f32]) + Send>;

pub trait OutputBackend: Send + Sync {
    fn name(&self) -> &'static str;
    /// Starts a stream at `sample_rate` that calls `render` for every block. Dropping the
    /// handle closes the stream.
    fn open(&self, sample_rate: u32, render: RenderFn) -> Result<Box<dyn OutputHandle>, String>;
}

pub trait OutputHandle: Send {
    /// A device failure that happened after opening, if any.
    fn error(&self) -> Option<String>;
}

fn log_open(name: &str, rate: u32) {
    eprintln!("calliope: audio backend={name} rate={rate}");
}

// ---------------------------------------------------------------- selection

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BackendKind {
    Cpal,
    Null,
    Capture(PathBuf),
}

/// Pure choice. Without `e2e-hooks` it is always the real device; with it, never.
/// `env` is the value of `CALLIOPE_E2E_AUDIO`; `capture:<absolute path>` adds the WAV capture.
pub fn backend_kind(e2e_hooks: bool, env: Option<&str>) -> BackendKind {
    if !e2e_hooks {
        return BackendKind::Cpal;
    }
    match env.and_then(|v| v.strip_prefix("capture:")) {
        Some(p) if Path::new(p).is_absolute() => BackendKind::Capture(PathBuf::from(p)),
        _ => BackendKind::Null,
    }
}

pub fn select_backend() -> Box<dyn OutputBackend> {
    let env = std::env::var("CALLIOPE_E2E_AUDIO").ok();
    match backend_kind(cfg!(feature = "e2e-hooks"), env.as_deref()) {
        BackendKind::Cpal => Box::new(CpalBackend),
        BackendKind::Null => Box::new(NullBackend::new(None)),
        BackendKind::Capture(p) => Box::new(NullBackend::new(Some(p))),
    }
}

// --------------------------------------------------------------------- null

/// Calls `render` with 10 ms blocks paced by the wall clock; plays nothing.
pub struct NullBackend {
    capture: Option<PathBuf>,
}

impl NullBackend {
    pub fn new(capture: Option<PathBuf>) -> Self {
        Self { capture }
    }
}

struct NullHandle {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl OutputHandle for NullHandle {
    fn error(&self) -> Option<String> {
        None
    }
}

impl Drop for NullHandle {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl OutputBackend for NullBackend {
    fn name(&self) -> &'static str {
        if self.capture.is_some() {
            "capture"
        } else {
            "null"
        }
    }

    fn open(&self, sample_rate: u32, mut render: RenderFn) -> Result<Box<dyn OutputHandle>, String> {
        let mut wav = match &self.capture {
            Some(p) => Some(WavWriter::create(p, sample_rate).map_err(|e| format!("capture file: {e}"))?),
            None => None,
        };
        log_open(self.name(), sample_rate);
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let block = (sample_rate / 100).max(1) as usize;
        let thread = std::thread::spawn(move || {
            let mut buf = vec![0.0f32; block * 2];
            let start = Instant::now();
            let mut n: u32 = 0;
            while !flag.load(Ordering::SeqCst) {
                buf.fill(0.0);
                render(&mut buf);
                if let Some(w) = wav.as_mut() {
                    if w.write(&buf).is_err() {
                        wav = None;
                    }
                }
                n += 1;
                let due = start + Duration::from_millis(10 * u64::from(n));
                while !flag.load(Ordering::SeqCst) {
                    let now = Instant::now();
                    if now >= due {
                        break;
                    }
                    std::thread::sleep((due - now).min(Duration::from_millis(2)));
                }
            }
            if let Some(w) = wav {
                let _ = w.finish();
            }
        });
        Ok(Box::new(NullHandle { stop, thread: Some(thread) }))
    }
}

/// Float32 stereo WAV, header patched on `finish`.
struct WavWriter {
    out: BufWriter<File>,
    bytes: u32,
}

impl WavWriter {
    fn create(path: &Path, rate: u32) -> std::io::Result<Self> {
        let mut out = BufWriter::new(File::create(path)?);
        let mut h = Vec::with_capacity(44);
        h.extend_from_slice(b"RIFF");
        h.extend_from_slice(&36u32.to_le_bytes());
        h.extend_from_slice(b"WAVEfmt ");
        h.extend_from_slice(&16u32.to_le_bytes());
        h.extend_from_slice(&3u16.to_le_bytes()); // IEEE float
        h.extend_from_slice(&2u16.to_le_bytes());
        h.extend_from_slice(&rate.to_le_bytes());
        h.extend_from_slice(&(rate * 8).to_le_bytes());
        h.extend_from_slice(&8u16.to_le_bytes());
        h.extend_from_slice(&32u16.to_le_bytes());
        h.extend_from_slice(b"data");
        h.extend_from_slice(&0u32.to_le_bytes());
        out.write_all(&h)?;
        Ok(Self { out, bytes: 0 })
    }

    fn write(&mut self, samples: &[f32]) -> std::io::Result<()> {
        for s in samples {
            self.out.write_all(&s.to_le_bytes())?;
        }
        self.bytes = self.bytes.saturating_add(samples.len() as u32 * 4);
        Ok(())
    }

    fn finish(mut self) -> std::io::Result<()> {
        self.out.flush()?;
        let f = self.out.get_mut();
        f.seek(SeekFrom::Start(4))?;
        f.write_all(&self.bytes.saturating_add(36).to_le_bytes())?;
        f.seek(SeekFrom::Start(40))?;
        f.write_all(&self.bytes.to_le_bytes())?;
        f.sync_all()
    }
}

// --------------------------------------------------------------------- cpal

/// The real default output device. Only `select_backend` and `gui.rs` may name it.
pub struct CpalBackend;

struct CpalHandle {
    stop: Option<mpsc::Sender<()>>,
    thread: Option<JoinHandle<()>>,
    error: Arc<Mutex<Option<String>>>,
}

impl OutputHandle for CpalHandle {
    fn error(&self) -> Option<String> {
        self.error.lock().ok().and_then(|e| e.clone())
    }
}

impl Drop for CpalHandle {
    fn drop(&mut self) {
        drop(self.stop.take()); // the stream thread wakes up, drops the stream and ends
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl OutputBackend for CpalBackend {
    fn name(&self) -> &'static str {
        "cpal"
    }

    fn open(&self, sample_rate: u32, render: RenderFn) -> Result<Box<dyn OutputHandle>, String> {
        let error = Arc::new(Mutex::new(None));
        let (stop_tx, stop_rx) = mpsc::channel::<()>();
        let (ready_tx, ready_rx) = mpsc::channel::<Result<(), String>>();
        let err_slot = error.clone();
        // The cpal::Stream is !Send: it is created, kept and dropped on this thread.
        let thread = std::thread::spawn(move || {
            match build_stream(sample_rate, render, err_slot) {
                Ok(stream) => {
                    let _ = ready_tx.send(Ok(()));
                    let _ = stop_rx.recv(); // returns when the handle drops its sender
                    drop(stream);
                }
                Err(e) => {
                    let _ = ready_tx.send(Err(e));
                }
            }
        });
        match ready_rx.recv() {
            Ok(Ok(())) => {
                log_open("cpal", sample_rate);
                Ok(Box::new(CpalHandle { stop: Some(stop_tx), thread: Some(thread), error }))
            }
            Ok(Err(e)) => {
                let _ = thread.join();
                Err(e)
            }
            Err(_) => Err("The audio output failed to start".to_string()),
        }
    }
}

fn build_stream(
    rate: u32,
    render: RenderFn,
    error: Arc<Mutex<Option<String>>>,
) -> Result<cpal::Stream, String> {
    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .ok_or_else(|| "No audio output device is available".to_string())?;
    let unsupported = || format!("The audio output does not support {rate} Hz");
    let mut ranges: Vec<_> = device
        .supported_output_configs()
        .map_err(|e| format!("The audio output cannot be queried: {e}"))?
        .filter(|r| r.min_sample_rate() <= rate && rate <= r.max_sample_rate() && r.channels() >= 1)
        .collect();
    // Prefer stereo or more, then f32, then the fewest channels.
    ranges.sort_by_key(|r| (r.channels() < 2, r.sample_format() != SampleFormat::F32, r.channels()));
    let cfg = ranges
        .into_iter()
        .next()
        .and_then(|r| r.try_with_sample_rate(rate))
        .ok_or_else(unsupported)?;
    let format = cfg.sample_format();
    let config: cpal::StreamConfig = cfg.into();
    let on_err = move |e: cpal::Error| {
        if let Ok(mut slot) = error.lock() {
            *slot = Some(format!("The audio output failed: {e}"));
        }
    };
    let stream = match format {
        SampleFormat::F32 => typed::<f32>(&device, config, render, on_err),
        SampleFormat::F64 => typed::<f64>(&device, config, render, on_err),
        SampleFormat::I16 => typed::<i16>(&device, config, render, on_err),
        SampleFormat::I32 => typed::<i32>(&device, config, render, on_err),
        SampleFormat::U16 => typed::<u16>(&device, config, render, on_err),
        SampleFormat::U8 => typed::<u8>(&device, config, render, on_err),
        SampleFormat::I8 => typed::<i8>(&device, config, render, on_err),
        SampleFormat::I64 => typed::<i64>(&device, config, render, on_err),
        SampleFormat::U32 => typed::<u32>(&device, config, render, on_err),
        SampleFormat::U64 => typed::<u64>(&device, config, render, on_err),
        other => Err(format!("The audio output uses an unsupported sample format ({other})")),
    }?;
    stream.play().map_err(|e| format!("The audio output cannot start: {e}"))?;
    Ok(stream)
}

fn typed<T>(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    mut render: RenderFn,
    on_err: impl FnMut(cpal::Error) + Send + 'static,
) -> Result<cpal::Stream, String>
where
    T: SizedSample + FromSample<f32>,
{
    let channels = config.channels as usize;
    let mut scratch = vec![0.0f32; 8192 * 2];
    device
        .build_output_stream(
            config,
            move |data: &mut [T], _| {
                let frames = data.len() / channels;
                if scratch.len() < frames * 2 {
                    scratch.resize(frames * 2, 0.0);
                }
                let block = &mut scratch[..frames * 2];
                block.fill(0.0);
                render(block);
                for (out, src) in data.chunks_mut(channels).zip(block.chunks(2)) {
                    if channels == 1 {
                        out[0] = T::from_sample((src[0] + src[1]) * 0.5);
                    } else {
                        out[0] = T::from_sample(src[0]);
                        out[1] = T::from_sample(src[1]);
                        for extra in &mut out[2..] {
                            *extra = T::from_sample(0.0);
                        }
                    }
                }
            },
            on_err,
            None,
        )
        .map_err(|e| format!("The audio output cannot be opened: {e}"))
}

// ------------------------------------------------------------------- manual

/// Test backend: nothing runs until the test calls `pull`, which runs the real render callback.
#[cfg(test)]
#[derive(Clone, Default)]
pub struct ManualBackend {
    render: Arc<Mutex<Option<RenderFn>>>,
    error: Arc<Mutex<Option<String>>>,
}

#[cfg(test)]
struct ManualHandle {
    render: Arc<Mutex<Option<RenderFn>>>,
    error: Arc<Mutex<Option<String>>>,
}

#[cfg(test)]
impl ManualBackend {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_open(&self) -> bool {
        self.render.lock().unwrap().is_some()
    }

    /// Renders `frames` stereo frames (zeros when no stream is open).
    pub fn pull(&self, frames: usize) -> Vec<f32> {
        let mut out = vec![0.0f32; frames * 2];
        if let Some(r) = self.render.lock().unwrap().as_mut() {
            r(&mut out);
        }
        out
    }

    /// Simulates a device failure for `handle.error()`.
    pub fn fail(&self, msg: &str) {
        *self.error.lock().unwrap() = Some(msg.to_string());
    }
}

#[cfg(test)]
impl OutputBackend for ManualBackend {
    fn name(&self) -> &'static str {
        "manual"
    }

    fn open(&self, _rate: u32, render: RenderFn) -> Result<Box<dyn OutputHandle>, String> {
        *self.error.lock().unwrap() = None;
        *self.render.lock().unwrap() = Some(render);
        Ok(Box::new(ManualHandle { render: self.render.clone(), error: self.error.clone() }))
    }
}

#[cfg(test)]
impl OutputHandle for ManualHandle {
    fn error(&self) -> Option<String> {
        self.error.lock().unwrap().clone()
    }
}

#[cfg(test)]
impl Drop for ManualHandle {
    fn drop(&mut self) {
        *self.render.lock().unwrap() = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    #[test]
    fn manual_pull_returns_what_render_produced() {
        let b = ManualBackend::new();
        assert_eq!(b.pull(2), vec![0.0; 4]);
        let mut n = 0.0f32;
        let h = b
            .open(
                44100,
                Box::new(move |out| {
                    for s in out.iter_mut() {
                        n += 1.0;
                        *s = n;
                    }
                }),
            )
            .unwrap();
        assert!(b.is_open());
        assert_eq!(b.pull(2), vec![1.0, 2.0, 3.0, 4.0]);
        assert_eq!(b.pull(1), vec![5.0, 6.0]);
        assert!(h.error().is_none());
        b.fail("boom");
        assert_eq!(h.error().as_deref(), Some("boom"));
        drop(h);
        assert!(!b.is_open());
        assert_eq!(b.pull(1), vec![0.0, 0.0]);
    }

    #[test]
    fn null_backend_runs_at_real_time_and_stops_on_drop() {
        let frames = Arc::new(AtomicUsize::new(0));
        let f = frames.clone();
        let h = NullBackend::new(None)
            .open(
                22050,
                Box::new(move |out| {
                    f.fetch_add(out.len() / 2, Ordering::SeqCst);
                }),
            )
            .unwrap();
        std::thread::sleep(Duration::from_millis(100));
        drop(h);
        let got = frames.load(Ordering::SeqCst) as f64;
        assert!((got - 2205.0).abs() <= 2205.0 * 0.3, "got {got} frames");
        std::thread::sleep(Duration::from_millis(50));
        assert_eq!(frames.load(Ordering::SeqCst) as f64, got, "render called after drop");
    }

    #[test]
    fn null_capture_writes_a_valid_float_wav() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cap.wav");
        let b = NullBackend::new(Some(path.clone()));
        assert_eq!(b.name(), "capture");
        let h = b
            .open(
                8000,
                Box::new(|out| {
                    for (i, s) in out.iter_mut().enumerate() {
                        *s = if i % 2 == 0 { 0.25 } else { -0.5 };
                    }
                }),
            )
            .unwrap();
        std::thread::sleep(Duration::from_millis(60));
        drop(h);
        let w = std::fs::read(&path).unwrap();
        assert_eq!(&w[0..4], b"RIFF");
        assert_eq!(&w[8..16], b"WAVEfmt ");
        assert_eq!(u16::from_le_bytes([w[20], w[21]]), 3);
        assert_eq!(u16::from_le_bytes([w[22], w[23]]), 2);
        assert_eq!(u32::from_le_bytes(w[24..28].try_into().unwrap()), 8000);
        assert_eq!(&w[36..40], b"data");
        let data_len = u32::from_le_bytes(w[40..44].try_into().unwrap()) as usize;
        assert_eq!(data_len, w.len() - 44);
        assert_eq!(u32::from_le_bytes(w[4..8].try_into().unwrap()) as usize, w.len() - 8);
        assert!(data_len >= 80 * 4 * 2, "at least one block was captured");
        for pair in w[44..].chunks(8) {
            assert_eq!(f32::from_le_bytes(pair[0..4].try_into().unwrap()), 0.25);
            assert_eq!(f32::from_le_bytes(pair[4..8].try_into().unwrap()), -0.5);
        }
        assert_eq!(NullBackend::new(None).name(), "null");
    }

    #[test]
    fn backend_kind_cases() {
        let abs = if cfg!(windows) { "C:\\x\\a.wav" } else { "/tmp/a.wav" };
        let cap = format!("capture:{abs}");
        // Without e2e-hooks: always the real device, whatever the env says.
        assert_eq!(backend_kind(false, None), BackendKind::Cpal);
        assert_eq!(backend_kind(false, Some(&cap)), BackendKind::Cpal);
        assert_eq!(backend_kind(false, Some("null")), BackendKind::Cpal);
        // With e2e-hooks: never Cpal.
        assert_eq!(backend_kind(true, None), BackendKind::Null);
        assert_eq!(backend_kind(true, Some("")), BackendKind::Null);
        assert_eq!(backend_kind(true, Some("cpal")), BackendKind::Null);
        assert_eq!(backend_kind(true, Some("capture")), BackendKind::Null);
        assert_eq!(backend_kind(true, Some("capture:")), BackendKind::Null);
        assert_eq!(backend_kind(true, Some("capture:rel/a.wav")), BackendKind::Null);
        assert_eq!(backend_kind(true, Some(&cap)), BackendKind::Capture(PathBuf::from(abs)));
    }

    #[test]
    fn e2e_hooks_build_never_selects_cpal() {
        if cfg!(feature = "e2e-hooks") {
            std::env::remove_var("CALLIOPE_E2E_AUDIO");
            assert_eq!(select_backend().name(), "null");
        }
    }
}
