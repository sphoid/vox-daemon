//! Recording session management — captures audio, transcribes, and saves results.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use vox_capture::{AudioChunk, AudioSource, StreamRole};
use vox_core::config::AppConfig;
use vox_core::session::{
    AudioRole, AudioSourceInfo, ConfigSnapshot, Session, SpeakerMapping, SpeakerSource,
    TranscriptSegment,
};
use vox_storage::{JsonFileStore, SessionStore};
use vox_transcribe::{AudioSourceRole, Transcriber, TranscriptionRequest};

/// Run a recording session.
///
/// Captures audio from `PipeWire` (or a mock source), transcribes it with Whisper
/// (or a stub transcriber), and saves the resulting session to disk.
///
/// The `stop_flag` allows external callers (e.g., the daemon) to signal the
/// recording to stop. When `None`, the recording runs until Ctrl+C.
pub async fn record_session(
    config: AppConfig,
    mic_override: Option<String>,
    app_override: Option<String>,
    stop_flag: Option<Arc<AtomicBool>>,
) -> Result<()> {
    let (mic_chunks, app_chunks) =
        capture_audio(&config, mic_override, app_override, stop_flag).await?;

    let mic_sample_count: usize = mic_chunks.iter().map(|c| c.samples.len()).sum();
    let app_sample_count: usize = app_chunks.iter().map(|c| c.samples.len()).sum();
    tracing::info!(
        "captured {} mic samples, {} app samples",
        mic_sample_count,
        app_sample_count
    );

    if mic_sample_count == 0 && app_sample_count == 0 {
        tracing::warn!("no audio captured, skipping transcription");
        return Ok(());
    }

    let mut session = build_session(&config, mic_sample_count, app_sample_count);

    transcribe_audio(&mut session, mic_chunks, app_chunks)?;

    // Save session
    let store =
        JsonFileStore::new(&config.storage.data_dir).context("failed to open session store")?;
    store.save(&session).context("failed to save session")?;
    tracing::info!("session saved: {}", session.id);

    // Auto-summarize if configured
    if config.summarization.auto_summarize && !session.transcript.is_empty() {
        match auto_summarize(&config, &mut session).await {
            Ok(()) => {
                store
                    .save(&session)
                    .context("failed to save session after summarization")?;
                tracing::info!("auto-summarization complete for session {}", session.id);
            }
            Err(e) => {
                tracing::warn!("auto-summarization failed (session saved without summary): {e}");
            }
        }
    }

    Ok(())
}

/// Run auto-summarization on the session transcript.
async fn auto_summarize(config: &AppConfig, session: &mut Session) -> Result<()> {
    tracing::info!("running auto-summarization...");
    let summarizer = vox_summarize::create_summarizer(&config.summarization)
        .map_err(|e| anyhow::anyhow!("failed to create summarizer: {e}"))?;
    let summary = summarizer
        .summarize(&session.transcript)
        .await
        .map_err(|e| anyhow::anyhow!("summarization failed: {e}"))?;
    session.summary = Some(summary);
    Ok(())
}

/// Capture audio from the source until Ctrl+C or the stop flag is set.
async fn capture_audio(
    config: &AppConfig,
    mic_override: Option<String>,
    app_override: Option<String>,
    stop_flag: Option<Arc<AtomicBool>>,
) -> Result<(Vec<AudioChunk>, Vec<AudioChunk>)> {
    #[cfg(feature = "pw")]
    let mut source = {
        let targets = select_capture_targets(config, mic_override, app_override)?;
        if targets.is_empty() {
            anyhow::bail!("no audio sources found to capture");
        }
        vox_capture::pw::PipeWireSource::new(targets)
            .map_err(|e| anyhow::anyhow!("failed to create PipeWire source: {e}"))?
    };
    #[cfg(not(feature = "pw"))]
    let mut source = {
        let _ = (config, mic_override, app_override);
        vox_capture::mock::MockAudioSource::new()
    };

    source
        .start()
        .map_err(|e| anyhow::anyhow!("failed to start capture: {e}"))?;

    let rx = source.stream_receiver().clone();
    tracing::info!("recording started — press Ctrl+C to stop");

    let mut mic_chunks: Vec<AudioChunk> = Vec::new();
    let mut app_chunks: Vec<AudioChunk> = Vec::new();

    let shutdown = tokio::signal::ctrl_c();
    tokio::pin!(shutdown);

    loop {
        // Check external stop flag
        if let Some(ref flag) = stop_flag {
            if flag.load(Ordering::Relaxed) {
                tracing::info!("stopping recording (external stop signal)");
                break;
            }
        }

        tokio::select! {
            _ = &mut shutdown => {
                tracing::info!("stopping recording");
                break;
            }
            () = tokio::task::yield_now() => {
                while let Ok(chunk) = rx.try_recv() {
                    match chunk.role {
                        StreamRole::Microphone => mic_chunks.push(chunk),
                        StreamRole::Application => app_chunks.push(chunk),
                    }
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }
    }

    source
        .stop()
        .map_err(|e| anyhow::anyhow!("failed to stop capture: {e}"))?;

    Ok((mic_chunks, app_chunks))
}

/// Build a `Session` with metadata from the config and audio sizes.
fn build_session(config: &AppConfig, mic_samples: usize, app_samples: usize) -> Session {
    let audio_sources = vec![
        AudioSourceInfo {
            name: "microphone".to_owned(),
            pipewire_node_id: 0,
            role: AudioRole::Microphone,
        },
        AudioSourceInfo {
            name: "application".to_owned(),
            pipewire_node_id: 0,
            role: AudioRole::Application,
        },
    ];

    let config_snapshot = ConfigSnapshot {
        model: config.transcription.model.clone(),
        language: config.transcription.language.clone(),
        gpu_backend: config.transcription.gpu_backend.clone(),
    };

    let mut session = Session::new(audio_sources, config_snapshot);
    let total_samples = mic_samples.max(app_samples);
    session.duration_seconds = (total_samples as u64) / 16_000;

    session.speakers = vec![
        SpeakerMapping {
            id: "speaker_0".to_owned(),
            friendly_name: "You".to_owned(),
            source: SpeakerSource::Microphone,
        },
        SpeakerMapping {
            id: "speaker_1".to_owned(),
            friendly_name: "Remote".to_owned(),
            source: SpeakerSource::Remote,
        },
    ];

    session
}

/// Select PipeWire node targets for capture based on config and overrides.
///
/// When a source is `"auto"` (the default), auto-detection picks the first
/// suitable node: mic → first `Audio/Source` node, app → first
/// `Stream/Input/Audio` node.  When a numeric node ID or node name is given
/// explicitly, that node is used directly.
#[cfg(feature = "pw")]
fn select_capture_targets(
    config: &AppConfig,
    mic_override: Option<String>,
    app_override: Option<String>,
) -> Result<Vec<(u32, StreamRole)>> {
    use vox_capture::StreamFilter;

    let mic_setting = mic_override
        .as_deref()
        .unwrap_or(&config.audio.mic_source);
    let app_setting = app_override
        .as_deref()
        .unwrap_or(&config.audio.app_source);

    let all_streams = vox_capture::PipeWireSource::enumerate_streams(&StreamFilter::default())
        .map_err(|e| anyhow::anyhow!("failed to enumerate PipeWire sources: {e}"))?;

    tracing::debug!("discovered {} PipeWire nodes", all_streams.len());
    for s in &all_streams {
        tracing::debug!(
            node_id = s.node_id,
            name = %s.name,
            class = ?s.media_class,
            "  node"
        );
    }

    let mut targets: Vec<(u32, StreamRole)> = Vec::new();

    // Resolve mic target.
    if let Some(node_id) = resolve_source(mic_setting, &all_streams, true) {
        tracing::info!(node_id, setting = mic_setting, "selected mic source");
        targets.push((node_id, StreamRole::Microphone));
    } else {
        tracing::warn!(setting = mic_setting, "no mic source found");
    }

    // Resolve app audio target.
    if let Some(node_id) = resolve_source(app_setting, &all_streams, false) {
        tracing::info!(node_id, setting = app_setting, "selected app source");
        targets.push((node_id, StreamRole::Application));
    } else {
        tracing::warn!(setting = app_setting, "no app audio source found");
    }

    Ok(targets)
}

/// Resolve a source setting string to a PipeWire node ID.
///
/// `setting` may be:
/// - `"auto"` — pick the first node matching the role heuristic
/// - a numeric string (e.g. `"42"`) — use that node ID directly
/// - a node name — match against `StreamInfo::name`
///
/// `is_mic` selects whether we look for source nodes (mic) or sink/stream
/// nodes (app audio) when auto-detecting.
#[cfg(feature = "pw")]
fn resolve_source(
    setting: &str,
    streams: &[vox_capture::StreamInfo],
    is_mic: bool,
) -> Option<u32> {
    if setting == "auto" || setting.is_empty() {
        // Auto-detect: pick the first node matching the role.
        if is_mic {
            streams.iter().find(|s| s.is_source()).map(|s| s.node_id)
        } else {
            // For app audio, prefer Stream/Input/Audio (active application
            // playback), fall back to any Audio/Sink.
            streams
                .iter()
                .find(|s| s.is_app_sink())
                .or_else(|| {
                    streams.iter().find(|s| {
                        s.media_class
                            .as_deref()
                            .is_some_and(|c| c.contains("Sink"))
                    })
                })
                .map(|s| s.node_id)
        }
    } else if let Ok(id) = setting.parse::<u32>() {
        // Numeric node ID — verify it exists.
        if streams.iter().any(|s| s.node_id == id) {
            Some(id)
        } else {
            tracing::warn!(node_id = id, "configured node ID not found in PipeWire");
            Some(id) // try anyway; the node might appear later
        }
    } else {
        // Name match.
        streams
            .iter()
            .find(|s| s.name == setting)
            .map(|s| s.node_id)
    }
}

/// Transcribe mic and app audio chunks into the session transcript.
fn transcribe_audio(
    session: &mut Session,
    mic_chunks: Vec<AudioChunk>,
    app_chunks: Vec<AudioChunk>,
) -> Result<()> {
    tracing::info!("starting transcription...");

    #[cfg(feature = "whisper")]
    let transcriber = {
        let tc = vox_core::config::TranscriptionConfig {
            model: session.config_snapshot.model.clone(),
            language: session.config_snapshot.language.clone(),
            gpu_backend: session.config_snapshot.gpu_backend.clone(),
            model_path: String::new(),
        };
        vox_transcribe::WhisperTranscriber::from_config(&tc)
            .map_err(|e| anyhow::anyhow!("failed to load Whisper model: {e}"))?
    };
    #[cfg(not(feature = "whisper"))]
    let transcriber = vox_transcribe::StubTranscriber::new();

    let mic_audio: Vec<f32> = mic_chunks.into_iter().flat_map(|c| c.samples).collect();
    let app_audio: Vec<f32> = app_chunks.into_iter().flat_map(|c| c.samples).collect();

    if !mic_audio.is_empty() {
        let request = TranscriptionRequest::new(mic_audio, AudioSourceRole::Microphone);
        let result = transcriber
            .transcribe(&request)
            .map_err(|e| anyhow::anyhow!("mic transcription failed: {e}"))?;
        session.transcript.extend(result.segments);
    }

    if !app_audio.is_empty() {
        let request = TranscriptionRequest::new(app_audio, AudioSourceRole::Application);
        let result = transcriber
            .transcribe(&request)
            .map_err(|e| anyhow::anyhow!("app transcription failed: {e}"))?;
        session.transcript.extend(result.segments);
    }

    // Remove echo/duplicate segments: when the mic picks up the user's voice
    // AND the application stream also carries it (loopback, monitor, or the
    // call echoing back), the same words appear under both "You" and "Remote".
    // We keep the mic (You) version and drop any app (Remote) segment whose
    // time window overlaps a mic segment and whose text is similar.
    let before = session.transcript.len();
    deduplicate_echo_segments(&mut session.transcript);
    let removed = before - session.transcript.len();
    if removed > 0 {
        tracing::info!("removed {removed} echo/duplicate segments");
    }

    session.transcript.sort_by(|a, b| {
        a.start_time
            .partial_cmp(&b.start_time)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    tracing::info!(
        "transcription complete: {} segments",
        session.transcript.len()
    );
    Ok(())
}

/// Remove "Remote" segments that are echoes of "You" segments.
///
/// An app-stream segment is considered an echo when:
/// 1. Its time window overlaps with a mic segment (within a tolerance), AND
/// 2. Its text is similar to the mic segment's text (word overlap >= 50%).
///
/// This handles the common case where the user's mic audio leaks into the
/// application stream via PipeWire monitor/loopback or the call echoes back.
fn deduplicate_echo_segments(segments: &mut Vec<TranscriptSegment>) {
    // Collect mic segment data for comparison (avoid borrow conflicts with retain).
    let mic_segments: Vec<(f64, f64, String)> = segments
        .iter()
        .filter(|s| s.speaker == "You")
        .map(|s| (s.start_time, s.end_time, s.text.clone()))
        .collect();

    if mic_segments.is_empty() {
        return;
    }

    // Time overlap tolerance in seconds — accounts for slight offset between
    // the two streams being transcribed independently.
    const TIME_TOLERANCE: f64 = 3.0;

    segments.retain(|seg| {
        if seg.speaker != "Remote" {
            return true; // keep all non-Remote segments
        }

        // Check if this Remote segment overlaps any mic segment with similar text.
        let is_echo = mic_segments.iter().any(|(mic_start, mic_end, mic_text)| {
            let time_overlaps = seg.start_time <= mic_end + TIME_TOLERANCE
                && seg.end_time >= mic_start - TIME_TOLERANCE;

            if !time_overlaps {
                return false;
            }

            word_similarity(&seg.text, mic_text) >= 0.5
        });

        if is_echo {
            tracing::debug!(
                start = seg.start_time,
                text = %seg.text,
                "dropping echo segment"
            );
        }

        !is_echo
    });
}

/// Compute word-level Jaccard similarity between two strings.
///
/// Returns a value in `[0.0, 1.0]` where 1.0 means identical word sets.
fn word_similarity(a: &str, b: &str) -> f64 {
    let normalize = |s: &str| -> Vec<String> {
        s.split_whitespace()
            .map(|w| {
                w.trim_matches(|c: char| !c.is_alphanumeric())
                    .to_lowercase()
            })
            .filter(|w| !w.is_empty())
            .collect()
    };

    let words_a: std::collections::HashSet<String> = normalize(a).into_iter().collect();
    let words_b: std::collections::HashSet<String> = normalize(b).into_iter().collect();

    if words_a.is_empty() && words_b.is_empty() {
        return 1.0;
    }
    if words_a.is_empty() || words_b.is_empty() {
        return 0.0;
    }

    let intersection = words_a.intersection(&words_b).count();
    let union = words_a.union(&words_b).count();

    #[allow(clippy::cast_precision_loss)]
    let similarity = intersection as f64 / union as f64;
    similarity
}
