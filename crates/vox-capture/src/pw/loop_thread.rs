//! The PipeWire event loop thread.
//!
//! PipeWire's [`MainLoop`][pipewire::main_loop::MainLoop] must run on a dedicated OS
//! thread. This module defines the message types used to communicate
//! between the Tokio context and the PipeWire thread, and the function that
//! drives the loop.
//!
//! # Message flow
//!
//! ```text
//! Tokio context                  PipeWire thread
//!      │                               │
//!      │  targets via fn args ─────────►│  open streams on startup
//!      │  LoopCommand::Stop  ──────────►│  close streams, quit loop
//!      │                               │
//!      │◄────── AudioChunk ─────────────│  (via crossbeam audio_tx)
//!      │◄────── LoopEvent  ─────────────│  (via crossbeam event_tx)
//! ```
//!
//! The `Start` command is not sent over the channel — instead the target list
//! is passed directly as a function argument to [`run_loop`]. Only `Stop` is
//! sent over the channel so the PipeWire timer can pick it up.

use std::cell::Cell;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, Sender};
use pipewire::{
    context::ContextBox,
    main_loop::MainLoopBox,
    spa::{
        param::audio::{AudioFormat, AudioInfoRaw},
        pod::Pod,
        utils::Direction,
    },
    stream::{StreamBox, StreamFlags, StreamListener},
};
use tracing::{debug, error, info, warn};

use crate::{
    error::CaptureError,
    resample,
    types::{AudioChunk, StreamRole},
};

/// Commands sent from the Tokio context to the PipeWire thread.
#[derive(Debug)]
pub enum LoopCommand {
    /// Stop all capture streams and quit the event loop.
    Stop,
}

/// Events emitted by the PipeWire thread back to the Tokio context.
#[derive(Debug)]
pub enum LoopEvent {
    /// Capture started successfully and streams are open.
    Started,
    /// Capture stopped cleanly.
    Stopped,
    /// A stream was disconnected (node ID, reason).
    StreamDisconnected {
        /// The PipeWire node ID that disconnected.
        node_id: u32,
        /// Human-readable disconnect reason.
        reason: String,
    },
    /// A recoverable warning.
    Warning(String),
    /// A fatal error; the thread is about to exit.
    FatalError(String),
}

/// Run the PipeWire main loop on the calling thread.
///
/// This function blocks until a [`LoopCommand::Stop`] is received or a fatal
/// error occurs. It **must** be called from a dedicated OS thread — never from
/// a Tokio worker thread.
///
/// Streams are opened immediately for each `(node_id, role)` pair in `targets`
/// before the event loop starts. A [`LoopEvent::Started`] is sent on `event_tx`
/// once all streams have been opened (or attempted).
///
/// # Errors
///
/// Returns [`CaptureError::Connection`] if the PipeWire connection itself fails.
/// All subsequent runtime errors are reported via `event_tx` as
/// [`LoopEvent::Warning`] or [`LoopEvent::FatalError`].
pub fn run_loop(
    targets: Vec<(u32, StreamRole)>,
    cmd_rx: Receiver<LoopCommand>,
    audio_tx: Sender<AudioChunk>,
    event_tx: Sender<LoopEvent>,
    session_start: Instant,
) -> Result<(), CaptureError> {
    // Initialise the PipeWire library. Safe to call multiple times.
    pipewire::init();

    // MainLoopBox::new(None) creates a new owned main loop (None = no extra properties).
    let main_loop = MainLoopBox::new(None).map_err(|e| {
        CaptureError::Connection(format!("failed to create PipeWire MainLoop: {e}"))
    })?;

    // ContextBox::new takes &Loop (obtained via main_loop.loop_()) and Option<PropertiesBox>.
    let context = ContextBox::new(main_loop.loop_(), None)
        .map_err(|e| CaptureError::Connection(format!("failed to create PipeWire Context: {e}")))?;

    let core = context.connect(None).map_err(|e| {
        CaptureError::Connection(format!("failed to connect to PipeWire daemon: {e}"))
    })?;

    info!("connected to PipeWire daemon");

    // Shared quit flag: the timer closure sets it; the main polling loop reads it.
    let quit = Arc::new(AtomicBool::new(false));

    // Active capture streams, keyed by node ID.
    // Stored in a plain HashMap — single-threaded (PipeWire thread only).
    // NOT captured by the timer closure to avoid 'static lifetime conflicts.
    let mut active_streams: HashMap<u32, (StreamBox<'_>, StreamListener<()>)> = HashMap::new();

    // --- Timer: poll the command channel every 10 ms ---
    let quit_for_timer = Arc::clone(&quit);
    let event_tx_timer = event_tx.clone();

    let ml_loop = main_loop.loop_();
    // add_timer callback receives the number of expirations (u64).
    // It only sets the quit flag; stream cleanup happens after the loop exits.
    let _timer = {
        let cmd_rx = cmd_rx.clone();
        ml_loop.add_timer(move |_expirations: u64| {
            loop {
                match cmd_rx.try_recv() {
                    Ok(LoopCommand::Stop) => {
                        debug!("PipeWire thread received Stop command");
                        quit_for_timer.store(true, Ordering::Relaxed);
                        let _ = event_tx_timer.try_send(LoopEvent::Stopped);
                    }
                    Err(crossbeam_channel::TryRecvError::Empty) => break,
                    Err(crossbeam_channel::TryRecvError::Disconnected) => {
                        // Tokio side dropped the handle; shut down gracefully.
                        quit_for_timer.store(true, Ordering::Relaxed);
                        break;
                    }
                }
            }
        })
    };

    // Arm the timer to fire every 10 ms. update_timer takes Option<Duration> value and interval.
    _timer.update_timer(
        Some(Duration::from_millis(10)),
        Some(Duration::from_millis(10)),
    );

    // --- Open capture streams for all requested targets ---
    for (node_id, role) in &targets {
        match open_capture_stream(
            &core,
            *node_id,
            *role,
            audio_tx.clone(),
            event_tx.clone(),
            session_start,
        ) {
            Ok((stream, listener)) => {
                active_streams.insert(*node_id, (stream, listener));
                info!(node_id, %role, "opened capture stream");
            }
            Err(e) => {
                let msg = format!("failed to open stream for node {node_id}: {e}");
                error!("{}", msg);
                let _ = event_tx.try_send(LoopEvent::Warning(msg));
            }
        }
    }

    // Signal the caller that we are up and running.
    let _ = event_tx.try_send(LoopEvent::Started);

    // --- Main event loop ---
    while !quit.load(Ordering::Relaxed) {
        ml_loop.iterate(Duration::from_millis(10));
    }

    // Drop streams before tearing down the PipeWire context.
    drop(active_streams);

    info!("PipeWire event loop exiting cleanly");
    Ok(())
}

/// Serialise [`AudioInfoRaw`] into a SPA Pod suitable for passing to
/// [`StreamBox::connect`].
fn make_audio_param(rate: u32, channels: u32) -> Result<Vec<u8>, CaptureError> {
    use pipewire::spa::param::ParamType;
    use pipewire::spa::pod::{Object, Value, serialize::PodSerializer};
    use pipewire::spa::utils::SpaTypes;

    let mut audio_info = AudioInfoRaw::new();
    audio_info.set_format(AudioFormat::F32LE);
    audio_info.set_rate(rate);
    audio_info.set_channels(channels);

    let obj = Value::Object(Object {
        type_: SpaTypes::ObjectParamFormat.as_raw(),
        id: ParamType::EnumFormat.as_raw(),
        properties: audio_info.into(),
    });

    PodSerializer::serialize(std::io::Cursor::new(Vec::new()), &obj)
        .map(|(writer, _)| writer.into_inner())
        .map_err(|e| CaptureError::Format(format!("failed to serialize audio params: {e}")))
}

/// Open a single PipeWire capture stream connected to `node_id`.
///
/// The audio callback converts received buffers to mono 16 kHz f32 PCM and
/// sends [`AudioChunk`]s into `audio_tx`.
fn open_capture_stream(
    core: &pipewire::core::Core,
    node_id: u32,
    role: StreamRole,
    audio_tx: Sender<AudioChunk>,
    event_tx: Sender<LoopEvent>,
    session_start: Instant,
) -> Result<(StreamBox<'_>, StreamListener<()>), CaptureError> {
    use pipewire::properties::properties;

    // properties! macro returns a PropertiesBox, which StreamBox::new requires.
    let props = properties! {
        "media.type"     => "Audio",
        "media.category" => "Capture",
        "media.role"     => "Music",
        // Target the specific node. Deprecated in newer PW but still widely supported.
        "node.target"    => node_id.to_string().as_str(),
    };

    // StreamBox::new takes (&Core, &str, PropertiesBox).
    let stream = StreamBox::new(core, "vox-capture", props)
        .map_err(|e| CaptureError::Stream(format!("failed to create stream: {e}")))?;

    // Request F32LE 48 kHz stereo — the most common hardware default.
    // We resample to 16 kHz mono in the process callback.
    let param_bytes = make_audio_param(48_000, 2)?;
    let param_pod = Pod::from_bytes(&param_bytes)
        .ok_or_else(|| CaptureError::Format("failed to interpret param bytes as Pod".to_owned()))?;

    // These cells track the negotiated format so the process callback can use
    // the correct values even after a mid-session format change.
    let src_rate = std::rc::Rc::new(Cell::new(48_000_u32));
    let src_channels = std::rc::Rc::new(Cell::new(2_u32));

    // Clones for process callback.
    let rate_proc = std::rc::Rc::clone(&src_rate);
    let chan_proc = std::rc::Rc::clone(&src_channels);

    // Clones for param_changed callback.
    let rate_fmt = std::rc::Rc::clone(&src_rate);
    let chan_fmt = std::rc::Rc::clone(&src_channels);

    // Callback signatures in v0.9.2:
    //   process:       FnMut(&Stream, &mut D)
    //   param_changed: FnMut(&Stream, &mut D, u32, Option<&Pod>)
    //   state_changed: FnMut(&Stream, &mut D, StreamState, StreamState)
    let listener = stream
        .add_local_listener_with_user_data(())
        .process(move |stream, _user_data: &mut ()| {
            let Some(mut buffer) = stream.dequeue_buffer() else {
                return;
            };

            let datas = buffer.datas_mut();
            if datas.is_empty() {
                return;
            }

            let data = &mut datas[0];
            // Extract chunk size first, then release the immutable borrow
            // before taking the mutable borrow via data().
            #[allow(clippy::cast_possible_truncation)]
            let chunk_size = data.chunk().size() as usize;

            let raw_bytes = match data.data() {
                Some(bytes) => &*bytes,
                None => return,
            };
            if chunk_size == 0 || raw_bytes.len() < chunk_size {
                return;
            }
            let byte_slice = &raw_bytes[..chunk_size];

            // F32LE: 4 bytes per sample.
            if byte_slice.len() % 4 != 0 {
                warn!(
                    node_id,
                    "PipeWire buffer length {} is not a multiple of 4; skipping",
                    byte_slice.len()
                );
                return;
            }

            let samples: Vec<f32> = byte_slice
                .chunks_exact(4)
                .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
                .collect();

            let rate = rate_proc.get();
            let channels = chan_proc.get();

            match resample::convert(&samples, rate, channels) {
                Ok(pcm) => {
                    let ts = session_start.elapsed();
                    let audio_chunk = AudioChunk::new(pcm, ts, role);
                    if audio_tx.try_send(audio_chunk).is_err() {
                        warn!(node_id, %role, "audio channel full; dropping chunk");
                    }
                }
                Err(e) => {
                    warn!(node_id, %role, "resampling failed: {e}");
                }
            }
        })
        .param_changed(move |_stream, _user_data: &mut (), id, param| {
            use pipewire::spa::param::ParamType;
            if id != ParamType::Format.as_raw() {
                return;
            }
            let Some(param) = param else { return };
            // AudioInfoRaw::parse is a &mut self method in v0.9.2.
            let mut info = AudioInfoRaw::new();
            if info.parse(param).is_ok() {
                let new_rate = info.rate();
                let new_channels = info.channels();
                if new_rate > 0 {
                    rate_fmt.set(new_rate);
                }
                if new_channels > 0 {
                    chan_fmt.set(new_channels);
                }
                debug!(
                    node_id,
                    rate = new_rate,
                    channels = new_channels,
                    "stream format negotiated / changed"
                );
            }
        })
        .state_changed(move |_stream, _user_data: &mut (), old, new| {
            use pipewire::stream::StreamState;
            debug!(node_id, ?old, ?new, "stream state changed");
            match new {
                StreamState::Error(ref msg) => {
                    error!(node_id, "stream error: {msg}");
                    let _ = event_tx.try_send(LoopEvent::StreamDisconnected {
                        node_id,
                        reason: msg.clone(),
                    });
                }
                StreamState::Unconnected => {
                    info!(node_id, "stream disconnected (unconnected state)");
                    let _ = event_tx.try_send(LoopEvent::StreamDisconnected {
                        node_id,
                        reason: "unconnected".to_owned(),
                    });
                }
                _ => {}
            }
        })
        .register()
        .map_err(|e| CaptureError::Stream(format!("failed to register stream listener: {e}")))?;

    // Connect the stream. params is &mut [&Pod] in v0.9.2.
    let mut params = [param_pod];
    stream
        .connect(
            Direction::Input,
            Some(node_id),
            StreamFlags::AUTOCONNECT | StreamFlags::MAP_BUFFERS | StreamFlags::RT_PROCESS,
            &mut params,
        )
        .map_err(|e| {
            CaptureError::Stream(format!("failed to connect stream to node {node_id}: {e}"))
        })?;

    Ok((stream, listener))
}
