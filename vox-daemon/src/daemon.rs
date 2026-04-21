//! Daemon mode — runs the background service listening for tray control commands.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use anyhow::Result;
use vox_core::config::AppConfig;
use vox_notify::Notifier;
use vox_tray::{DaemonStatus, Tray, TrayEvent};

use crate::recording;

/// State of a recording task running in the background.
struct RecordingTask {
    /// Handle to the spawned tokio task.
    handle: tokio::task::JoinHandle<Result<()>>,
    /// Flag to signal the recording to stop.
    stop_flag: Arc<AtomicBool>,
    /// When the recording started.
    started_at: Instant,
}

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

    // Active recording task, if any.
    let mut active_recording: Option<RecordingTask> = None;

    // Poll tray events alongside Ctrl+C shutdown
    let shutdown = tokio::signal::ctrl_c();
    tokio::pin!(shutdown);

    loop {
        tokio::select! {
            _ = &mut shutdown => {
                tracing::info!("shutdown signal received, stopping daemon");
                // Stop any active recording before exiting.
                if let Some(task) = active_recording.take() {
                    task.stop_flag.store(true, Ordering::Relaxed);
                    match task.handle.await {
                        Ok(Ok(())) => tracing::info!("recording stopped cleanly on shutdown"),
                        Ok(Err(e)) => tracing::warn!("recording ended with error on shutdown: {e}"),
                        Err(e) => tracing::error!("recording task panicked on shutdown: {e}"),
                    }
                }
                break;
            }
            () = tokio::task::yield_now() => {
                if let Some(event) = tray.try_recv_event() {
                    match handle_tray_event(event, &config, &tray, &*notifier, &mut active_recording).await {
                        Ok(should_quit) if should_quit => {
                            // Stop any active recording before quitting.
                            if let Some(task) = active_recording.take() {
                                task.stop_flag.store(true, Ordering::Relaxed);
                                let _ = task.handle.await;
                            }
                            break;
                        }
                        Ok(_) => {}
                        Err(e) => tracing::error!("error handling tray event: {e}"),
                    }
                }

                // Check if the active recording has finished on its own.
                if let Some(ref task) = active_recording {
                    if task.handle.is_finished() {
                        let task = active_recording.take().expect("just checked Some");
                        let duration = task.started_at.elapsed();
                        match task.handle.await {
                            Ok(Ok(())) => {
                                tracing::info!("recording completed successfully");
                                if let Err(e) = notifier.recording_stopped(duration) {
                                    tracing::warn!("notification failed: {e}");
                                }
                            }
                            Ok(Err(e)) => {
                                tracing::error!("recording failed: {e}");
                            }
                            Err(e) => {
                                tracing::error!("recording task panicked: {e}");
                            }
                        }
                        tray.set_status(DaemonStatus::Idle)
                            .map_err(|e| anyhow::anyhow!("{e}"))?;
                    }
                }

                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
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
    active_recording: &mut Option<RecordingTask>,
) -> Result<bool> {
    match event {
        TrayEvent::StartRecording => {
            if active_recording.is_some() {
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
                recording::record_session(config_clone, None, None, Some(flag_clone)).await
            });

            *active_recording = Some(RecordingTask {
                handle,
                stop_flag,
                started_at,
            });
        }
        TrayEvent::StopRecording => {
            if let Some(task) = active_recording.take() {
                tracing::info!("stop recording requested via tray");
                task.stop_flag.store(true, Ordering::Relaxed);
                let duration = task.started_at.elapsed();

                match task.handle.await {
                    Ok(Ok(())) => {
                        tracing::info!("recording stopped successfully");
                        if let Err(e) = notifier.recording_stopped(duration) {
                            tracing::warn!("notification failed: {e}");
                        }
                    }
                    Ok(Err(e)) => {
                        tracing::error!("recording ended with error: {e}");
                    }
                    Err(e) => {
                        tracing::error!("recording task panicked: {e}");
                    }
                }
            } else {
                tracing::info!("stop recording requested but no recording in progress");
            }
            tray.set_status(DaemonStatus::Idle)
                .map_err(|e| anyhow::anyhow!("{e}"))?;
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
