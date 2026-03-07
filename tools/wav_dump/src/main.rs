use anyhow::{Context, Result, bail};
use hound::{SampleFormat, WavReader};
use std::env;
use std::path::Path;

fn main() -> Result<()> {
    let path = env::args().nth(1).context("usage: wav_dump <path>")?;
    dump_wav(Path::new(&path))
}

fn dump_wav(path: &Path) -> Result<()> {
    let mut reader =
        WavReader::open(path).with_context(|| format!("failed to open {}", path.display()))?;
    let spec = reader.spec();

    let mut peak = 0.0_f32;
    let mut sum_sq = 0.0_f64;
    let mut count = 0_u64;

    match (spec.sample_format, spec.bits_per_sample) {
        (SampleFormat::Float, 32) => {
            for sample in reader.samples::<f32>() {
                let sample = sample.context("failed to read float sample")?;
                peak = peak.max(sample.abs());
                sum_sq += (sample as f64) * (sample as f64);
                count += 1;
            }
        }
        (SampleFormat::Int, 16) => {
            for sample in reader.samples::<i16>() {
                let sample =
                    sample.context("failed to read int16 sample")? as f32 / i16::MAX as f32;
                peak = peak.max(sample.abs());
                sum_sq += (sample as f64) * (sample as f64);
                count += 1;
            }
        }
        (SampleFormat::Int, 24 | 32) => {
            for sample in reader.samples::<i32>() {
                let sample =
                    sample.context("failed to read int32 sample")? as f32 / i32::MAX as f32;
                peak = peak.max(sample.abs());
                sum_sq += (sample as f64) * (sample as f64);
                count += 1;
            }
        }
        _ => bail!(
            "unsupported wav format: {:?}/{}",
            spec.sample_format,
            spec.bits_per_sample
        ),
    }

    let rms = if count == 0 {
        0.0
    } else {
        (sum_sq / count as f64).sqrt() as f32
    };
    let duration_s = if spec.sample_rate == 0 || spec.channels == 0 {
        0.0
    } else {
        count as f64 / (spec.sample_rate as f64 * spec.channels as f64)
    };

    println!("path: {}", path.display());
    println!("channels: {}", spec.channels);
    println!("sample_rate: {}", spec.sample_rate);
    println!("bits_per_sample: {}", spec.bits_per_sample);
    println!("sample_format: {:?}", spec.sample_format);
    println!("duration_s: {:.3}", duration_s);
    println!("peak: {:.6}", peak);
    println!("rms: {:.6}", rms);

    Ok(())
}
