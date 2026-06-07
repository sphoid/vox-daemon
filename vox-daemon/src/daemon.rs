//! Daemon mode — runs the background service listening for tray control commands.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use anyhow::Result;
use uuid::Uuid;
use vox_core::config::AppConfig;
use vox_notify::Notifier;
use vox_tray::{DaemonStatus, Tray, TrayEvent};

use crate::recording::{self, CaptureOutput};

/// State of a capture (recording) task running in the background.
///
/// Only the audio capture runs while this task is active. Transcription and
/// summarization run in a separate [`PendingProcessing`] task spawned after
/// capture completes, so the tray can return to `Idle` immediately.
struct CaptureTask {
    /// Handle to the spawned capture task.
    handle: tokio::task::JoinHandle<Result<CaptureOutput>>,
    /// Flag to signal the capture to stop.
    stop_flag: Arc<AtomicBool>,
    /// When capture started.
    started_at: Instant,
}

/// A background processing job (transcription + save + summarization) for a
/// session whose capture phase has already finished.
struct PendingProcessing {
    /// UUID of the session being processed.
    session_id: Uuid,
    /// Handle to the spawned processing task.
    handle: tokio::task::JoinHandle<Result<recording::ProcessingOutcome>>,
}

/// Maximum time the daemon waits for in-flight background processing to
/// finish during shutdown before abandoning it.
const SHUTDOWN_PROCESSING_TIMEOUT: Duration = Duration::from_secs(60);

/// Run the daemon process.
///
/// Spawns a system tray and listens for user commands such as start/stop
/// recording, open settings, and quit.
/// When built with the `gtk` feature, a real system tray icon appears.
/// Otherwise, a mock tray is used (useful for testing / headless mode).
pub async fn run(config: AppConfig) -> Result<()> {
    tracing::info!(
        "daemon started with config: model={}, language={}",
        config.transcription.model,
        config.transcription.language
    );

    let notifier: Box<dyn Notifier> = Box::new(vox_notify::DesktopNotifier::new(
        config.notifications.clone(),
    ));

    #[cfg(feature = "gtk")]
    let tray = vox_tray::SystemTray::new()
        .map_err(|e| anyhow::anyhow!("failed to create system tray: {e}"))?;
    #[cfg(not(feature = "gtk"))]
    let tray = vox_tray::MockTray::new();

    tray.set_status(DaemonStatus::Idle)
        .map_err(|e| anyhow::anyhow!("failed to set tray status: {e}"))?;

    tracing::info!("daemon ready — listening for tray events (press Ctrl+C to stop)");

    // Active capture task, if any.
    let mut active_capture: Option<CaptureTask> = None;
    // Background processing jobs for previously captured sessions.
    let mut pending_processing: Vec<PendingProcessing> = Vec::new();

    // Poll tray events alongside Ctrl+C shutdown
    let shutdown = tokio::signal::ctrl_c();
    tokio::pin!(shutdown);

    loop {
        tokio::select! {
            _ = &mut shutdown => {
                tracing::info!("shutdown signal received, stopping daemon");
                // Stop any active capture before exiting.
                if let Some(task) = active_capture.take() {
                    task.stop_flag.store(true, Ordering::Relaxed);
                    let duration = task.started_at.elapsed();
                    let capture_result = task.handle.await;
                    finish_capture(
                        capture_result,
                        duration,
                        &tray,
                        &*notifier,
                        &mut pending_processing,
                    )?;
                }
                drain_pending(pending_processing, &*notifier, SHUTDOWN_PROCESSING_TIMEOUT).await;
                break;
            }
            () = tokio::task::yield_now() => {
                if let Some(event) = tray.try_recv_event() {
                    match handle_tray_event(
                        event,
                        &config,
                        &tray,
                        &*notifier,
                        &mut active_capture,
                        &mut pending_processing,
                    ).await {
                        Ok(should_quit) if should_quit => {
                            // Stop any active capture before quitting.
                            if let Some(task) = active_capture.take() {
                                task.stop_flag.store(true, Ordering::Relaxed);
                                let duration = task.started_at.elapsed();
                                let capture_result = task.handle.await;
                                finish_capture(
                                    capture_result,
                                    duration,
                                    &tray,
                                    &*notifier,
                                    &mut pending_processing,
                                )?;
                            }
                            drain_pending(
                                pending_processing,
                                &*notifier,
                                SHUTDOWN_PROCESSING_TIMEOUT,
                            ).await;
                            break;
                        }
                        Ok(_) => {}
                        Err(e) => tracing::error!("error handling tray event: {e}"),
                    }
                }

                // Check if the active capture has finished on its own (e.g.,
                // capture loop's own Ctrl+C handler fired).
                if let Some(ref task) = active_capture {
                    if task.handle.is_finished() {
                        let task = active_capture.take().expect("just checked Some");
                        let duration = task.started_at.elapsed();
                        let capture_result = task.handle.await;
                        finish_capture(
                            capture_result,
                            duration,
                            &tray,
                            &*notifier,
                            &mut pending_processing,
                        )?;
                    }
                }

                // Check for completed background processing jobs and fire
                // transcript_ready / summary_ready notifications.
                poll_pending_processing(&mut pending_processing, &*notifier).await;

                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        }
    }

    Ok(())
}

/// Handle a single tray event.
///
/// Returns `Ok(true)` when the daemon should quit.
async fn handle_tray_event(
    event: TrayEvent,
    config: &AppConfig,
    tray: &impl Tray,
    notifier: &dyn Notifier,
    active_capture: &mut Option<CaptureTask>,
    pending_processing: &mut Vec<PendingProcessing>,
) -> Result<bool> {
    match event {
        TrayEvent::StartRecording => {
            if active_capture.is_some() {
                tracing::warn!("recording already in progress, ignoring start request");
                return Ok(false);
            }

            tracing::info!("recording requested via tray");
            tray.set_status(DaemonStatus::Recording)
                .map_err(|e| anyhow::anyhow!("{e}"))?;
            if let Err(e) = notifier.recording_started() {
                tracing::warn!("notification failed: {e}");
            }

            let stop_flag = Arc::new(AtomicBool::new(false));
            let flag_clone = Arc::clone(&stop_flag);
            // Re-read config from disk so GUI settings changes take effect
            // without restarting the daemon.
            let config_clone = AppConfig::load().unwrap_or_else(|e| {
                tracing::warn!("failed to reload config, using startup config: {e}");
                config.clone()
            });
            let started_at = Instant::now();

            let handle = tokio::spawn(async move {
                recording::capture_session(config_clone, None, None, Some(flag_clone)).await
            });

            *active_capture = Some(CaptureTask {
                handle,
                stop_flag,
                started_at,
            });
        }
        TrayEvent::StopRecording => {
            if let Some(task) = active_capture.take() {
                tracing::info!("stop recording requested via tray");
                task.stop_flag.store(true, Ordering::Relaxed);
                let duration = task.started_at.elapsed();
                let capture_result = task.handle.await;
                finish_capture(capture_result, duration, tray, notifier, pending_processing)?;
            } else {
                tracing::info!("stop recording requested but no recording in progress");
                tray.set_status(DaemonStatus::Idle)
                    .map_err(|e| anyhow::anyhow!("{e}"))?;
            }
        }
        TrayEvent::PauseRecording => {
            tracing::info!("pause recording requested via tray (not yet implemented)");
        }
        TrayEvent::OpenSettings => {
            #[cfg(feature = "ui")]
            {
                tracing::info!("launching GUI window (settings)");
                launch_gui("settings");
            }
            #[cfg(not(feature = "ui"))]
            tracing::info!("GUI not available (build with --features ui)");
        }
        TrayEvent::OpenLastTranscript => {
            #[cfg(feature = "ui")]
            {
                tracing::info!("launching GUI window (latest transcript)");
                launch_gui("latest");
            }
            #[cfg(not(feature = "ui"))]
            tracing::info!("GUI not available (build with --features ui)");
        }
        TrayEvent::BrowseTranscripts => {
            #[cfg(feature = "ui")]
            {
                tracing::info!("launching GUI window (browser)");
                launch_gui("browser");
            }
            #[cfg(not(feature = "ui"))]
            tracing::info!("GUI not available (build with --features ui)");
        }
        TrayEvent::Quit => {
            tracing::info!("quit requested via tray");
            return Ok(true);
        }
    }
    Ok(false)
}

/// Finalise the capture phase: flip the tray to Idle, fire the
/// `recording_stopped` notification, and if any audio was captured spawn a
/// background processing task for transcription + summarization.
///
/// The tray is updated **before** processing is spawned so the user can
/// immediately start a new recording while the prior session is still being
/// transcribed.
fn finish_capture(
    capture_result: std::result::Result<Result<CaptureOutput>, tokio::task::JoinError>,
    duration: Duration,
    tray: &impl Tray,
    notifier: &dyn Notifier,
    pending_processing: &mut Vec<PendingProcessing>,
) -> Result<()> {
    // Flip the tray back to Idle first so the user can start a new recording
    // immediately. The notification + spawn happen on the heels of this.
    tray.set_status(DaemonStatus::Idle)
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    match capture_result {
        Ok(Ok(capture)) => {
            tracing::info!("capture stopped successfully");
            if let Err(e) = notifier.recording_stopped(duration) {
                tracing::warn!("notification failed: {e}");
            }
            if capture.is_empty() {
                tracing::warn!("no audio captured, skipping transcription");
                return Ok(());
            }
            let session_id = capture.session.id;
            let handle = tokio::spawn(async move { recording::process_session(capture).await });
            pending_processing.push(PendingProcessing { session_id, handle });
            tracing::info!(
                "processing spawned in background for session {session_id} \
                 ({} job(s) pending)",
                pending_processing.len()
            );
        }
        Ok(Err(e)) => {
            tracing::error!("capture ended with error: {e}");
            if let Err(e) = notifier.recording_stopped(duration) {
                tracing::warn!("notification failed: {e}");
            }
        }
        Err(e) => {
            tracing::error!("capture task panicked: {e}");
        }
    }
    Ok(())
}

/// Sweep the pending-processing list for finished jobs and fire their
/// completion notifications.
async fn poll_pending_processing(pending: &mut Vec<PendingProcessing>, notifier: &dyn Notifier) {
    let mut i = 0;
    while i < pending.len() {
        if pending[i].handle.is_finished() {
            let job = pending.swap_remove(i);
            handle_processing_result(job, notifier).await;
        } else {
            i += 1;
        }
    }
}

/// Await a single completed processing job and fire the appropriate
/// notifications.
async fn handle_processing_result(job: PendingProcessing, notifier: &dyn Notifier) {
    let PendingProcessing { session_id, handle } = job;
    match handle.await {
        Ok(Ok(outcome)) => {
            tracing::info!(
                "background processing complete for session {}",
                outcome.session_id
            );
            if let Err(e) = notifier.transcript_ready(outcome.session_id) {
                tracing::warn!("transcript notification failed: {e}");
            }
            if outcome.summary_generated {
                if let Err(e) = notifier.summary_ready(outcome.session_id) {
                    tracing::warn!("summary notification failed: {e}");
                }
            }
        }
        Ok(Err(e)) => {
            tracing::error!("background processing failed for session {session_id}: {e}");
        }
        Err(e) => {
            tracing::error!("background processing task panicked for session {session_id}: {e}");
        }
    }
}

/// Drain in-flight processing jobs during shutdown, bounded by a total
/// timeout. Jobs that don't finish in time are abandoned with a warning.
async fn drain_pending(
    pending: Vec<PendingProcessing>,
    notifier: &dyn Notifier,
    timeout: Duration,
) {
    if pending.is_empty() {
        return;
    }
    tracing::info!(
        "waiting up to {:?} for {} background processing job(s) before exit",
        timeout,
        pending.len()
    );
    let drained = tokio::time::timeout(timeout, async {
        for job in pending {
            handle_processing_result(job, notifier).await;
        }
    })
    .await;
    if drained.is_err() {
        tracing::warn!("shutdown timeout reached; abandoning remaining in-flight processing jobs");
    }
}

/// Spawn the GUI as a child process so iced/winit can use the main thread.
///
/// `page` should be `"settings"` or `"browser"`.
#[cfg(feature = "ui")]
fn launch_gui(page: &str) {
    let exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(e) => {
            tracing::error!("cannot determine own executable path: {e}");
            return;
        }
    };
    match std::process::Command::new(exe)
        .args(["gui", "--page", page])
        .spawn()
    {
        Ok(child) => tracing::info!(pid = child.id(), "GUI process spawned"),
        Err(e) => tracing::error!("failed to spawn GUI process: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::time::Duration;

    use tempfile::TempDir;
    use vox_capture::types::{AudioChunk, StreamRole};
    use vox_core::session::{AudioRole, AudioSourceInfo, ConfigSnapshot, Session};
    use vox_notify::StubNotifier;
    use vox_tray::MockTray;

    fn test_config_with_tempdir(dir: &TempDir) -> AppConfig {
        let mut cfg = AppConfig::default();
        cfg.storage.data_dir = dir.path().to_string_lossy().into_owned();
        cfg.summarization.auto_summarize = false;
        cfg
    }

    fn dummy_session(cfg: &AppConfig) -> Session {
        let sources = vec![AudioSourceInfo {
            name: "mic".to_owned(),
            pipewire_node_id: 0,
            role: AudioRole::Microphone,
        }];
        let snap = ConfigSnapshot {
            model: cfg.transcription.model.clone(),
            language: cfg.transcription.language.clone(),
            gpu_backend: cfg.transcription.gpu_backend.clone(),
            diarization_mode: cfg.transcription.diarization_mode.clone(),
            decoding_strategy: cfg.transcription.decoding_strategy.clone(),
            initial_prompt: cfg.transcription.initial_prompt.clone(),
        };
        Session::new(sources, snap)
    }

    /// Core invariant of the fix: `finish_capture` must hand the heavy work
    /// to a background task and return immediately, so the tray flips to
    /// `Idle` and the `recording_stopped` notification fires without
    /// waiting for transcription / summarization.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn finish_capture_returns_without_awaiting_processing() {
        let tmp = TempDir::new().expect("tempdir");
        let cfg = test_config_with_tempdir(&tmp);
        let session = dummy_session(&cfg);

        let mic_chunk = AudioChunk::new(
            vec![0.1_f32; 16_000],
            Duration::ZERO,
            StreamRole::Microphone,
        );
        let capture = CaptureOutput {
            config: cfg,
            session,
            mic_chunks: vec![mic_chunk],
            app_chunks: Vec::new(),
        };

        let tray = MockTray::new();
        let notifier = StubNotifier::new();
        let mut pending: Vec<PendingProcessing> = Vec::new();

        let before = std::time::Instant::now();
        finish_capture(
            Ok(Ok(capture)),
            Duration::from_secs(1),
            &tray,
            &notifier,
            &mut pending,
        )
        .expect("finish_capture should succeed");
        let elapsed = before.elapsed();

        // Tray flipped to Idle before we returned.
        assert_eq!(tray.last_status(), Some(DaemonStatus::Idle));
        // Exactly one background processing job was queued.
        assert_eq!(pending.len(), 1);
        // finish_capture itself is synchronous: it must not block on the
        // spawned task. Allow a generous bound to keep CI happy.
        assert!(
            elapsed < Duration::from_millis(250),
            "finish_capture took {elapsed:?}; it must not await processing"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn finish_capture_empty_capture_does_not_spawn_processing() {
        let tmp = TempDir::new().expect("tempdir");
        let cfg = test_config_with_tempdir(&tmp);
        let capture = CaptureOutput {
            session: dummy_session(&cfg),
            config: cfg,
            mic_chunks: Vec::new(),
            app_chunks: Vec::new(),
        };

        let tray = MockTray::new();
        let notifier = StubNotifier::new();
        let mut pending: Vec<PendingProcessing> = Vec::new();

        finish_capture(
            Ok(Ok(capture)),
            Duration::from_secs(1),
            &tray,
            &notifier,
            &mut pending,
        )
        .expect("finish_capture should succeed");

        assert_eq!(tray.last_status(), Some(DaemonStatus::Idle));
        assert!(pending.is_empty());
    }
}
