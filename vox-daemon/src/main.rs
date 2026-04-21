#![warn(clippy::all, clippy::pedantic)]

//! Vox Daemon — Linux-native meeting transcription & summarization service.
//!
//! This is the main binary entrypoint. It provides a CLI interface to:
//! - Start/stop recording sessions
//! - Manage configuration
//! - Run the background daemon

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use tracing_subscriber::EnvFilter;

mod audio_merge;
mod audio_save;
mod daemon;
mod recording;

/// Vox Daemon — Capture, transcribe, and summarize video call audio on Linux.
#[derive(Parser)]
#[command(name = "vox-daemon", version, about)]
struct Cli {
    /// Increase log verbosity (-v for debug, -vv for trace).
    #[arg(short, long, action = clap::ArgAction::Count)]
    verbose: u8,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Start the daemon and begin listening for recording commands.
    Start,

    /// Record a session (blocking — captures audio until stopped with Ctrl+C).
    Record {
        /// Override the microphone source (`PipeWire` node name or ID).
        #[arg(long)]
        mic: Option<String>,

        /// Override the application audio source (`PipeWire` node name or ID).
        #[arg(long)]
        app: Option<String>,
    },

    /// List available `PipeWire` audio sources.
    ListSources,

    /// List past recording sessions.
    ListSessions,

    /// Show the current configuration.
    ShowConfig,

    /// Initialize the default configuration file.
    InitConfig,

    /// Summarize a past session using the configured LLM backend.
    Summarize {
        /// Session UUID to summarize.
        session_id: String,
    },

    /// Export a session to Markdown.
    Export {
        /// Session UUID to export.
        session_id: String,
    },

    /// Re-transcribe a session using its saved audio file and current settings.
    Reprocess {
        /// Session UUID to reprocess.
        session_id: String,
    },

    /// Launch the settings / transcript browser GUI window.
    #[cfg(feature = "ui")]
    Gui {
        /// Which page to open: "settings" or "browser".
        #[arg(long, default_value = "settings")]
        page: String,
    },
}

fn init_logging(verbosity: u8) {
    let filter = match verbosity {
        0 => "vox_daemon=info,vox_core=info,vox_capture=info,vox_transcribe=info,vox_storage=info",
        1 => "debug",
        _ => "trace",
    };

    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(filter)),
        )
        .with_target(true)
        .init();
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    init_logging(cli.verbose);

    // The GUI command must run WITHOUT a tokio runtime because iced creates
    // its own internally. Handle it before building the async runtime.
    #[cfg(feature = "ui")]
    if let Command::Gui { ref page } = cli.command {
        let (initial_page, select_latest) = match page.as_str() {
            "browser" => (vox_gui::app::Page::Browser, false),
            "latest" => (vox_gui::app::Page::Browser, true),
            _ => (vox_gui::app::Page::Settings, false),
        };
        vox_gui::app::run_with_page(initial_page, select_latest)
            .map_err(|e| anyhow::anyhow!("GUI error: {e}"))?;
        return Ok(());
    }

    // All other commands use the tokio async runtime.
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("failed to create tokio runtime")?
        .block_on(async_main(cli))
}

async fn async_main(cli: Cli) -> Result<()> {
    // Ensure XDG directories exist
    let config = vox_core::config::AppConfig::load().context("failed to load configuration")?;
    vox_core::paths::ensure_dirs(&config.storage.data_dir)
        .context("failed to create application directories")?;

    match cli.command {
        #[cfg(feature = "ui")]
        Command::Gui { .. } => unreachable!("handled before runtime"),
        Command::Start => {
            tracing::info!("starting vox-daemon");
            daemon::run(config).await?;
        }
        Command::Record { mic, app } => {
            tracing::info!("starting recording session");
            recording::record_session(config, mic, app, None).await?;
        }
        Command::ListSources => {
            list_sources()?;
        }
        Command::ListSessions => {
            list_sessions(&config)?;
        }
        Command::ShowConfig => {
            let toml_str = toml::to_string_pretty(&config).context("failed to serialize config")?;
            println!("{toml_str}");
        }
        Command::InitConfig => {
            if vox_core::paths::config_dir().join("config.toml").exists() {
                println!("Config file already exists at:");
                println!(
                    "  {}",
                    vox_core::paths::config_dir().join("config.toml").display()
                );
            } else {
                config.save().context("failed to save config")?;
                println!(
                    "Config initialized at {}",
                    vox_core::paths::config_dir().join("config.toml").display()
                );
            }
        }
        Command::Summarize { session_id } => {
            summarize_session(&config, &session_id).await?;
        }
        Command::Export { session_id } => {
            export_session(&config, &session_id)?;
        }
        Command::Reprocess { session_id } => {
            recording::reprocess_session(&config, &session_id)?;
        }
    }

    Ok(())
}

fn list_sources() -> Result<()> {
    use vox_capture::{AudioSource, StreamFilter};

    #[cfg(feature = "pw")]
    let streams = {
        let mut source =
            vox_capture::pw::PipeWireSource::new(vec![]).map_err(|e| anyhow::anyhow!("{e}"))?;
        source
            .list_streams(&StreamFilter::default())
            .map_err(|e| anyhow::anyhow!("{e}"))?
    };
    #[cfg(not(feature = "pw"))]
    let streams = {
        let mut source = vox_capture::mock::MockAudioSource::new();
        source
            .list_streams(&StreamFilter::default())
            .map_err(|e| anyhow::anyhow!("{e}"))?
    };

    if streams.is_empty() {
        println!("No audio sources found. Is PipeWire running?");
        return Ok(());
    }

    println!("{:<8} {:<40} {:<16} {}", "Node ID", "Name", "Class", "App");
    println!("{}", "-".repeat(85));
    for stream in &streams {
        let display_name = stream.description.as_deref().unwrap_or(&stream.name);
        println!(
            "{:<8} {:<40} {:<16} {}",
            stream.node_id,
            display_name,
            stream.media_class.as_deref().unwrap_or("—"),
            stream.application_name.as_deref().unwrap_or("—"),
        );
    }
    Ok(())
}

fn list_sessions(config: &vox_core::config::AppConfig) -> Result<()> {
    use vox_storage::{JsonFileStore, SessionStore};

    let store =
        JsonFileStore::new(&config.storage.data_dir).context("failed to open session store")?;
    let sessions = store.list().context("failed to list sessions")?;

    if sessions.is_empty() {
        println!("No sessions found.");
        return Ok(());
    }

    println!("{:<38} {:<22} {:<10} Segments", "ID", "Date", "Duration");
    println!("{}", "-".repeat(80));
    for session in &sessions {
        let duration = format!("{}s", session.duration_seconds);
        println!(
            "{:<38} {:<22} {:<10} {}",
            session.id,
            session.created_at.format("%Y-%m-%d %H:%M:%S"),
            duration,
            session.transcript.len(),
        );
    }
    Ok(())
}

async fn summarize_session(config: &vox_core::config::AppConfig, session_id: &str) -> Result<()> {
    use uuid::Uuid;
    use vox_storage::{JsonFileStore, SessionStore};

    let id: Uuid = session_id.parse().context("invalid session UUID")?;

    let store =
        JsonFileStore::new(&config.storage.data_dir).context("failed to open session store")?;
    let mut session = store.load(id).context("failed to load session")?;

    if session.transcript.is_empty() {
        anyhow::bail!("session has no transcript segments to summarize");
    }

    tracing::info!(
        "creating summarizer with backend={}",
        config.summarization.backend
    );
    let summarizer = vox_summarize::create_summarizer(&config.summarization)
        .map_err(|e| anyhow::anyhow!("failed to create summarizer: {e}"))?;

    tracing::info!("summarizing {} segments...", session.transcript.len());
    let summary = summarizer
        .summarize(&session.transcript)
        .await
        .map_err(|e| anyhow::anyhow!("summarization failed: {e}"))?;

    println!("Summary generated:");
    println!("  Overview: {}", summary.overview);
    println!("  Key points: {}", summary.key_points.len());
    println!("  Action items: {}", summary.action_items.len());
    println!("  Decisions: {}", summary.decisions.len());

    session.summary = Some(summary);
    store
        .save(&session)
        .context("failed to save session with summary")?;
    tracing::info!("session updated with summary");

    Ok(())
}

fn export_session(config: &vox_core::config::AppConfig, session_id: &str) -> Result<()> {
    use uuid::Uuid;
    use vox_storage::{JsonFileStore, SessionStore};

    let id: Uuid = session_id.parse().context("invalid session UUID")?;

    let store =
        JsonFileStore::new(&config.storage.data_dir).context("failed to open session store")?;
    let markdown = store
        .export_markdown(id)
        .context("failed to export session")?;

    println!("{markdown}");
    Ok(())
}
