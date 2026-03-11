//! Real PipeWire backend for [`AudioSource`].
//!
//! This module is only compiled when the `pw` Cargo feature is enabled, which
//! requires `libpipewire-0.3-dev` to be installed on the build host.
//!
//! # Architecture
//!
//! ```text
//! ┌────────────────────────────────────────────────────────────────┐
//! │ Tokio async context                                            │
//! │                                                                │
//! │  PipeWireSource                                                │
//! │    .start() ─── spawn thread ─── pass targets ───────────────►│
//! │    .stop()  ─── LoopCommand::Stop ────────────────────────────►│
//! │    .stream_receiver() ◄── AudioChunk channel ─────────────────│
//! │                       ◄── LoopEvent  channel ─────────────────│
//! └────────────────────────────────────────────────────────────────┘
//!                                  │
//!              crossbeam channels  │
//!                                  ▼
//! ┌────────────────────────────────────────────────────────────────┐
//! │ Dedicated OS thread ("vox-pipewire")                           │
//! │                                                                │
//! │  loop_thread::run_loop()                                       │
//! │    ├─ PipeWire MainLoop                                        │
//! │    ├─ one Stream per target node                               │
//! │    └─ 10 ms timer polling for LoopCommand::Stop               │
//! └────────────────────────────────────────────────────────────────┘
//! ```
//!
//! [`AudioSource`]: crate::source::AudioSource

pub mod loop_thread;
pub mod registry;

use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, Sender, bounded};
use tracing::{debug, error, info, warn};

use crate::error::CaptureError;
use crate::source::AudioSource;
use crate::types::{AudioChunk, StreamFilter, StreamInfo, StreamRole};
use loop_thread::{LoopCommand, LoopEvent};

/// Capacity of the audio chunk channel.
///
/// At 16 kHz mono f32, a 10 ms chunk is 160 samples × 4 bytes = 640 bytes.
/// 1024 chunks gives ~10 seconds of buffering before the oldest chunks are
/// dropped.
const AUDIO_CHANNEL_CAPACITY: usize = 1024;

/// Capacity of the lifecycle event channel.
const EVENT_CHANNEL_CAPACITY: usize = 64;

/// Timeout for waiting on a [`LoopEvent::Started`] acknowledgement from the
/// PipeWire thread after calling [`start`][AudioSource::start].
const START_ACK_TIMEOUT: Duration = Duration::from_secs(5);

/// State of the [`PipeWireSource`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SourceState {
    Stopped,
    Running,
}

/// A real [`AudioSource`] backed by the PipeWire daemon.
///
/// Spawns a dedicated OS thread running the PipeWire event loop. Audio data
/// is delivered to the caller via a [`crossbeam_channel`] receiver.
///
/// # Usage
///
/// ```rust,no_run
/// # #[cfg(feature = "pw")]
/// # {
/// use vox_capture::pw::PipeWireSource;
/// use vox_capture::{AudioSource, StreamFilter, StreamRole};
///
/// let mut source = PipeWireSource::new(
///     vec![(42, StreamRole::Microphone), (77, StreamRole::Application)],
/// ).unwrap();
///
/// source.start().unwrap();
/// let rx = source.stream_receiver();
/// // … consume chunks from rx …
/// source.stop().unwrap();
/// # }
/// ```
pub struct PipeWireSource {
    /// Node ID + role pairs to capture. Provided at construction.
    targets: Vec<(u32, StreamRole)>,

    /// Sender used to signal the PipeWire thread (e.g., Stop).
    cmd_tx: Sender<LoopCommand>,

    /// Receiving end of the audio chunk channel — handed to callers.
    audio_rx: Receiver<AudioChunk>,

    /// Sending end of the audio chunk channel — kept alive here and given to
    /// the PipeWire thread so the channel stays open.
    audio_tx: Sender<AudioChunk>,

    /// Receiving end of the lifecycle event channel.
    event_rx: Receiver<LoopEvent>,

    state: SourceState,

    /// Handle to the PipeWire OS thread so we can join it on stop.
    thread_handle: Option<std::thread::JoinHandle<()>>,
}

impl PipeWireSource {
    /// Construct a new [`PipeWireSource`] for the given target nodes.
    ///
    /// This does **not** start the PipeWire thread or open any streams; call
    /// [`start`][AudioSource::start] to do that.
    ///
    /// # Parameters
    ///
    /// - `targets` — list of `(node_id, role)` pairs. Obtain node IDs by
    ///   calling [`enumerate_streams`][Self::enumerate_streams] first.
    ///
    /// # Errors
    ///
    /// Currently infallible but returns `Result` for API stability.
    pub fn new(targets: Vec<(u32, StreamRole)>) -> Result<Self, CaptureError> {
        // Create placeholder channels; they are rebuilt on start() anyway.
        let (cmd_tx, _) = bounded::<LoopCommand>(8);
        let (audio_tx, audio_rx) = bounded::<AudioChunk>(AUDIO_CHANNEL_CAPACITY);
        let (_, event_rx) = bounded::<LoopEvent>(EVENT_CHANNEL_CAPACITY);

        Ok(Self {
            targets,
            cmd_tx,
            audio_rx,
            audio_tx,
            event_rx,
            state: SourceState::Stopped,
            thread_handle: None,
        })
    }

    /// Enumerate all PipeWire audio nodes visible to the current session.
    ///
    /// Performs a synchronous registry query (blocks up to 2 seconds). This
    /// is a free function — the source does not need to be started first.
    ///
    /// # Errors
    ///
    /// Returns [`CaptureError::Connection`] if the PipeWire daemon is
    /// unreachable.
    pub fn enumerate_streams(filter: &StreamFilter) -> Result<Vec<StreamInfo>, CaptureError> {
        let all = registry::list_streams(Duration::from_secs(2))?;
        Ok(all.into_iter().filter(|s| filter.matches(s)).collect())
    }
}

impl AudioSource for PipeWireSource {
    fn list_streams(&mut self, filter: &StreamFilter) -> Result<Vec<StreamInfo>, CaptureError> {
        Self::enumerate_streams(filter)
    }

    fn start(&mut self) -> Result<(), CaptureError> {
        if self.state == SourceState::Running {
            return Err(CaptureError::InvalidState("already started".to_owned()));
        }

        // Rebuild all channels so a stop/start cycle starts with clean state.
        let (cmd_tx, cmd_rx) = bounded::<LoopCommand>(8);
        let (audio_tx, audio_rx) = bounded::<AudioChunk>(AUDIO_CHANNEL_CAPACITY);
        let (event_tx, event_rx) = bounded::<LoopEvent>(EVENT_CHANNEL_CAPACITY);

        self.cmd_tx = cmd_tx;
        self.audio_tx = audio_tx.clone();
        self.audio_rx = audio_rx;
        self.event_rx = event_rx;

        let session_start = Instant::now();
        let targets = self.targets.clone();
        let event_tx_thread = event_tx.clone();

        let handle = std::thread::Builder::new()
            .name("vox-pipewire".to_owned())
            .spawn(move || {
                if let Err(e) =
                    loop_thread::run_loop(targets, cmd_rx, audio_tx, event_tx_thread, session_start)
                {
                    error!("PipeWire loop thread failed: {e}");
                }
            })
            .map_err(|e| {
                CaptureError::ThreadPanic(format!("failed to spawn PipeWire thread: {e}"))
            })?;

        self.thread_handle = Some(handle);

        // Wait for the Started (or FatalError) acknowledgement from the thread.
        match self.event_rx.recv_timeout(START_ACK_TIMEOUT) {
            Ok(LoopEvent::Started) => {
                info!("PipeWire capture started successfully");
                self.state = SourceState::Running;
                Ok(())
            }
            Ok(LoopEvent::FatalError(msg)) => Err(CaptureError::Connection(msg)),
            Ok(LoopEvent::Warning(msg)) => {
                // Non-fatal; some streams may have failed but continue.
                warn!("PipeWire start warning: {msg}");
                self.state = SourceState::Running;
                Ok(())
            }
            Ok(other) => {
                warn!("unexpected event during start: {other:?}");
                self.state = SourceState::Running;
                Ok(())
            }
            Err(_timeout) => Err(CaptureError::Connection(
                "PipeWire thread did not acknowledge start within timeout".to_owned(),
            )),
        }
    }

    fn stop(&mut self) -> Result<(), CaptureError> {
        if self.state == SourceState::Stopped {
            debug!("stop() called while already stopped — no-op");
            return Ok(());
        }

        // Ask the PipeWire thread to exit.
        let _ = self.cmd_tx.send(LoopCommand::Stop);

        // Wait for acknowledgement.
        match self.event_rx.recv_timeout(Duration::from_secs(5)) {
            Ok(LoopEvent::Stopped) => {
                info!("PipeWire capture stopped cleanly");
            }
            Ok(other) => {
                warn!("unexpected event during stop: {other:?}");
            }
            Err(_) => {
                warn!("PipeWire thread did not acknowledge stop within 5 s");
            }
        }

        // Join the thread; propagate panics as errors.
        if let Some(handle) = self.thread_handle.take() {
            match handle.join() {
                Ok(()) => debug!("PipeWire thread joined cleanly"),
                Err(_) => {
                    return Err(CaptureError::ThreadPanic(
                        "PipeWire thread panicked".to_owned(),
                    ));
                }
            }
        }

        self.state = SourceState::Stopped;
        Ok(())
    }

    fn stream_receiver(&self) -> &Receiver<AudioChunk> {
        &self.audio_rx
    }
}

impl Drop for PipeWireSource {
    fn drop(&mut self) {
        // Best-effort cleanup on drop. Errors are logged, not propagated.
        if let Err(e) = self.stop() {
            warn!("error during PipeWireSource drop: {e}");
        }
    }
}
