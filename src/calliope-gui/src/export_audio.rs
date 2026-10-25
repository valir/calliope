//! The audio pipeline of an export (plan sections 2.3 and 2.4): the count-in layout, the Player's
//! click mixed at exact frames, and the run that streams `ffmpeg` (decoder) through Rust (5 beats
//! of silence, the ticks) into the RC-5 WAV or into a second `ffmpeg` (encoder). No Tauri.

use std::ffi::OsString;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ChildStdin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{sync_channel, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use calliope_lib::process::{self, CancelHandle};

use crate::export_rules::ExportKind;
use crate::metronome::Click;
use crate::tempo_map::TempoMap;
use crate::wav::WavWriter;

pub use crate::metronome::{LEAD_BEATS, TICK_BEATS};

/// What `run` returns for a cancel.
pub const CANCELLED: &str = "cancelled";

/// Frames (not samples) per block of silence written at the start.
const LEAD_BLOCK_FRAMES: usize = 4096;
/// How long the job loop waits for decoded data before looking at the cancel flag.
const RECV_TIMEOUT: Duration = Duration::from_millis(50);

/// Where the ticks fall. All values are output frames at the export's rate.
#[derive(Debug, Clone, PartialEq)]
pub struct CountInPlan {
    pub bpm: f64,
    pub beat_frames: u64,
    pub lead_frames: u64,
    /// In the output: `lead + first`.
    pub first_beat_frame: u64,
    pub ticks: [u64; 6],
}

/// The count-in for the first section of `map` at `rate`; `None` for an empty map. The beat is
/// rounded once (`round(rate * 60 / bpm)`, as the Player's count-in), the first beat is
/// `round(start_ms * rate / 1000)` frames into the original, each tick is `first + round(offset * beat)`.
pub fn count_in_plan(map: &TempoMap, rate: u32) -> Option<CountInPlan> {
    let seg = map.segments.first()?;
    if !(seg.bpm.is_finite() && seg.bpm > 0.0) {
        return None;
    }
    let beat = (f64::from(rate) * 60.0 / seg.bpm).round() as u64;
    let lead = LEAD_BEATS * beat;
    let first = (seg.start_ms * f64::from(rate) / 1000.0).round().max(0.0) as u64;
    let mut ticks = [0u64; 6];
    for (t, off) in ticks.iter_mut().zip(TICK_BEATS) {
        *t = first + (off * beat as f64).round() as u64;
    }
    Some(CountInPlan {
        bpm: seg.bpm,
        beat_frames: beat,
        lead_frames: lead,
        first_beat_frame: lead + first,
        ticks,
    })
}

/// Adds the ticks that overlap `block` (interleaved, `channels`, starting at output frame
/// `start`). Only the samples under a tick are touched; they are clamped to [-1, 1]. Returns
/// how many samples were clamped.
pub(crate) fn mix_ticks(
    block: &mut [f32],
    channels: usize,
    start: u64,
    plan: &CountInPlan,
    click: &[f32],
) -> u32 {
    if channels == 0 {
        return 0;
    }
    let frames = (block.len() / channels) as u64;
    let end = start + frames;
    let mut clipped = 0;
    for &t in &plan.ticks {
        let t_end = t + click.len() as u64;
        if t >= end || t_end <= start {
            continue;
        }
        let from = t.max(start);
        let to = t_end.min(end);
        for f in from..to {
            let c = click[(f - t) as usize];
            let base = (f - start) as usize * channels;
            for s in &mut block[base..base + channels] {
                let v = *s + c;
                if v > 1.0 {
                    clipped += 1;
                    *s = 1.0;
                } else if v < -1.0 {
                    clipped += 1;
                    *s = -1.0;
                } else {
                    *s = v;
                }
            }
        }
    }
    clipped
}

fn os(s: &str) -> OsString {
    OsString::from(s)
}

fn file_arg(prefix: &str, p: &Path) -> OsString {
    let mut s = OsString::from(prefix);
    s.push(p.as_os_str());
    s
}

/// `ffmpeg` arguments that decode `src` (absolute) to raw f32 at `rate` / `channels` on stdout.
pub fn decoder_args(src: &Path, rate: u32, channels: u32) -> Vec<OsString> {
    let mut a: Vec<OsString> = [
        "-nostdin",
        "-hide_banner",
        "-loglevel",
        "error",
        "-protocol_whitelist",
        "file,pipe",
        "-i",
    ]
    .iter()
    .map(|s| os(s))
    .collect();
    a.push(file_arg("file:", src));
    for s in [
        "-map",
        "0:a:0",
        "-vn",
        "-sn",
        "-dn",
        "-f",
        "f32le",
        "-acodec",
        "pcm_f32le",
        "-ac",
    ] {
        a.push(os(s));
    }
    a.push(channels.to_string().into());
    a.push(os("-ar"));
    a.push(rate.to_string().into());
    a.push(os("pipe:1"));
    a
}

/// `ffmpeg` arguments that encode raw f32 from stdin into `part` (MP3 or Ogg Vorbis) with the
/// tags. `-y` because Calliope created the part file itself; the encoder writes to a file
/// (not a pipe) so that the VBR header can be patched by seeking back.
pub fn encoder_args(
    kind: ExportKind,
    rate: u32,
    channels: u32,
    tags: &[(&'static str, String)],
    part: &Path,
) -> Vec<OsString> {
    let mut a: Vec<OsString> = [
        "-nostdin",
        "-hide_banner",
        "-loglevel",
        "error",
        "-f",
        "f32le",
        "-ar",
    ]
    .iter()
    .map(|s| os(s))
    .collect();
    a.push(rate.to_string().into());
    a.push(os("-ac"));
    a.push(channels.to_string().into());
    for s in [
        "-i",
        "pipe:0",
        "-map_metadata",
        "-1",
        "-fflags",
        "+bitexact",
        "-flags:a",
        "+bitexact",
    ] {
        a.push(os(s));
    }
    let codec: &[&str] = match kind {
        ExportKind::Ogg => &["-c:a", "libvorbis", "-q:a", "6", "-f", "ogg"],
        _ => &[
            "-c:a",
            "libmp3lame",
            "-q:a",
            "2",
            "-id3v2_version",
            "3",
            "-write_id3v1",
            "0",
            "-f",
            "mp3",
        ],
    };
    for s in codec {
        a.push(os(s));
    }
    for (k, v) in tags {
        a.push(os("-metadata"));
        a.push(format!("{k}={v}").into());
    }
    a.push(os("-y"));
    a.push(file_arg("file:", part));
    a
}

/// What to render.
pub(crate) struct Pipeline {
    /// The decoder `ffmpeg`.
    pub ffmpeg: PathBuf,
    /// Absolute path of the source file.
    pub source: PathBuf,
    pub rate: u32,
    pub channels: u32,
    pub plan: CountInPlan,
    /// The part file being written (for the error texts).
    pub part: PathBuf,
    /// Tests only: the export takes `pace` times the audio's duration.
    pub pace: Option<f64>,
}

pub(crate) enum Output {
    /// The RC-5 file (stereo only).
    Wav(WavWriter),
    /// A second `ffmpeg` (the encoder) reading raw f32 on stdin.
    Encode {
        ffmpeg: PathBuf,
        args: Vec<OsString>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunStats {
    /// Frames written (lead + original).
    pub frames: u64,
    /// Samples clamped under a tick.
    pub clipped: u32,
}

enum Sink {
    Wav(WavWriter),
    Pipe(ChildStdin),
}

/// A child process with the last line it wrote to stderr.
struct Child {
    running: Option<process::Running>,
    handle: CancelHandle,
    last_err: Arc<Mutex<String>>,
}

impl Child {
    fn last(&self) -> String {
        self.last_err.lock().map(|s| s.clone()).unwrap_or_default()
    }

    fn wait(&mut self) -> std::io::Result<std::process::ExitStatus> {
        match self.running.take() {
            Some(r) => r.wait(),
            None => Err(std::io::Error::other("already waited")),
        }
    }
}

fn last_line_sink(last: &Arc<Mutex<String>>) -> impl FnMut(String) + Send + 'static {
    let last = last.clone();
    move |l| {
        if !l.trim().is_empty() {
            if let Ok(mut s) = last.lock() {
                *s = l;
            }
        }
    }
}

fn with_reason(what: String, last: &str) -> String {
    if last.is_empty() {
        what
    } else {
        format!("{what}: {last}")
    }
}

fn label(p: &Path) -> String {
    p.file_name().map_or_else(
        || p.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    )
}

/// Sleeps (in short steps, watching `cancel`) until `frames` of audio at `rate` have taken
/// `pace` times their duration since `t0`.
fn pace_wait(t0: Instant, frames: u64, rate: u32, pace: f64, cancel: &AtomicBool) {
    let target = Duration::from_secs_f64(frames as f64 / f64::from(rate) * pace);
    while !cancel.load(Ordering::SeqCst) {
        let el = t0.elapsed();
        if el >= target {
            break;
        }
        std::thread::sleep((target - el).min(Duration::from_millis(20)));
    }
}

struct Writer<'a> {
    sink: Option<Sink>,
    channels: usize,
    plan: &'a CountInPlan,
    click: Vec<f32>,
    pos: u64,
    clipped: u32,
    bytes: Vec<u8>,
}

impl Writer<'_> {
    /// Mixes the ticks into `block` (whole frames) and writes it.
    fn emit(&mut self, block: &mut [f32]) -> std::io::Result<()> {
        self.clipped += mix_ticks(block, self.channels, self.pos, self.plan, &self.click);
        self.pos += (block.len() / self.channels) as u64;
        match self.sink.as_mut() {
            Some(Sink::Wav(w)) => w.write(block),
            Some(Sink::Pipe(p)) => {
                self.bytes.clear();
                self.bytes.reserve(block.len() * 4);
                for s in block.iter() {
                    self.bytes.extend_from_slice(&s.to_le_bytes());
                }
                p.write_all(&self.bytes)
            }
            None => Ok(()),
        }
    }
}

/// Runs the export: the encoder first (when there is one), then the decoder; writes the
/// `lead` frames of silence, then the decoded original, with the ticks mixed in at their
/// frames. `progress(frames_written)` is called after every block. Errors are plain
/// messages; a cancel returns [`CANCELLED`].
pub(crate) fn run(
    p: &Pipeline,
    out: Output,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(u64),
) -> Result<RunStats, String> {
    let part = p.part.display().to_string();
    let channels = p.channels as usize;
    let mut encoder: Option<Child> = None;
    let sink = match out {
        Output::Wav(w) => Sink::Wav(w),
        Output::Encode { ffmpeg, args } => {
            let last = Arc::new(Mutex::new(String::new()));
            let (running, stdin) =
                process::spawn_with_stdin(&ffmpeg, &args, None, |_| {}, last_line_sink(&last))
                    .map_err(|e| format!("Cannot encode: {e}"))?;
            encoder = Some(Child {
                handle: running.cancel_handle(),
                running: Some(running),
                last_err: last,
            });
            Sink::Pipe(stdin)
        }
    };
    let mut writer = Writer {
        sink: Some(sink),
        channels,
        plan: &p.plan,
        click: Click::new(p.rate).samples().to_vec(),
        pos: 0,
        clipped: 0,
        bytes: Vec::new(),
    };

    let dec_args = decoder_args(&p.source, p.rate, p.channels);
    let dec_err = Arc::new(Mutex::new(String::new()));
    let (tx, rx) = sync_channel::<Vec<u8>>(8);
    let spawned = process::spawn_raw_stdout(
        &p.ffmpeg,
        &dec_args,
        None,
        move |chunk| {
            // A closed channel (the job stopped) just discards the rest.
            let _ = tx.send(chunk.to_vec());
        },
        last_line_sink(&dec_err),
    );
    let mut decoder = match spawned {
        Ok(r) => Child {
            handle: r.cancel_handle(),
            running: Some(r),
            last_err: dec_err,
        },
        Err(e) => {
            abort(&mut encoder, &mut writer);
            return Err(format!("Cannot decode {}: {e}", label(&p.source)));
        }
    };

    let write_err = |e: std::io::Error, enc: &mut Option<Child>| -> String {
        let is_pipe = enc.is_some();
        if is_pipe {
            // The encoder most likely died: its own message says why.
            let mut c = enc.take().expect("encoder");
            let _ = c.wait();
            let last = c.last();
            return if last.is_empty() {
                format!("Cannot encode: {e}")
            } else {
                format!("Cannot encode: {last}")
            };
        }
        format!("Cannot write {part}: {e}")
    };

    let t0 = Instant::now();
    let stop = |decoder: &mut Child, encoder: &mut Option<Child>, writer: &mut Writer| {
        decoder.handle.cancel();
        abort(encoder, writer);
        let _ = decoder.wait();
    };

    // 1. The silence of the count-in.
    let mut remaining = p.plan.lead_frames;
    let mut buf: Vec<f32> = Vec::new();
    while remaining > 0 {
        if cancel.load(Ordering::SeqCst) {
            drop(rx);
            stop(&mut decoder, &mut encoder, &mut writer);
            return Err(CANCELLED.into());
        }
        let n = remaining.min(LEAD_BLOCK_FRAMES as u64) as usize;
        buf.clear();
        buf.resize(n * channels, 0.0);
        if let Err(e) = writer.emit(&mut buf) {
            drop(rx);
            decoder.handle.cancel();
            drop(writer.sink.take());
            let msg = write_err(e, &mut encoder);
            let _ = decoder.wait();
            return Err(msg);
        }
        remaining -= n as u64;
        progress(writer.pos);
        if let Some(f) = p.pace {
            pace_wait(t0, writer.pos, p.rate, f, cancel);
        }
    }

    // 2. The original.
    let frame_bytes = channels * 4;
    let mut pending: Vec<u8> = Vec::new();
    loop {
        if cancel.load(Ordering::SeqCst) {
            drop(rx);
            stop(&mut decoder, &mut encoder, &mut writer);
            return Err(CANCELLED.into());
        }
        let chunk = match rx.recv_timeout(RECV_TIMEOUT) {
            Ok(c) => c,
            Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => break,
        };
        pending.extend_from_slice(&chunk);
        let usable = pending.len() / frame_bytes * frame_bytes;
        if usable == 0 {
            continue;
        }
        buf.clear();
        buf.extend(
            pending[..usable]
                .chunks_exact(4)
                .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])),
        );
        pending.drain(..usable);
        if let Err(e) = writer.emit(&mut buf) {
            drop(rx);
            decoder.handle.cancel();
            drop(writer.sink.take());
            let msg = write_err(e, &mut encoder);
            let _ = decoder.wait();
            return Err(msg);
        }
        progress(writer.pos);
        if let Some(f) = p.pace {
            pace_wait(t0, writer.pos, p.rate, f, cancel);
        }
    }
    drop(rx);

    let status = decoder
        .wait()
        .map_err(|e| format!("Cannot decode {}: {e}", label(&p.source)))?;
    if cancel.load(Ordering::SeqCst) {
        abort(&mut encoder, &mut writer);
        return Err(CANCELLED.into());
    }
    if !status.success() {
        abort(&mut encoder, &mut writer);
        return Err(with_reason(
            format!("Cannot decode {}", label(&p.source)),
            &decoder.last(),
        ));
    }

    // 3. Finish the output.
    let frames = writer.pos;
    let clipped = writer.clipped;
    match writer.sink.take() {
        Some(Sink::Wav(w)) => w
            .finish()
            .map_err(|e| format!("Cannot write {part}: {e}"))?,
        Some(Sink::Pipe(stdin)) => {
            drop(stdin);
            let mut enc = encoder.take().expect("encoder");
            let status = enc.wait().map_err(|e| format!("Cannot encode: {e}"))?;
            if cancel.load(Ordering::SeqCst) {
                return Err(CANCELLED.into());
            }
            if !status.success() {
                return Err(with_reason("Cannot encode".into(), &enc.last()));
            }
        }
        None => {}
    }
    Ok(RunStats { frames, clipped })
}

/// Stops the encoder (if any) and drops the output.
fn abort(encoder: &mut Option<Child>, writer: &mut Writer) {
    drop(writer.sink.take());
    if let Some(mut e) = encoder.take() {
        e.handle.cancel();
        let _ = e.wait();
    }
}

#[cfg(test)]
#[path = "export_audio_tests.rs"]
mod tests;
