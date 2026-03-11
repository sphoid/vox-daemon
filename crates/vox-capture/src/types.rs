//! Shared data types for the `vox-capture` crate.
//!
//! These types cross the boundary between the `PipeWire` thread and the async
//! Tokio context via [`crossbeam_channel`].

use std::time::Duration;

/// The role a captured audio stream plays in the session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StreamRole {
    /// The local user's microphone input.
    Microphone,
    /// The remote application's audio output (e.g., from a video call app).
    Application,
}

impl std::fmt::Display for StreamRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Microphone => write!(f, "microphone"),
            Self::Application => write!(f, "application"),
        }
    }
}

/// A chunk of captured and resampled PCM audio data.
///
/// All samples are **16 kHz, mono, f32** regardless of the original hardware
/// format. The resampling happens inside the capture layer before the chunk
/// is sent through the channel.
#[derive(Debug, Clone)]
pub struct AudioChunk {
    /// Linear PCM samples at 16 kHz mono f32.
    pub samples: Vec<f32>,

    /// Time elapsed since the recording session started.
    ///
    /// This is a relative offset, not a wall-clock timestamp, so that chunks
    /// from mic and app streams can be aligned by the downstream transcriber.
    pub timestamp: Duration,

    /// Which audio stream this chunk came from.
    pub role: StreamRole,

    /// Always 16 000 after resampling. Included for downstream convenience
    /// so consumers do not need out-of-band knowledge of the format.
    pub sample_rate: u32,
}

impl AudioChunk {
    /// Construct a new [`AudioChunk`].
    ///
    /// `sample_rate` should always be `16_000`; it is included in the struct
    /// so consumers do not need to hard-code that assumption.
    #[must_use]
    pub fn new(samples: Vec<f32>, timestamp: Duration, role: StreamRole) -> Self {
        Self {
            samples,
            timestamp,
            role,
            sample_rate: 16_000,
        }
    }

    /// Returns the duration represented by this chunk in seconds.
    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub fn duration_secs(&self) -> f64 {
        self.samples.len() as f64 / f64::from(self.sample_rate)
    }
}

/// Metadata describing a single `PipeWire` audio node discovered during
/// enumeration.
#[derive(Debug, Clone)]
pub struct StreamInfo {
    /// `PipeWire` node ID.
    pub node_id: u32,

    /// Human-readable node name (e.g., `"alsa_input.pci-0000_00_1f.3.analog-stereo"`).
    pub name: String,

    /// Application/process name that owns this node, if available.
    pub application_name: Option<String>,

    /// `PipeWire` media class (e.g., `"Audio/Source"`, `"Audio/Sink"`).
    pub media_class: Option<String>,

    /// Suggested role for this stream based on its media class.
    pub suggested_role: Option<StreamRole>,
}

impl StreamInfo {
    /// Returns `true` if this node looks like a microphone source.
    ///
    /// Heuristic: media class contains `"Source"` or the name contains `"input"`.
    #[must_use]
    pub fn is_source(&self) -> bool {
        self.media_class
            .as_deref()
            .is_some_and(|c| c.contains("Source"))
            || self.name.to_ascii_lowercase().contains("input")
    }

    /// Returns `true` if this node looks like an application audio sink.
    ///
    /// Heuristic: media class is `"Stream/Input/Audio"` (sink input in
    /// `PipeWire` terminology is a stream consuming audio from an application).
    #[must_use]
    pub fn is_app_sink(&self) -> bool {
        self.media_class
            .as_deref()
            .is_some_and(|c| c.contains("Stream/Input/Audio"))
    }
}

/// Filter criteria for [`AudioSource::list_streams`].
///
/// All provided fields are `ANDed` together. Leave a field as `None` to skip
/// that criterion.
#[derive(Debug, Clone, Default)]
pub struct StreamFilter {
    /// Match streams whose `application_name` contains this substring
    /// (case-insensitive).
    pub application_name: Option<String>,

    /// Match streams whose `media_class` contains this substring
    /// (case-insensitive).
    pub media_class: Option<String>,

    /// Match a specific `PipeWire` node ID exactly.
    pub node_id: Option<u32>,
}

impl StreamFilter {
    /// Returns `true` if `info` satisfies every criterion in this filter.
    #[must_use]
    pub fn matches(&self, info: &StreamInfo) -> bool {
        if let Some(ref app) = self.application_name {
            let app_lower = app.to_ascii_lowercase();
            let matches = info
                .application_name
                .as_deref()
                .is_some_and(|n| n.to_ascii_lowercase().contains(&app_lower));
            if !matches {
                return false;
            }
        }

        if let Some(ref cls) = self.media_class {
            let cls_lower = cls.to_ascii_lowercase();
            let matches = info
                .media_class
                .as_deref()
                .is_some_and(|c| c.to_ascii_lowercase().contains(&cls_lower));
            if !matches {
                return false;
            }
        }

        if let Some(id) = self.node_id {
            if info.node_id != id {
                return false;
            }
        }

        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audio_chunk_duration() {
        let chunk = AudioChunk::new(
            vec![0.0_f32; 16_000],
            Duration::from_secs(0),
            StreamRole::Microphone,
        );
        assert!((chunk.duration_secs() - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn stream_filter_empty_matches_all() {
        let filter = StreamFilter::default();
        let info = StreamInfo {
            node_id: 1,
            name: "test".to_owned(),
            application_name: Some("zoom".to_owned()),
            media_class: Some("Audio/Source".to_owned()),
            suggested_role: None,
        };
        assert!(filter.matches(&info));
    }

    #[test]
    fn stream_filter_application_name_case_insensitive() {
        let filter = StreamFilter {
            application_name: Some("ZOOM".to_owned()),
            ..Default::default()
        };
        let info = StreamInfo {
            node_id: 2,
            name: "zoom-audio".to_owned(),
            application_name: Some("Zoom".to_owned()),
            media_class: None,
            suggested_role: None,
        };
        assert!(filter.matches(&info));
    }

    #[test]
    fn stream_filter_node_id_mismatch() {
        let filter = StreamFilter {
            node_id: Some(99),
            ..Default::default()
        };
        let info = StreamInfo {
            node_id: 1,
            name: "mic".to_owned(),
            application_name: None,
            media_class: None,
            suggested_role: None,
        };
        assert!(!filter.matches(&info));
    }

    #[test]
    fn stream_info_is_source() {
        let info = StreamInfo {
            node_id: 3,
            name: "alsa_input.pci".to_owned(),
            application_name: None,
            media_class: Some("Audio/Source".to_owned()),
            suggested_role: None,
        };
        assert!(info.is_source());
    }

    #[test]
    fn stream_info_is_app_sink() {
        let info = StreamInfo {
            node_id: 4,
            name: "pw-stream".to_owned(),
            application_name: Some("zoom".to_owned()),
            media_class: Some("Stream/Input/Audio".to_owned()),
            suggested_role: None,
        };
        assert!(info.is_app_sink());
    }
}
