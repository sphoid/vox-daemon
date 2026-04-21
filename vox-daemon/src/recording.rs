//! Recording session management — captures audio, transcribes, and saves results.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use vox_capture::{AudioChunk, AudioSource, StreamRole};
use vox_core::config::AppConfig;
use vox_core::session::{
    AudioRole, AudioSourceInfo, ConfigSnapshot, Session, SpeakerMapping, SpeakerSource,
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

    let merged = transcribe_audio(&config, &mut session, mic_chunks, app_chunks)?;

    // Optionally retain the raw audio for later reprocessing.
    if config.storage.retain_audio && !merged.is_empty() {
        let wav_path = vox_core::paths::sessions_dir_or(&config.storage.data_dir)
            .join(format!("{}.wav", session.id));
        match crate::audio_save::save_wav(&wav_path, &merged) {
            Ok(()) => {
                session.audio_file_path = Some(wav_path.display().to_string());
            }
            Err(e) => {
                tracing::warn!("failed to save audio file (continuing without it): {e}");
            }
        }
    }

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
        diarization_mode: config.transcription.diarization_mode.clone(),
        decoding_strategy: config.transcription.decoding_strategy.clone(),
        initial_prompt: config.transcription.initial_prompt.clone(),
    };

    let mut session = Session::new(audio_sources, config_snapshot);
    let total_samples = mic_samples.max(app_samples);
    session.duration_seconds = (total_samples as u64) / 16_000;

    if config.transcription.diarization_mode == "none" {
        session.speakers = vec![SpeakerMapping {
            id: "Speaker".to_owned(),
            friendly_name: "Speaker".to_owned(),
            source: SpeakerSource::Unknown,
        }];
    } else {
        session.speakers = vec![
            SpeakerMapping {
                id: "You".to_owned(),
                friendly_name: "You".to_owned(),
                source: SpeakerSource::Microphone,
            },
            SpeakerMapping {
                id: "Remote".to_owned(),
                friendly_name: "Remote".to_owned(),
                source: SpeakerSource::Remote,
            },
        ];
    }

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

    tracing::info!("discovered {} PipeWire nodes", all_streams.len());
    for s in &all_streams {
        tracing::info!(
            node_id = s.node_id,
            name = %s.name,
            class = ?s.media_class,
            app = ?s.application_name,
            is_source = s.is_source(),
            is_monitor = s.is_monitor_or_virtual(),
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
            // Prefer a real hardware source (excludes monitors/virtual).
            // Fall back to any source if no hardware source is found.
            let node = streams.iter().find(|s| s.is_source()).or_else(|| {
                let fallback = streams.iter().find(|s| s.is_any_source());
                if let Some(ref fb) = fallback {
                    tracing::warn!(
                        node_id = fb.node_id,
                        name = %fb.name,
                        "no hardware mic found; falling back to virtual/monitor source — \
                         speaker attribution may be unreliable. Set `audio.mic_source` \
                         in config to the correct node ID or name."
                    );
                }
                fallback
            });
            node.map(|s| s.node_id)
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

/// Transcribe audio chunks into the session transcript.
///
/// When `diarization_mode` is `"none"` (default), both streams are merged
/// into a single audio buffer and transcribed once with all segments
/// labelled `"Speaker"`.  This avoids the stream-based attribution
/// problems where mic picks up both speakers and the app stream may be
/// silent or misconfigured.
/// Returns the merged audio buffer so it can optionally be saved to disk.
fn transcribe_audio(
    config: &AppConfig,
    session: &mut Session,
    mic_chunks: Vec<AudioChunk>,
    app_chunks: Vec<AudioChunk>,
) -> Result<Vec<f32>> {
    tracing::info!("starting transcription...");

    // Log audio diagnostics to help troubleshoot source selection issues.
    log_audio_diagnostics("mic", &mic_chunks);
    log_audio_diagnostics("app", &app_chunks);

    #[cfg(feature = "whisper")]
    let transcriber = {
        vox_transcribe::WhisperTranscriber::from_config(&config.transcription)
            .map_err(|e| anyhow::anyhow!("failed to load Whisper model: {e}"))?
    };
    #[cfg(not(feature = "whisper"))]
    let transcriber = vox_transcribe::StubTranscriber::new();

    // Merge both streams into a single audio buffer and transcribe once.
    let merged = crate::audio_merge::merge_chunks(&mic_chunks, &app_chunks);

    if merged.is_empty() {
        tracing::warn!("no audio to transcribe after merging streams");
        return Ok(Vec::new());
    }

    let merged_duration = {
        #[allow(clippy::cast_precision_loss)]
        let d = merged.len() as f64 / 16_000.0;
        d
    };
    tracing::info!(
        "merged audio: {} samples ({:.1}s)",
        merged.len(),
        merged_duration
    );

    let request = TranscriptionRequest::new(merged.clone(), AudioSourceRole::Merged);
    let result = transcriber
        .transcribe(&request)
        .map_err(|e| anyhow::anyhow!("transcription failed: {e}"))?;

    tracing::info!("transcription produced {} segments", result.segments.len());
    session.transcript = result.segments;

    // Run speaker diarization if configured and available.
    #[cfg(feature = "diarize")]
    if config.transcription.diarization_mode == "embedding" {
        run_diarization(config, session, &merged, &mic_chunks)?;
    }
    #[cfg(not(feature = "diarize"))]
    if config.transcription.diarization_mode == "embedding" {
        tracing::warn!(
            "diarization_mode is 'embedding' but the `diarize` feature is not enabled; \
             skipping diarization"
        );
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
    Ok(merged)
}

/// Run ONNX-based speaker diarization on the transcribed segments.
#[cfg(feature = "diarize")]
#[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn run_diarization(
    config: &AppConfig,
    session: &mut Session,
    merged_audio: &[f32],
    mic_chunks: &[AudioChunk],
) -> Result<()> {
    tracing::info!("running speaker diarization...");

    let model_path = vox_diarize::model::resolve_model_path(&config.transcription.diarize_model_path)
        .map_err(|e| anyhow::anyhow!("diarization model error: {e}"))?;

    let diarizer = vox_diarize::OnnxDiarizer::from_model_path(
        &model_path,
        config.transcription.diarize_threshold,
    )
    .map_err(|e| anyhow::anyhow!("failed to load diarization model: {e}"))?;

    // Build enrollment audio from the first N seconds of mic-only chunks.
    let enrollment_samples = (config.transcription.enrollment_seconds * 16_000.0) as usize;
    let enrollment: Vec<f32> = mic_chunks
        .iter()
        .flat_map(|c| c.samples.iter().copied())
        .take(enrollment_samples)
        .collect();

    let enrollment_ref = if enrollment.len() >= 8_000 {
        // Need at least 0.5s for useful enrollment.
        Some(enrollment.as_slice())
    } else {
        tracing::warn!(
            "insufficient mic audio for enrollment ({} samples); \
             'You' identification will be skipped",
            enrollment.len()
        );
        None
    };

    let request = vox_diarize::DiarizationRequest {
        segments: &session.transcript,
        audio: merged_audio,
        enrollment: enrollment_ref,
    };

    match vox_diarize::Diarizer::diarize(&diarizer, &request) {
        Ok(result) => {
            session.transcript = result.segments;
            session.speakers = result.speakers;
            tracing::info!(
                "diarization complete: {} speakers identified",
                session.speakers.len()
            );
        }
        Err(e) => {
            tracing::warn!("diarization failed, keeping undiarized transcript: {e}");
        }
    }

    Ok(())
}

/// Log diagnostic information about captured audio chunks.
///
/// Helps the user verify that the correct PipeWire nodes were selected and
/// that audio is being captured at a reasonable level.
#[allow(clippy::cast_precision_loss)]
fn log_audio_diagnostics(label: &str, chunks: &[AudioChunk]) {
    if chunks.is_empty() {
        tracing::warn!("{label}: no audio chunks captured");
        return;
    }

    let total_samples: usize = chunks.iter().map(|c| c.samples.len()).sum();
    let duration_secs = total_samples as f64 / 16_000.0;

    // Compute overall RMS energy.
    let sum_sq: f64 = chunks
        .iter()
        .flat_map(|c| c.samples.iter())
        .map(|&s| f64::from(s) * f64::from(s))
        .sum();
    let rms = (sum_sq / total_samples as f64).sqrt();

    tracing::info!(
        "{label}: {:.1}s of audio, {total_samples} samples, RMS energy = {rms:.4}",
        duration_secs
    );

    if rms < 0.001 {
        tracing::warn!(
            "{label}: audio energy is extremely low — this stream may be \
             silent or connected to the wrong PipeWire node"
        );
    }
}

/// Re-transcribe a previously recorded session using its saved audio file.
///
/// Loads the session and its retained WAV audio, runs transcription with the
/// current [`AppConfig::transcription`] settings, and overwrites the old
/// transcript.  This allows iterating on transcription parameters without
/// re-recording.
pub fn reprocess_session(config: &AppConfig, session_id: &str) -> Result<()> {
    use uuid::Uuid;

    let id: Uuid = session_id.parse().context("invalid session UUID")?;

    let store =
        JsonFileStore::new(&config.storage.data_dir).context("failed to open session store")?;
    let mut session = store.load(id).context("failed to load session")?;

    let audio_path = session
        .audio_file_path
        .as_deref()
        .filter(|p| !p.is_empty())
        .context(
            "session has no retained audio file — \
             enable `storage.retain_audio` before recording to use reprocessing",
        )?;

    let audio_path = std::path::Path::new(audio_path);
    anyhow::ensure!(
        audio_path.exists(),
        "audio file not found: {}",
        audio_path.display()
    );

    let samples = crate::audio_save::load_wav(audio_path)
        .context("failed to load audio file for reprocessing")?;

    let old_segment_count = session.transcript.len();

    // Build transcriber with current config settings.
    #[cfg(feature = "whisper")]
    let transcriber = {
        vox_transcribe::WhisperTranscriber::from_config(&config.transcription)
            .map_err(|e| anyhow::anyhow!("failed to load Whisper model: {e}"))?
    };
    #[cfg(not(feature = "whisper"))]
    let transcriber = vox_transcribe::StubTranscriber::new();

    let request = TranscriptionRequest::new(samples.clone(), AudioSourceRole::Merged);
    let result = transcriber
        .transcribe(&request)
        .map_err(|e| anyhow::anyhow!("transcription failed: {e}"))?;

    session.transcript = result.segments;

    // Re-run diarization if configured.
    #[cfg(feature = "diarize")]
    if config.transcription.diarization_mode == "embedding" {
        // For reprocessing we don't have separate mic chunks, so skip enrollment.
        tracing::info!("diarization not available during reprocessing (no separate mic audio)");
    }

    session.transcript.sort_by(|a, b| {
        a.start_time
            .partial_cmp(&b.start_time)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    // Update config snapshot to reflect the settings used for this reprocessing.
    session.config_snapshot = ConfigSnapshot {
        model: config.transcription.model.clone(),
        language: config.transcription.language.clone(),
        gpu_backend: config.transcription.gpu_backend.clone(),
        diarization_mode: config.transcription.diarization_mode.clone(),
        decoding_strategy: config.transcription.decoding_strategy.clone(),
        initial_prompt: config.transcription.initial_prompt.clone(),
    };

    store
        .save(&session)
        .context("failed to save reprocessed session")?;

    println!(
        "Session {} reprocessed: {} → {} segments",
        session.id,
        old_segment_count,
        session.transcript.len()
    );

    Ok(())
}


