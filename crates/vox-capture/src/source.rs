//! The [`AudioSource`] trait — the primary public API of `vox-capture`.
//!
//! Downstream crates should program against this trait rather than the
//! concrete [`PipeWireSource`][crate::pw::PipeWireSource] type so that they
//! can be tested with a [`MockAudioSource`][crate::mock::MockAudioSource].

use crate::error::CaptureError;
use crate::types::{AudioChunk, StreamFilter, StreamInfo};

/// Primary interface for audio capture backends.
///
/// Implementors must be [`Send`] because the controller lives in an async
/// Tokio task while the underlying work happens on a separate OS thread.
///
/// # Example
///
/// ```rust,no_run
/// # use vox_capture::{AudioSource, StreamFilter};
/// fn record<S: AudioSource>(mut source: S) -> Result<(), vox_capture::CaptureError> {
///     let streams = source.list_streams(&StreamFilter::default())?;
///     println!("Found {} streams", streams.len());
///     source.start()?;
///     let rx = source.stream_receiver();
///     // drain the channel for a while …
///     source.stop()
/// }
/// ```
pub trait AudioSource: Send {
    /// Return metadata for all currently visible `PipeWire` audio nodes that
    /// match the given `filter`. An empty filter returns everything.
    ///
    /// This performs a synchronous registry query on the `PipeWire` thread. It
    /// may block for up to a few hundred milliseconds while waiting for the
    /// registry to reply.
    ///
    /// # Errors
    ///
    /// Returns [`CaptureError::Connection`] if the `PipeWire` daemon is
    /// unreachable, or [`CaptureError::Stream`] if the query fails.
    fn list_streams(&mut self, filter: &StreamFilter) -> Result<Vec<StreamInfo>, CaptureError>;

    /// Begin capturing audio from the configured streams.
    ///
    /// Audio chunks are pushed into the channel accessible via
    /// [`stream_receiver`][Self::stream_receiver]. Calling `start` while
    /// already started returns [`CaptureError::InvalidState`].
    ///
    /// # Errors
    ///
    /// Returns [`CaptureError::Connection`] if the `PipeWire` thread cannot be
    /// started, or [`CaptureError::Stream`] if opening a stream fails.
    fn start(&mut self) -> Result<(), CaptureError>;

    /// Stop capturing audio and flush any in-flight buffers.
    ///
    /// It is safe to call `stop` multiple times; subsequent calls after the
    /// first are no-ops. The channel receiver remains valid after `stop`;
    /// remaining queued chunks can still be drained.
    ///
    /// # Errors
    ///
    /// Returns [`CaptureError::ThreadPanic`] if the `PipeWire` thread already
    /// exited abnormally.
    fn stop(&mut self) -> Result<(), CaptureError>;

    /// Return a reference to the receiving end of the audio chunk channel.
    ///
    /// Chunks arrive in real time. The caller is responsible for draining the
    /// channel to prevent back-pressure. The channel has a bounded internal
    /// buffer; if the receiver falls too far behind, older chunks may be
    /// dropped with a warning logged.
    ///
    /// The receiver is valid for the lifetime of the [`AudioSource`] instance.
    #[must_use]
    fn stream_receiver(&self) -> &crossbeam_channel::Receiver<AudioChunk>;
}
