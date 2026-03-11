//! Recording session management — captures audio, transcribes, and saves results.

use std::time::Duration;

use anyhow::{Context, Result};
use vox_capture::{AudioChunk, AudioSource, StreamRole};
use vox_core::config::AppConfig;
use vox_core::session::{
    AudioRole, AudioSourceInfo, ConfigSnapshot, Session, SpeakerMapping, SpeakerSource,
};
use vox_storage::{JsonFileStore, SessionStore};
use vox_transcribe::{AudioSourceRole, StubTranscriber, Transcriber, TranscriptionRequest};

/// Run a recording session.
///
/// Captures audio from `PipeWire` (or a mock source), transcribes it with Whisper
/// (or a stub transcriber), and saves the resulting session to disk.
/// Runs until interrupted with Ctrl+C.
pub async fn record_session(
    config: AppConfig,
    _mic_override: Option<String>,
    _app_override: Option<String>,
) -> Result<()> {
    let (mic_chunks, app_chunks) = capture_audio().await?;

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

    Ok(())
}

/// Capture audio from the source until Ctrl+C.
async fn capture_audio() -> Result<(Vec<AudioChunk>, Vec<AudioChunk>)> {
    // In Phase 1 without the `pw` feature, fall back to the mock source.
    let mut source = vox_capture::mock::MockAudioSource::new();

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

/// Transcribe mic and app audio chunks into the session transcript.
fn transcribe_audio(
    session: &mut Session,
    mic_chunks: Vec<AudioChunk>,
    app_chunks: Vec<AudioChunk>,
) -> Result<()> {
    tracing::info!("starting transcription...");
    let transcriber = StubTranscriber::new();

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
