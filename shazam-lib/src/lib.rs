#![warn(missing_docs)]

//! Library for identifying songs using the Shazam API.

mod response;

use std::path::Path;

use base64::{Engine as _, engine::general_purpose};
use miette::Diagnostic;
use tracing::{debug, trace};

/// Context about what was extracted from a file, used to validate the API response.
#[derive(Debug, Clone, Copy)]
pub struct ExtractionContext {
    /// Total duration of the source file in milliseconds.
    pub file_duration_ms: u64,
    /// Start of the extracted sample within the file in milliseconds.
    pub sample_start_ms: u64,
}

/// Identifies a song by extracting a sample from the middle of the audio file
/// and sending it to the Shazam API.
///
/// # Errors
///
/// Returns [`ShazamError`] if the audio file cannot be read, the sample cannot
/// be extracted, or the API request fails.
#[tracing::instrument(skip(api_key, path, sample_duration_ms))]
pub async fn identify_song(
    path: &Path,
    api_key: &str,
    sample_duration_ms: u64,
) -> Result<String, ShazamError> {
    let (raw_audio, context) = extract_audio_sample(path, sample_duration_ms)?;
    let encoded = encode_to_base64(&raw_audio);
    send_to_shazam(&encoded, api_key, sample_duration_ms, &context).await
}

/// Parses a raw Shazam API JSON response and formats it for human-readable display.
///
/// `context` provides information about what was extracted from the source file, enabling
/// the formatter to flag mismatches between the file and the identified song.
///
/// # Errors
///
/// Returns [`ShazamError::ParseResponse`] if the JSON cannot be parsed.
pub fn format_shazam_response(
    json: &str,
    context: Option<&ExtractionContext>,
) -> Result<String, ShazamError> {
    let parsed: response::ShazamResponse =
        serde_json::from_str(json).map_err(|e| ShazamError::ParseResponse(e.to_string()))?;
    Ok(parsed.format_display(context))
}

#[tracing::instrument(skip(encoded, api_key), fields(encoded_bytes = encoded.len()))]
async fn send_to_shazam(
    encoded: &str,
    api_key: &str,
    sample_ms: u64,
    context: &ExtractionContext,
) -> Result<String, ShazamError> {
    debug!(
        encoded_bytes = encoded.len(),
        sample_ms, "sending request to Shazam API"
    );
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

    let status = response.status();
    debug!(%status, "received response from Shazam API");
    let body = response.text().await?;
    format_shazam_response(&body, Some(context))
}

#[tracing::instrument(skip(path, sample_duration_ms))]
fn extract_audio_sample(
    path: &Path,
    sample_duration_ms: u64,
) -> Result<(Vec<u8>, ExtractionContext), ShazamError> {
    let (mut ictx, stream_index, decoder, start_ms, end_ms, total_ms) =
        open_input(path, sample_duration_ms)?;

    let seek_us = i64::try_from(start_ms.saturating_mul(1_000)).unwrap_or(i64::MAX);
    debug!(seek_us, "seeking");
    ictx.seek(seek_us, ..seek_us)
        .map_err(|e| ShazamError::Seek(e.to_string()))?;

    let actual_sample_ms = end_ms - start_ms;
    let target_bytes =
        usize::try_from(44_100_u64 * 2 * actual_sample_ms / 1_000).unwrap_or(usize::MAX);
    debug!(actual_sample_ms, target_bytes, "collecting audio");

    let bytes = collect_audio_bytes(ictx, stream_index, decoder, end_ms, target_bytes)?;
    let context = ExtractionContext {
        file_duration_ms: total_ms,
        sample_start_ms: start_ms,
    };
    Ok((bytes, context))
}

#[tracing::instrument(skip(path, sample_duration_ms))]
fn open_input(
    path: &Path,
    sample_duration_ms: u64,
) -> Result<
    (
        ffmpeg_next::format::context::Input,
        usize,
        ffmpeg_next::codec::decoder::Audio,
        u64,
        u64,
        u64,
    ),
    ShazamError,
> {
    ffmpeg_next::init().map_err(|e| ShazamError::FfmpegInit(e.to_string()))?;

    let ictx = ffmpeg_next::format::input(path).map_err(|e| ShazamError::OpenFile {
        path: path.display().to_string(),
        message: e.to_string(),
    })?;

    let raw_duration = ictx.duration();
    debug!(raw_duration, "opened file");
    if raw_duration <= 0 {
        return Err(ShazamError::InvalidDuration);
    }

    let total_ms = u64::try_from(raw_duration).unwrap_or(u64::MAX) / 1_000;
    debug!(total_ms, "file duration");

    let (stream_index, decoder_context) = {
        let audio_stream = ictx
            .streams()
            .best(ffmpeg_next::media::Type::Audio)
            .ok_or(ShazamError::NoAudioStream)?;
        debug!(
            stream_index = audio_stream.index(),
            time_base_num = audio_stream.time_base().numerator(),
            time_base_den = audio_stream.time_base().denominator(),
            "found audio stream"
        );
        let ctx = ffmpeg_next::codec::context::Context::from_parameters(audio_stream.parameters())
            .map_err(|e| ShazamError::Codec(e.to_string()))?;
        (audio_stream.index(), ctx)
    };

    let decoder = decoder_context
        .decoder()
        .audio()
        .map_err(|e| ShazamError::Codec(e.to_string()))?;
    debug!(
        format = ?decoder.format(),
        channels = decoder.channels(),
        rate = decoder.rate(),
        channel_layout = ?decoder.channel_layout(),
        "created audio decoder"
    );

    let (start_ms, end_ms) = compute_sample_window(total_ms, sample_duration_ms);
    debug!(start_ms, end_ms, "computed sample window");

    Ok((ictx, stream_index, decoder, start_ms, end_ms, total_ms))
}

#[expect(clippy::too_many_lines)]
#[tracing::instrument(skip(ictx, decoder))]
fn collect_audio_bytes(
    mut ictx: ffmpeg_next::format::context::Input,
    stream_index: usize,
    mut decoder: ffmpeg_next::codec::decoder::Audio,
    end_ms: u64,
    target_bytes: usize,
) -> Result<Vec<u8>, ShazamError> {
    let target_format = ffmpeg_next::format::Sample::I16(ffmpeg_next::format::sample::Type::Packed);
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
            trace!(pts, pts_ms, packet_size = packet.size(), "audio packet");
            if pts_ms > i64::try_from(end_ms).unwrap_or(i64::MAX) {
                debug!(pts_ms, end_ms, "reached end of sample window, stopping");
                break;
            }
        }

        if decoder.send_packet(&packet).is_err() {
            debug!("send_packet failed, skipping");
            continue;
        }

        while decoder.receive_frame(&mut audio_frame).is_ok() {
            normalise_channel_layout(&mut audio_frame);
            if resampler.is_none() {
                debug!(
                    src_format = ?audio_frame.format(),
                    src_channels = audio_frame.channels(),
                    src_rate = audio_frame.rate(),
                    src_channel_layout = ?audio_frame.channel_layout(),
                    "creating resampler"
                );
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
                debug!("resampler created");
            }

            let r = resampler.as_mut().expect("initialized above");
            let mut output_frame = ffmpeg_next::frame::Audio::empty();
            match r.run(&audio_frame, &mut output_frame) {
                Ok(_) => {
                    let out_samples = output_frame.samples();
                    trace!(
                        input_samples = audio_frame.samples(),
                        output_samples = out_samples,
                        "resampled frame"
                    );
                    if out_samples > 0 {
                        raw_bytes.extend_from_slice(output_frame.data(0));
                    }
                }
                Err(e) => debug!(error = %e, "resampler run failed"),
            }

            if raw_bytes.len() >= target_bytes {
                debug!(
                    collected = raw_bytes.len(),
                    "collected enough audio, stopping"
                );
                break 'packet_loop;
            }
        }
    }

    debug!(collected = raw_bytes.len(), "packet loop done, flushing");

    decoder.send_eof().ok();
    while decoder.receive_frame(&mut audio_frame).is_ok() {
        normalise_channel_layout(&mut audio_frame);
        if let Some(r) = resampler.as_mut() {
            let mut output_frame = ffmpeg_next::frame::Audio::empty();
            if r.run(&audio_frame, &mut output_frame).is_ok() && output_frame.samples() > 0 {
                raw_bytes.extend_from_slice(output_frame.data(0));
            }
        }
    }

    if let Some(mut r) = resampler {
        let mut output_frame = ffmpeg_next::frame::Audio::empty();
        if r.flush(&mut output_frame).is_ok() && output_frame.samples() > 0 {
            debug!(samples = output_frame.samples(), "flushed resampler");
            raw_bytes.extend_from_slice(output_frame.data(0));
        }
    }

    raw_bytes.truncate(target_bytes);
    debug!(
        final_bytes = raw_bytes.len(),
        target_bytes, "audio extraction complete"
    );

    if raw_bytes.is_empty() {
        return Err(ShazamError::NoAudio);
    }

    Ok(raw_bytes)
}

// WAV files without a channel-layout chunk leave ch_layout as AV_CHANNEL_ORDER_UNSPEC.
// swresample promotes that to native stereo on init, then rejects every frame as
// "Input changed" because the frame still carries UNSPEC. Normalise to a concrete
// default so the resampler always sees a matching layout.
fn normalise_channel_layout(frame: &mut ffmpeg_next::frame::Audio) {
    if frame.channel_layout().is_empty() {
        frame.set_channel_layout(ffmpeg_next::util::channel_layout::ChannelLayout::default(
            i32::from(frame.channels()),
        ));
    }
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

    /// The API response JSON could not be parsed.
    #[error("Failed to parse API response: {0}")]
    ParseResponse(String),
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

    #[test]
    fn test_format_response_full_match() {
        let json = r#"{
            "results": {"matches": [{"id": "1", "type": "shazam-songs"}]},
            "resources": {
                "shazam-songs": {"1": {
                    "attributes": {
                        "title": "Test Song",
                        "artist": "Test Artist",
                        "explicit": false,
                        "webUrl": "https://www.shazam.com/track/1/test-song?co=US"
                    },
                    "meta": {"matchOffset": 90.0, "duration": 183.0},
                    "relationships": {
                        "genres": {"data": [{"id": "10", "type": "genres"}]}
                    }
                }},
                "genres": {
                    "10": {"id": "10", "attributes": {"name": "Rock"}, "type": "genres"}
                }
            }
        }"#;
        let output = format_shazam_response(json, None).unwrap();
        assert!(output.contains("Test Song"));
        assert!(output.contains("Test Artist"));
        assert!(output.contains("Rock"));
        assert!(output.contains("3:03"));
        assert!(output.contains("1:30"));
        assert!(output.contains("https://www.shazam.com/track/1/test-song"));
        assert!(!output.contains("?co=US"));
    }

    #[test]
    fn test_format_response_multiple_genres() {
        let json = r#"{
            "results": {"matches": [{"id": "1", "type": "shazam-songs"}]},
            "resources": {
                "shazam-songs": {"1": {
                    "attributes": {"title": "T", "artist": "A", "explicit": false},
                    "meta": {"matchOffset": 60.0, "duration": 120.0},
                    "relationships": {
                        "genres": {"data": [
                            {"id": "1", "type": "genres"},
                            {"id": "2", "type": "genres"},
                            {"id": "3", "type": "genres"}
                        ]}
                    }
                }},
                "genres": {
                    "1": {"id": "1", "attributes": {"name": "Pop"}, "type": "genres"},
                    "2": {"id": "2", "attributes": {"name": "Music"}, "type": "genres"},
                    "3": {"id": "3", "attributes": {"name": "R&B/Soul"}, "type": "genres"}
                }
            }
        }"#;
        let output = format_shazam_response(json, None).unwrap();
        assert!(output.contains("Pop, Music, R&B/Soul"));
    }

    #[test]
    fn test_format_response_no_genres() {
        let json = r#"{
            "results": {"matches": [{"id": "2", "type": "shazam-songs"}]},
            "resources": {"shazam-songs": {"2": {
                "attributes": {
                    "title": "Another Song",
                    "artist": "Another Artist",
                    "explicit": true,
                    "webUrl": "https://www.shazam.com/track/2/another"
                },
                "meta": {"matchOffset": 120.0, "duration": 240.0}
            }}}
        }"#;
        let output = format_shazam_response(json, None).unwrap();
        assert!(output.contains("Another Song"));
        assert!(output.contains("[Explicit]"));
        assert!(output.contains("4:00"));
        assert!(!output.contains("Genres:"));
    }

    #[test]
    fn test_format_response_context_matching() {
        let json = r#"{
            "results": {"matches": [{"id": "1", "type": "shazam-songs"}]},
            "resources": {"shazam-songs": {"1": {
                "attributes": {"title": "T", "artist": "A", "explicit": false},
                "meta": {"matchOffset": 90.0, "duration": 183.0}
            }}}
        }"#;
        let ctx = ExtractionContext { file_duration_ms: 184_000, sample_start_ms: 90_000 };
        let output = format_shazam_response(json, Some(&ctx)).unwrap();
        assert!(output.contains("3:03"));
        assert!(!output.contains("(song) vs"));
        assert!(output.contains("1:30 into song"));
        assert!(!output.contains("in song vs"));
    }

    #[test]
    fn test_format_response_context_mismatch() {
        let json = r#"{
            "results": {"matches": [{"id": "1", "type": "shazam-songs"}]},
            "resources": {"shazam-songs": {"1": {
                "attributes": {"title": "T", "artist": "A", "explicit": false},
                "meta": {"matchOffset": 90.0, "duration": 183.0}
            }}}
        }"#;
        let ctx = ExtractionContext { file_duration_ms: 250_000, sample_start_ms: 60_000 };
        let output = format_shazam_response(json, Some(&ctx)).unwrap();
        assert!(output.contains("(song) vs"));
        assert!(output.contains("in song vs"));
    }

    #[test]
    fn test_format_response_no_matches() {
        let json = r#"{"results": {"matches": []}, "resources": {}}"#;
        let output = format_shazam_response(json, None).unwrap();
        assert_eq!(output, "No match found.");
    }

    #[test]
    fn test_format_response_invalid_json() {
        let result = format_shazam_response("not json at all", None);
        assert!(matches!(result, Err(ShazamError::ParseResponse(_))));
    }
}
