#![warn(missing_docs)]

//! Library for identifying songs using the Shazam API.

use std::path::Path;

use base64::{Engine as _, engine::general_purpose};
use miette::Diagnostic;

/// Identifies a song by extracting a sample from the middle of the audio file
/// and sending it to the Shazam API.
///
/// # Errors
///
/// Returns [`ShazamError`] if the audio file cannot be read, the sample cannot
/// be extracted, or the API request fails.
pub async fn identify_song(
    path: &Path,
    api_key: &str,
    sample_duration_ms: u64,
) -> Result<String, ShazamError> {
    let raw_audio = extract_audio_sample(path, sample_duration_ms)?;
    let encoded = encode_to_base64(&raw_audio);
    send_to_shazam(&encoded, api_key, sample_duration_ms).await
}

async fn send_to_shazam(
    encoded: &str,
    api_key: &str,
    sample_ms: u64,
) -> Result<String, ShazamError> {
    let client = reqwest::Client::new();
    let response = client
        .post("https://shazam.p.rapidapi.com/songs/v3/detect")
        .query(&[
            ("timezone", "UTC"),
            ("locale", "en-US"),
            ("samplems", sample_ms.to_string().as_str()),
        ])
        .header("X-RapidAPI-Host", "shazam.p.rapidapi.com")
        .header("X-RapidAPI-Key", api_key)
        .header("Content-Type", "text/plain")
        .body(encoded.to_owned())
        .send()
        .await?;

    let body = response.text().await?;
    Ok(body)
}

fn extract_audio_sample(path: &Path, sample_duration_ms: u64) -> Result<Vec<u8>, ShazamError> {
    ffmpeg_next::init().map_err(|e| ShazamError::FfmpegInit(e.to_string()))?;

    let mut ictx = ffmpeg_next::format::input(path).map_err(|e| ShazamError::OpenFile {
        path: path.display().to_string(),
        message: e.to_string(),
    })?;

    let raw_duration = ictx.duration();
    if raw_duration <= 0 {
        return Err(ShazamError::InvalidDuration);
    }

    let total_ms = u64::try_from(raw_duration).unwrap_or(u64::MAX) / 1_000;

    let (stream_index, decoder_context) = {
        let audio_stream = ictx
            .streams()
            .best(ffmpeg_next::media::Type::Audio)
            .ok_or(ShazamError::NoAudioStream)?;
        let ctx =
            ffmpeg_next::codec::context::Context::from_parameters(audio_stream.parameters())
                .map_err(|e| ShazamError::Codec(e.to_string()))?;
        (audio_stream.index(), ctx)
    };

    let mut decoder = decoder_context
        .decoder()
        .audio()
        .map_err(|e| ShazamError::Codec(e.to_string()))?;

    let (start_ms, end_ms) = compute_sample_window(total_ms, sample_duration_ms);

    let seek_us = i64::try_from(start_ms.saturating_mul(1_000)).unwrap_or(i64::MAX);
    ictx.seek(seek_us, ..seek_us)
        .map_err(|e| ShazamError::Seek(e.to_string()))?;

    let actual_sample_ms = end_ms - start_ms;
    let target_bytes =
        usize::try_from(44_100_u64 * 2 * actual_sample_ms / 1_000).unwrap_or(usize::MAX);

    let target_format =
        ffmpeg_next::format::Sample::I16(ffmpeg_next::format::sample::Type::Packed);
    let target_layout = ffmpeg_next::util::channel_layout::ChannelLayout::MONO;
    let target_rate = 44_100_u32;

    let mut raw_bytes: Vec<u8> = Vec::with_capacity(target_bytes);
    let mut resampler: Option<ffmpeg_next::software::resampling::Context> = None;
    let mut audio_frame = ffmpeg_next::frame::Audio::empty();

    'packet_loop: for (stream, packet) in ictx.packets() {
        if stream.index() != stream_index {
            continue;
        }

        if let Some(pts) = packet.pts() {
            let tb = stream.time_base();
            let pts_ms = pts
                .saturating_mul(1_000)
                .saturating_mul(i64::from(tb.numerator()))
                / i64::from(tb.denominator());
            if pts_ms > i64::try_from(end_ms).unwrap_or(i64::MAX) {
                break;
            }
        }

        if decoder.send_packet(&packet).is_err() {
            continue;
        }

        while decoder.receive_frame(&mut audio_frame).is_ok() {
            if resampler.is_none() {
                resampler = Some(
                    ffmpeg_next::software::resampling::Context::get(
                        audio_frame.format(),
                        audio_frame.channel_layout(),
                        audio_frame.rate(),
                        target_format,
                        target_layout,
                        target_rate,
                    )
                    .map_err(|e| ShazamError::Resample(e.to_string()))?,
                );
            }

            let r = resampler.as_mut().expect("initialized above");
            let mut output_frame = ffmpeg_next::frame::Audio::empty();
            if r.run(&audio_frame, &mut output_frame).is_ok() && output_frame.samples() > 0 {
                raw_bytes.extend_from_slice(output_frame.data(0));
            }

            if raw_bytes.len() >= target_bytes {
                break 'packet_loop;
            }
        }
    }

    // Flush decoder
    decoder.send_eof().ok();
    while decoder.receive_frame(&mut audio_frame).is_ok() {
        if let Some(r) = resampler.as_mut() {
            let mut output_frame = ffmpeg_next::frame::Audio::empty();
            if r.run(&audio_frame, &mut output_frame).is_ok() && output_frame.samples() > 0 {
                raw_bytes.extend_from_slice(output_frame.data(0));
            }
        }
    }

    // Flush resampler
    if let Some(mut r) = resampler {
        let mut output_frame = ffmpeg_next::frame::Audio::empty();
        if r.flush(&mut output_frame).is_ok() && output_frame.samples() > 0 {
            raw_bytes.extend_from_slice(output_frame.data(0));
        }
    }

    raw_bytes.truncate(target_bytes);

    if raw_bytes.is_empty() {
        return Err(ShazamError::NoAudio);
    }

    Ok(raw_bytes)
}

const fn compute_sample_window(total_duration_ms: u64, sample_duration_ms: u64) -> (u64, u64) {
    let actual_sample = if sample_duration_ms <= total_duration_ms {
        sample_duration_ms
    } else {
        total_duration_ms
    };
    let half_total = total_duration_ms / 2;
    let half_sample = actual_sample / 2;
    let start = half_total.saturating_sub(half_sample);
    (start, start + actual_sample)
}

fn encode_to_base64(data: &[u8]) -> String {
    general_purpose::STANDARD.encode(data)
}

/// Error variants for Shazam CLI operations.
#[derive(Debug, Diagnostic, thiserror::Error)]
pub enum ShazamError {
    /// Failed to initialize `FFmpeg`.
    #[error("Failed to initialize FFmpeg: {0}")]
    FfmpegInit(String),

    /// Failed to open the audio file.
    #[error("Failed to open file '{path}': {message}")]
    OpenFile {
        /// The file path that could not be opened.
        path: String,
        /// The underlying error message.
        message: String,
    },

    /// No audio stream was found in the file.
    #[error("No audio stream found in the file")]
    NoAudioStream,

    /// The file has an invalid or missing duration.
    #[error("File has an invalid or missing duration")]
    InvalidDuration,

    /// Failed to create or configure a codec context.
    #[error("Codec error: {0}")]
    Codec(String),

    /// Failed to seek to the sample start position.
    #[error("Failed to seek in file: {0}")]
    Seek(String),

    /// Failed to decode audio frames.
    #[error("Failed to decode audio: {0}")]
    Decode(String),

    /// Failed to resample audio to 44100 Hz mono S16.
    #[error("Failed to resample audio: {0}")]
    Resample(String),

    /// No audio samples could be collected from the file.
    #[error("No audio samples could be collected from the file")]
    NoAudio,

    /// An HTTP error occurred while communicating with the Shazam API.
    #[error("HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_sample_window_normal() {
        let (start, end) = compute_sample_window(60_000, 4_000);
        assert_eq!(start, 28_000);
        assert_eq!(end, 32_000);
    }

    #[test]
    fn test_compute_sample_window_short_file() {
        let (start, end) = compute_sample_window(2_000, 4_000);
        assert_eq!(start, 0);
        assert_eq!(end, 2_000);
    }

    #[test]
    fn test_compute_sample_window_exact_fit() {
        let (start, end) = compute_sample_window(4_000, 4_000);
        assert_eq!(start, 0);
        assert_eq!(end, 4_000);
    }

    #[test]
    fn test_compute_sample_window_odd_duration() {
        let (start, end) = compute_sample_window(10_001, 4_000);
        assert_eq!(end - start, 4_000);
    }

    #[test]
    fn test_encode_to_base64() {
        let encoded = encode_to_base64(b"hello");
        assert_eq!(encoded, "aGVsbG8=");
    }

    #[test]
    fn test_encode_to_base64_empty() {
        let encoded = encode_to_base64(b"");
        assert_eq!(encoded, "");
    }
}
