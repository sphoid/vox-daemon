//! Mock [`AudioSource`] implementation for unit testing.
//!
//! [`MockAudioSource`] does not talk to `PipeWire` at all. It either plays back
//! a pre-loaded set of [`AudioChunk`]s or generates a synthetic sine-wave
//! signal at 16 kHz. This lets downstream crates and the daemon itself be
//! tested without a live `PipeWire` daemon.
//!
//! # Example
//!
//! ```rust
//! use std::time::Duration;
//! use vox_capture::mock::MockAudioSource;
//! use vox_capture::{AudioSource, StreamFilter, StreamRole};
//! use vox_capture::types::AudioChunk;
//!
//! let chunks = vec![
//!     AudioChunk::new(vec![0.0; 1600], Duration::from_millis(0), StreamRole::Microphone),
//!     AudioChunk::new(vec![0.0; 1600], Duration::from_millis(100), StreamRole::Application),
//! ];
//! let mut source = MockAudioSource::with_chunks(chunks);
//! source.start().unwrap();
//! let rx = source.stream_receiver();
//! let chunk = rx.recv_timeout(Duration::from_secs(1)).unwrap();
//! assert_eq!(chunk.role, StreamRole::Microphone);
//! ```

use std::time::Duration;

use crossbeam_channel::{Receiver, Sender, bounded};
use tracing::{debug, warn};

use crate::error::CaptureError;
use crate::source::AudioSource;
use crate::types::{AudioChunk, StreamFilter, StreamInfo, StreamRole};

/// The capacity of the mock's internal audio chunk channel.
const CHANNEL_CAPACITY: usize = 256;

/// State of the mock source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Stopped,
    Running,
}

/// A mock implementation of [`AudioSource`] for testing.
///
/// Construct via [`MockAudioSource::new`] for a silent source,
/// [`MockAudioSource::with_chunks`] to replay specific chunks, or
/// [`MockAudioSource::with_sine`] to generate a synthetic sine wave.
pub struct MockAudioSource {
    /// Pre-loaded chunks to send on `start()`.
    chunks: Vec<AudioChunk>,

    /// Fake stream list returned by `list_streams`.
    streams: Vec<StreamInfo>,

    state: State,

    sender: Sender<AudioChunk>,
    receiver: Receiver<AudioChunk>,
}

impl MockAudioSource {
    /// Create a silent mock source with no pre-loaded audio and default fake
    /// stream metadata.
    #[must_use]
    pub fn new() -> Self {
        let (sender, receiver) = bounded(CHANNEL_CAPACITY);
        Self {
            chunks: Vec::new(),
            streams: default_fake_streams(),
            state: State::Stopped,
            sender,
            receiver,
        }
    }

    /// Create a mock source that will push `chunks` into the channel as soon
    /// as [`start`][AudioSource::start] is called.
    #[must_use]
    pub fn with_chunks(chunks: Vec<AudioChunk>) -> Self {
        let (sender, receiver) = bounded(CHANNEL_CAPACITY);
        Self {
            chunks,
            streams: default_fake_streams(),
            state: State::Stopped,
            sender,
            receiver,
        }
    }

    /// Create a mock source that generates a sine wave at `frequency` Hz for
    /// `duration` on the given `role` stream.
    ///
    /// The generated audio is at 16 kHz mono f32 PCM.
    #[must_use]
    pub fn with_sine(frequency: f32, duration: Duration, role: StreamRole) -> Self {
        let sample_rate = 16_000_u32;
        #[allow(
            clippy::cast_precision_loss,
            clippy::cast_sign_loss,
            clippy::cast_possible_truncation
        )]
        let num_samples = (duration.as_secs_f64() * f64::from(sample_rate)) as usize;
        let samples: Vec<f32> = (0..num_samples)
            .map(|i| {
                #[allow(clippy::cast_precision_loss)]
                let t = i as f32 / sample_rate as f32;
                (2.0 * std::f32::consts::PI * frequency * t).sin()
            })
            .collect();

        let chunk = AudioChunk::new(samples, Duration::ZERO, role);
        Self::with_chunks(vec![chunk])
    }

    /// Override the fake stream list returned by `list_streams`.
    #[must_use]
    pub fn with_streams(mut self, streams: Vec<StreamInfo>) -> Self {
        self.streams = streams;
        self
    }
}

impl Default for MockAudioSource {
    fn default() -> Self {
        Self::new()
    }
}

impl AudioSource for MockAudioSource {
    fn list_streams(&mut self, filter: &StreamFilter) -> Result<Vec<StreamInfo>, CaptureError> {
        let result: Vec<StreamInfo> = self
            .streams
            .iter()
            .filter(|s| filter.matches(s))
            .cloned()
            .collect();
        debug!(count = result.len(), "mock list_streams");
        Ok(result)
    }

    fn start(&mut self) -> Result<(), CaptureError> {
        if self.state == State::Running {
            return Err(CaptureError::InvalidState("already started".to_owned()));
        }
        self.state = State::Running;
        debug!("mock audio source started");

        // Drain any previously queued chunks from a prior run.
        while self.receiver.try_recv().is_ok() {}

        // Push all pre-loaded chunks into the bounded channel.
        for chunk in &self.chunks {
            if self.sender.try_send(chunk.clone()).is_err() {
                warn!("mock channel full; dropping chunk at {:?}", chunk.timestamp);
            }
        }

        Ok(())
    }

    fn stop(&mut self) -> Result<(), CaptureError> {
        if self.state == State::Stopped {
            debug!("mock audio source stop called while already stopped — no-op");
            return Ok(());
        }
        self.state = State::Stopped;
        debug!("mock audio source stopped");
        Ok(())
    }

    fn stream_receiver(&self) -> &Receiver<AudioChunk> {
        &self.receiver
    }
}

/// Build a minimal fake stream list for the mock.
fn default_fake_streams() -> Vec<StreamInfo> {
    vec![
        StreamInfo {
            node_id: 1,
            name: "mock_microphone".to_owned(),
            application_name: None,
            media_class: Some("Audio/Source".to_owned()),
            suggested_role: Some(StreamRole::Microphone),
        },
        StreamInfo {
            node_id: 2,
            name: "mock_app_audio".to_owned(),
            application_name: Some("MockApp".to_owned()),
            media_class: Some("Stream/Input/Audio".to_owned()),
            suggested_role: Some(StreamRole::Application),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::AudioSource;

    #[test]
    fn list_streams_returns_all_with_empty_filter() {
        let mut source = MockAudioSource::new();
        let streams = source
            .list_streams(&StreamFilter::default())
            .expect("list_streams should succeed");
        assert_eq!(streams.len(), 2);
    }

    #[test]
    fn list_streams_filters_by_media_class() {
        let mut source = MockAudioSource::new();
        let filter = StreamFilter {
            media_class: Some("Audio/Source".to_owned()),
            ..Default::default()
        };
        let streams = source.list_streams(&filter).expect("list_streams ok");
        assert_eq!(streams.len(), 1);
        assert_eq!(streams[0].node_id, 1);
    }

    #[test]
    fn start_pushes_chunks_into_channel() {
        let chunk = AudioChunk::new(
            vec![0.0; 160],
            Duration::from_millis(0),
            StreamRole::Microphone,
        );
        let mut source = MockAudioSource::with_chunks(vec![chunk]);
        source.start().expect("start ok");

        let rx = source.stream_receiver();
        let received = rx
            .recv_timeout(Duration::from_millis(100))
            .expect("should receive chunk");
        assert_eq!(received.role, StreamRole::Microphone);
        assert_eq!(received.samples.len(), 160);
    }

    #[test]
    fn double_start_returns_invalid_state() {
        let mut source = MockAudioSource::new();
        source.start().expect("first start ok");
        let err = source.start().unwrap_err();
        assert!(matches!(err, CaptureError::InvalidState(_)));
    }

    #[test]
    fn stop_is_idempotent() {
        let mut source = MockAudioSource::new();
        source.start().expect("start ok");
        source.stop().expect("first stop ok");
        source.stop().expect("second stop should be a no-op");
    }

    #[test]
    fn sine_wave_mock_has_correct_length() {
        let duration = Duration::from_millis(100);
        let source = MockAudioSource::with_sine(440.0, duration, StreamRole::Application);
        assert_eq!(source.chunks.len(), 1);
        // 100 ms at 16 kHz = 1 600 samples
        assert_eq!(source.chunks[0].samples.len(), 1_600);
    }
}
