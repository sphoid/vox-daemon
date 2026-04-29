//! Application configuration types, deserialized from TOML.
//!
//! The config file lives at `$XDG_CONFIG_HOME/vox-daemon/config.toml`.

use serde::{Deserialize, Serialize};

use crate::error::ConfigError;
use crate::paths;

/// Top-level application configuration.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct AppConfig {
    /// Audio capture settings.
    #[serde(default)]
    pub audio: AudioConfig,

    /// Transcription settings.
    #[serde(default)]
    pub transcription: TranscriptionConfig,

    /// Summarization settings.
    #[serde(default)]
    pub summarization: SummarizationConfig,

    /// Storage settings.
    #[serde(default)]
    pub storage: StorageConfig,

    /// Notification settings.
    #[serde(default)]
    pub notifications: NotificationConfig,
}

impl AppConfig {
    /// Load the configuration from the default XDG config path.
    ///
    /// If the file does not exist, returns the default configuration.
    ///
    /// # Errors
    ///
    /// Returns `ConfigError` if the file exists but cannot be read or parsed.
    pub fn load() -> Result<Self, ConfigError> {
        let path = paths::config_dir().join("config.toml");
        if !path.exists() {
            tracing::info!("no config file found at {}, using defaults", path.display());
            return Ok(Self::default());
        }
        let contents = std::fs::read_to_string(&path)?;
        let config: Self = toml::from_str(&contents)?;
        Ok(config)
    }

    /// Save the configuration to the default XDG config path.
    ///
    /// Creates the config directory if it does not exist.
    ///
    /// # Errors
    ///
    /// Returns `ConfigError` if the file cannot be written or serialized.
    pub fn save(&self) -> Result<(), ConfigError> {
        let dir = paths::config_dir();
        std::fs::create_dir_all(&dir).map_err(ConfigError::ReadFile)?;
        let path = dir.join("config.toml");
        let contents = toml::to_string_pretty(self)?;
        std::fs::write(&path, contents).map_err(ConfigError::ReadFile)?;
        tracing::info!("config saved to {}", path.display());
        Ok(())
    }
}

/// Audio capture configuration.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AudioConfig {
    /// `PipeWire` source identifier for the microphone. `"auto"` for automatic selection.
    #[serde(default = "default_auto")]
    pub mic_source: String,

    /// `PipeWire` source identifier for the application audio. `"auto"` for automatic selection.
    #[serde(default = "default_auto")]
    pub app_source: String,
}

impl Default for AudioConfig {
    fn default() -> Self {
        Self {
            mic_source: "auto".to_owned(),
            app_source: "auto".to_owned(),
        }
    }
}

/// Transcription configuration.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TranscriptionConfig {
    /// Whisper model size.
    #[serde(default = "default_model")]
    pub model: String,

    /// Language code (e.g., `"en"`) or `"auto"` for auto-detection.
    #[serde(default = "default_language")]
    pub language: String,

    /// GPU backend preference.
    #[serde(default = "default_auto")]
    pub gpu_backend: String,

    /// Custom model path, or empty string to use the default cache directory.
    #[serde(default)]
    pub model_path: String,

    /// Diarization mode: `"none"` (merge streams, single speaker label) or
    /// `"embedding"` (ONNX speaker embeddings + clustering).
    #[serde(default = "default_diarization_mode")]
    pub diarization_mode: String,

    /// Custom path to an ONNX speaker embedding model.  Empty string uses
    /// the default auto-downloaded ECAPA-TDNN model.
    #[serde(default)]
    pub diarize_model_path: String,

    /// Cosine distance threshold for agglomerative clustering.
    /// Lower values produce more clusters (stricter speaker separation).
    #[serde(default = "default_diarize_threshold")]
    pub diarize_threshold: f64,

    /// Duration (seconds) of mic-only audio at session start to use as
    /// enrollment for identifying the local user's voice.
    #[serde(default = "default_enrollment_seconds")]
    pub enrollment_seconds: f64,

    /// Decoding strategy: `"greedy"` or `"beam_search"`.
    #[serde(default = "default_decoding_strategy")]
    pub decoding_strategy: String,

    /// Beam width for beam search decoding. Only used when `decoding_strategy` is `"beam_search"`.
    #[serde(default = "default_beam_size")]
    pub beam_size: i32,

    /// Number of candidates for greedy decoding. Only used when `decoding_strategy` is `"greedy"`.
    #[serde(default = "default_best_of")]
    pub best_of: i32,

    /// Initial prompt to condition the decoder. Include participant names, technical
    /// terms, and domain vocabulary to reduce misrecognition of similar-sounding words.
    #[serde(default)]
    pub initial_prompt: String,

    /// Initial decoding temperature. `0.0` is most confident.
    #[serde(default)]
    pub temperature: f32,

    /// Temperature increment for fallback retries. `0.0` disables fallback.
    #[serde(default = "default_temperature_inc")]
    pub temperature_inc: f32,

    /// Entropy threshold — segments with higher entropy trigger temperature fallback retries.
    #[serde(default = "default_entropy_thold")]
    pub entropy_thold: f32,

    /// Average log probability threshold — segments below this trigger fallback retries.
    #[serde(default = "default_logprob_thold")]
    pub logprob_thold: f32,

    /// No-speech probability threshold — segments above this are suppressed.
    #[serde(default = "default_no_speech_thold")]
    pub no_speech_thold: f32,

    /// Suppress blank tokens at the start of a segment.
    ///
    /// When enabled, Whisper will not output segments that begin with blank
    /// (silence) tokens, which can reduce hallucinated output on quiet audio.
    #[serde(default = "default_true")]
    pub suppress_blank: bool,

    /// Suppress non-speech tokens (e.g., `[Music]`, `(applause)`).
    ///
    /// When enabled, Whisper's internal non-speech token suppression list is
    /// applied during beam search / greedy decoding. This is one of the primary
    /// defences against bracketed hallucinations on low-confidence audio.
    #[serde(default = "default_true")]
    pub suppress_non_speech_tokens: bool,

    /// Energy gate in dBFS.
    ///
    /// If the root-mean-square energy of the audio buffer is below this
    /// threshold (in decibels relative to full scale), Whisper inference is
    /// skipped entirely and an empty result is returned.  This prevents
    /// hallucinations on near-silent buffers.
    ///
    /// Set to `f32::NEG_INFINITY` (or a very large negative value) to disable
    /// the gate.  Default is `-50.0` dBFS.
    #[serde(default = "default_energy_gate_dbfs")]
    pub min_rms_dbfs: f32,
}

impl Default for TranscriptionConfig {
    fn default() -> Self {
        Self {
            model: "small".to_owned(),
            language: "en".to_owned(),
            gpu_backend: "auto".to_owned(),
            model_path: String::new(),
            diarization_mode: "none".to_owned(),
            diarize_model_path: String::new(),
            diarize_threshold: 0.5,
            enrollment_seconds: 5.0,
            decoding_strategy: "beam_search".to_owned(),
            beam_size: 5,
            best_of: 5,
            initial_prompt: String::new(),
            temperature: 0.0,
            temperature_inc: 0.2,
            entropy_thold: 2.4,
            logprob_thold: -1.0,
            no_speech_thold: 0.6,
            suppress_blank: true,
            suppress_non_speech_tokens: true,
            min_rms_dbfs: -50.0,
        }
    }
}

/// Summarization configuration.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SummarizationConfig {
    /// Whether to automatically summarize after transcription completes.
    #[serde(default = "default_true")]
    pub auto_summarize: bool,

    /// LLM backend to use.
    #[serde(default = "default_backend")]
    pub backend: String,

    /// Ollama server URL.
    #[serde(default = "default_ollama_url")]
    pub ollama_url: String,

    /// Ollama model name.
    #[serde(default = "default_ollama_model")]
    pub ollama_model: String,

    /// OpenAI-compatible API URL.
    #[serde(default)]
    pub api_url: String,

    /// API key for OpenAI-compatible API.
    #[serde(default)]
    pub api_key: String,

    /// Model name for the OpenAI-compatible API.
    #[serde(default)]
    pub api_model: String,
}

impl Default for SummarizationConfig {
    fn default() -> Self {
        Self {
            auto_summarize: true,
            backend: "builtin".to_owned(),
            ollama_url: "http://localhost:11434".to_owned(),
            ollama_model: "qwen2.5:1.5b".to_owned(),
            api_url: String::new(),
            api_key: String::new(),
            api_model: String::new(),
        }
    }
}

/// Storage configuration.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StorageConfig {
    /// Custom data directory, or empty string for XDG default.
    #[serde(default)]
    pub data_dir: String,

    /// Whether to retain raw audio files after transcription.
    #[serde(default)]
    pub retain_audio: bool,

    /// Export format preference.
    #[serde(default = "default_export_format")]
    pub export_format: String,
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            data_dir: String::new(),
            retain_audio: false,
            export_format: "markdown".to_owned(),
        }
    }
}

/// Notification configuration.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NotificationConfig {
    /// Whether desktop notifications are enabled.
    #[serde(default = "default_true")]
    pub enabled: bool,

    /// Notify when recording starts.
    #[serde(default = "default_true")]
    pub on_record_start: bool,

    /// Notify when recording stops.
    #[serde(default = "default_true")]
    pub on_record_stop: bool,

    /// Notify when transcription is ready.
    #[serde(default = "default_true")]
    pub on_transcript_ready: bool,

    /// Notify when summary is generated.
    #[serde(default = "default_true")]
    pub on_summary_ready: bool,
}

impl Default for NotificationConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            on_record_start: true,
            on_record_stop: true,
            on_transcript_ready: true,
            on_summary_ready: true,
        }
    }
}

fn default_auto() -> String {
    "auto".to_owned()
}

fn default_model() -> String {
    "small".to_owned()
}

fn default_language() -> String {
    "en".to_owned()
}

fn default_true() -> bool {
    true
}

fn default_backend() -> String {
    "builtin".to_owned()
}

fn default_ollama_url() -> String {
    "http://localhost:11434".to_owned()
}

fn default_ollama_model() -> String {
    "qwen2.5:1.5b".to_owned()
}

fn default_export_format() -> String {
    "markdown".to_owned()
}

fn default_diarization_mode() -> String {
    "none".to_owned()
}

fn default_diarize_threshold() -> f64 {
    0.5
}

fn default_enrollment_seconds() -> f64 {
    5.0
}

fn default_decoding_strategy() -> String {
    "beam_search".to_owned()
}

fn default_beam_size() -> i32 {
    5
}

fn default_best_of() -> i32 {
    5
}

fn default_temperature_inc() -> f32 {
    0.2
}

fn default_entropy_thold() -> f32 {
    2.4
}

fn default_logprob_thold() -> f32 {
    -1.0
}

fn default_no_speech_thold() -> f32 {
    0.6
}

fn default_energy_gate_dbfs() -> f32 {
    -50.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config_roundtrip() {
        let config = AppConfig::default();
        let toml_str = toml::to_string_pretty(&config).expect("serialize");
        let parsed: AppConfig = toml::from_str(&toml_str).expect("parse");
        assert_eq!(config, parsed);
    }

    #[test]
    fn test_partial_config_uses_defaults() {
        let toml_str = r#"
[audio]
mic_source = "alsa_input.usb"
"#;
        let config: AppConfig = toml::from_str(toml_str).expect("parse");
        assert_eq!(config.audio.mic_source, "alsa_input.usb");
        assert_eq!(config.audio.app_source, "auto");
        assert_eq!(config.transcription.model, "small");
    }

    #[test]
    fn test_load_missing_file_returns_default() {
        // When no config file exists, load() returns defaults
        // This test works because the test environment has no XDG config dir set up
        let config = AppConfig::load().expect("should return default");
        assert_eq!(config, AppConfig::default());
    }

    #[test]
    fn test_save_and_load_roundtrip() {
        let dir = tempfile::tempdir().expect("create tempdir");
        let config_dir = dir.path().join("vox-daemon");
        std::fs::create_dir_all(&config_dir).expect("create config dir");
        let config_path = config_dir.join("config.toml");

        let mut config = AppConfig::default();
        config.audio.mic_source = "test_mic".to_owned();

        // Write directly to the temp path
        let contents = toml::to_string_pretty(&config).expect("serialize");
        std::fs::write(&config_path, &contents).expect("write");

        // Read back and verify
        let read_back = std::fs::read_to_string(&config_path).expect("read");
        let loaded: AppConfig = toml::from_str(&read_back).expect("parse");
        assert_eq!(config, loaded);
    }
}
