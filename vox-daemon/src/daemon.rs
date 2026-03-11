//! Daemon mode — runs the background service listening for tray control commands.

use std::time::Instant;

use anyhow::Result;
use vox_core::config::AppConfig;
use vox_notify::{Notifier, StubNotifier};
use vox_tray::{DaemonStatus, Tray, TrayEvent};

use crate::recording;

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

    let notifier = StubNotifier::new();

    #[cfg(feature = "gtk")]
    let tray = vox_tray::SystemTray::new()
        .map_err(|e| anyhow::anyhow!("failed to create system tray: {e}"))?;
    #[cfg(not(feature = "gtk"))]
    let tray = vox_tray::MockTray::new();

    tray.set_status(DaemonStatus::Idle)
        .map_err(|e| anyhow::anyhow!("failed to set tray status: {e}"))?;

    tracing::info!("daemon ready — listening for tray events (press Ctrl+C to stop)");

    // Poll tray events alongside Ctrl+C shutdown
    let shutdown = tokio::signal::ctrl_c();
    tokio::pin!(shutdown);

    loop {
        tokio::select! {
            _ = &mut shutdown => {
                tracing::info!("shutdown signal received, stopping daemon");
                break;
            }
            () = tokio::task::yield_now() => {
                if let Some(event) = tray.try_recv_event() {
                    match handle_tray_event(event, &config, &tray, &notifier).await {
                        Ok(should_quit) if should_quit => break,
                        Ok(_) => {}
                        Err(e) => tracing::error!("error handling tray event: {e}"),
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
    notifier: &impl Notifier,
) -> Result<bool> {
    match event {
        TrayEvent::StartRecording => {
            tracing::info!("recording requested via tray");
            tray.set_status(DaemonStatus::Recording)
                .map_err(|e| anyhow::anyhow!("{e}"))?;
            if let Err(e) = notifier.recording_started() {
                tracing::warn!("notification failed: {e}");
            }
            let start = Instant::now();
            let config_clone = config.clone();
            recording::record_session(config_clone, None, None).await?;
            let duration = start.elapsed();
            if let Err(e) = notifier.recording_stopped(duration) {
                tracing::warn!("notification failed: {e}");
            }
            tray.set_status(DaemonStatus::Idle)
                .map_err(|e| anyhow::anyhow!("{e}"))?;
        }
        TrayEvent::StopRecording => {
            tracing::info!("stop recording requested via tray");
            // In Phase 2, this would signal the recording task to stop.
            tray.set_status(DaemonStatus::Idle)
                .map_err(|e| anyhow::anyhow!("{e}"))?;
        }
        TrayEvent::PauseRecording => {
            tracing::info!("pause recording requested via tray (not yet implemented)");
        }
        TrayEvent::OpenSettings | TrayEvent::OpenLastTranscript | TrayEvent::BrowseTranscripts => {
            #[cfg(feature = "ui")]
            {
                tracing::info!("launching GUI window");
                launch_gui();
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
#[cfg(feature = "ui")]
fn launch_gui() {
    let exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(e) => {
            tracing::error!("cannot determine own executable path: {e}");
            return;
        }
    };
    match std::process::Command::new(exe).arg("gui").spawn() {
        Ok(child) => tracing::info!(pid = child.id(), "GUI process spawned"),
        Err(e) => tracing::error!("failed to spawn GUI process: {e}"),
    }
}
