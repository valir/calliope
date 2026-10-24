//! Float32 stereo WAV writer, shared by the audio capture and the export.

use std::fs::File;
use std::io::{BufWriter, Seek, SeekFrom, Write};
use std::path::Path;

/// Float32 stereo WAV: "RIFF" <36+n> "WAVE" "fmt " 16 {format 3 (IEEE float), 2 ch, rate,
/// rate*8 B/s, align 8, 32 bit} "data" <n>; 44-byte header, sizes patched on `finish`.
pub(crate) struct WavWriter {
    out: BufWriter<File>,
    bytes: u32,
}

impl WavWriter {
    pub(crate) fn create(path: &Path, rate: u32) -> std::io::Result<Self> {
        Self::from_file(File::create(path)?, rate)
    }

    pub(crate) fn from_file(file: File, rate: u32) -> std::io::Result<Self> {
        let mut out = BufWriter::new(file);
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

    pub(crate) fn write(&mut self, samples: &[f32]) -> std::io::Result<()> {
        for s in samples {
            self.out.write_all(&s.to_le_bytes())?;
        }
        self.bytes = self.bytes.saturating_add(samples.len() as u32 * 4);
        Ok(())
    }

    pub(crate) fn finish(mut self) -> std::io::Result<()> {
        self.out.flush()?;
        let f = self.out.get_mut();
        f.seek(SeekFrom::Start(4))?;
        f.write_all(&self.bytes.saturating_add(36).to_le_bytes())?;
        f.seek(SeekFrom::Start(40))?;
        f.write_all(&self.bytes.to_le_bytes())?;
        f.sync_all()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn three_frames_make_a_44_plus_24_byte_float_wav() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.wav");
        let samples = [0.5f32, -0.5, 1.0, -1.0, 0.25, 0.0];
        let mut w = WavWriter::create(&p, 44100).unwrap();
        w.write(&samples).unwrap();
        w.finish().unwrap();
        let b = std::fs::read(&p).unwrap();
        assert_eq!(b.len(), 44 + 24);
        let u32_at = |i: usize| u32::from_le_bytes(b[i..i + 4].try_into().unwrap());
        let u16_at = |i: usize| u16::from_le_bytes(b[i..i + 2].try_into().unwrap());
        assert_eq!(&b[0..4], b"RIFF");
        assert_eq!(u32_at(4), 60);
        assert_eq!(&b[8..16], b"WAVEfmt ");
        assert_eq!(u32_at(16), 16);
        assert_eq!(u16_at(20), 3);
        assert_eq!(u16_at(22), 2);
        assert_eq!(u32_at(24), 44100);
        assert_eq!(u32_at(28), 352800);
        assert_eq!(u16_at(32), 8);
        assert_eq!(u16_at(34), 32);
        assert_eq!(&b[36..40], b"data");
        assert_eq!(u32_at(40), 24);
        let got: Vec<f32> = b[44..]
            .chunks(4)
            .map(|c| f32::from_le_bytes(c.try_into().unwrap()))
            .collect();
        assert_eq!(got, samples);
    }

    #[test]
    fn from_file_writes_into_the_given_file() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("b.wav");
        let f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&p)
            .unwrap();
        WavWriter::from_file(f, 48000).unwrap().finish().unwrap();
        let b = std::fs::read(&p).unwrap();
        assert_eq!(b.len(), 44);
        assert_eq!(&b[40..44], &0u32.to_le_bytes());
    }
}
