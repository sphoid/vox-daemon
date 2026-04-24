//! Settings data model that mirrors [`AppConfig`] with UI-friendly field
//! representations.
//!
//! [`SettingsModel`] is the editable in-memory representation of all user
//! preferences. It can be built from an [`AppConfig`] and converted back, which
//! makes it easy to drive both the iced UI and headless tests without coupling
//! directly to the TOML serialization types.

use vox_core::config::{
    AffineExportConfig, AppConfig, AudioConfig, ExportConfig, NotificationConfig, StorageConfig,
    SummarizationConfig, TranscriptionConfig,
};

// ──────────────────────────────────────────────────────────────────────────────
// Enums for strongly-typed UI selection lists
// ──────────────────────────────────────────────────────────────────────────────

/// Available Whisper model sizes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WhisperModel {
    /// ~75 MB — fastest, lowest accuracy.
    Tiny,
    /// ~142 MB — good balance for short utterances.
    Base,
    /// ~466 MB — recommended for most use cases.
    Small,
    /// ~1.5 GB — high accuracy.
    Medium,
    /// ~2.9 GB — best accuracy.
    Large,
}

impl WhisperModel {
    /// Returns all variants in ascending size order.
    #[must_use]
    pub fn all() -> &'static [Self] {
        &[
            Self::Tiny,
            Self::Base,
            Self::Small,
            Self::Medium,
            Self::Large,
        ]
    }

    /// Returns the string identifier used in the config file.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Tiny => "tiny",
            Self::Base => "base",
            Self::Small => "small",
            Self::Medium => "medium",
            Self::Large => "large",
        }
    }

    /// Parse from the config-file string identifier.
    ///
    /// Returns `None` for unrecognised values.
    #[must_use]
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "tiny" => Some(Self::Tiny),
            "base" => Some(Self::Base),
            "small" => Some(Self::Small),
            "medium" => Some(Self::Medium),
            "large" => Some(Self::Large),
            _ => None,
        }
    }
}

impl std::fmt::Display for WhisperModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Tiny => "Tiny (~75 MB)",
            Self::Base => "Base (~142 MB)",
            Self::Small => "Small (~466 MB)",
            Self::Medium => "Medium (~1.5 GB)",
            Self::Large => "Large (~2.9 GB)",
        })
    }
}

/// GPU backend preference for Whisper inference.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GpuBackend {
    /// Let the runtime pick the best available backend.
    Auto,
    /// NVIDIA CUDA.
    Cuda,
    /// AMD ROCm/hipBLAS.
    Rocm,
    /// Force CPU-only inference.
    Cpu,
}

impl GpuBackend {
    /// Returns all variants.
    #[must_use]
    pub fn all() -> &'static [Self] {
        &[Self::Auto, Self::Cuda, Self::Rocm, Self::Cpu]
    }

    /// Returns the string identifier used in the config file.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Cuda => "cuda",
            Self::Rocm => "rocm",
            Self::Cpu => "cpu",
        }
    }

    /// Parse from the config-file string identifier.
    ///
    /// Returns `None` for unrecognised values.
    #[must_use]
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "auto" => Some(Self::Auto),
            "cuda" => Some(Self::Cuda),
            "rocm" => Some(Self::Rocm),
            "cpu" => Some(Self::Cpu),
            _ => None,
        }
    }
}

impl std::fmt::Display for GpuBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Auto => "Auto (recommended)",
            Self::Cuda => "CUDA (NVIDIA)",
            Self::Rocm => "ROCm (AMD)",
            Self::Cpu => "CPU only",
        })
    }
}

/// LLM summarization backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SummarizationBackend {
    /// Use the built-in bundled model (smallest, offline).
    Builtin,
    /// Use a local Ollama server.
    Ollama,
    /// Use an OpenAI-compatible HTTP API (cloud or self-hosted).
    OpenAiCompatible,
}

impl SummarizationBackend {
    /// Returns all variants.
    #[must_use]
    pub fn all() -> &'static [Self] {
        &[Self::Builtin, Self::Ollama, Self::OpenAiCompatible]
    }

    /// Returns the string identifier used in the config file.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Builtin => "builtin",
            Self::Ollama => "ollama",
            Self::OpenAiCompatible => "openai_compatible",
        }
    }

    /// Parse from the config-file string identifier.
    ///
    /// Returns `None` for unrecognised values.
    #[must_use]
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "builtin" => Some(Self::Builtin),
            "ollama" => Some(Self::Ollama),
            "openai_compatible" => Some(Self::OpenAiCompatible),
            _ => None,
        }
    }
}

impl std::fmt::Display for SummarizationBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Builtin => "Built-in (local, offline)",
            Self::Ollama => "Ollama (local server)",
            Self::OpenAiCompatible => "OpenAI-compatible API",
        })
    }
}

/// Export format preference.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportFormat {
    /// Markdown (`.md`) — human-readable, default.
    Markdown,
    /// Raw JSON — preserves all metadata.
    Json,
    /// Plain text (`.txt`) — no formatting.
    Text,
}

impl ExportFormat {
    /// Returns all variants.
    #[must_use]
    pub fn all() -> &'static [Self] {
        &[Self::Markdown, Self::Json, Self::Text]
    }

    /// Returns the string identifier used in the config file.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Markdown => "markdown",
            Self::Json => "json",
            Self::Text => "text",
        }
    }

    /// Returns the file extension (without leading dot).
    #[must_use]
    pub fn extension(self) -> &'static str {
        match self {
            Self::Markdown => "md",
            Self::Json => "json",
            Self::Text => "txt",
        }
    }

    /// Parse from the config-file string identifier.
    ///
    /// Returns `None` for unrecognised values.
    #[must_use]
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "markdown" => Some(Self::Markdown),
            "json" => Some(Self::Json),
            "text" => Some(Self::Text),
            _ => None,
        }
    }
}

impl std::fmt::Display for ExportFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Markdown => "Markdown (.md)",
            Self::Json => "JSON (.json)",
            Self::Text => "Text (.txt)",
        })
    }
}

/// What content to include in an export.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ExportContent {
    /// Only the transcript.
    Transcript,
    /// Only the AI summary.
    Summary,
    /// Both transcript and AI summary (default).
    #[default]
    TranscriptAndSummary,
}

impl ExportContent {
    /// Returns all variants.
    #[must_use]
    pub fn all() -> &'static [Self] {
        &[Self::TranscriptAndSummary, Self::Transcript, Self::Summary]
    }
}

impl std::fmt::Display for ExportContent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Transcript => "Transcript",
            Self::Summary => "AI Summary",
            Self::TranscriptAndSummary => "Transcript + AI Summary",
        })
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// Main settings model
// ──────────────────────────────────────────────────────────────────────────────

/// Audio source settings, mirroring [`AudioConfig`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioSettings {
    /// `PipeWire` source identifier for the microphone.
    ///
    /// `"auto"` selects the system default.
    pub mic_source: String,

    /// `PipeWire` source identifier for the application audio.
    ///
    /// `"auto"` selects automatically based on active streams.
    pub app_source: String,
}

/// Transcription settings, mirroring [`TranscriptionConfig`].
#[derive(Debug, Clone, PartialEq)]
pub struct TranscriptionSettings {
    /// Selected Whisper model size.
    pub model: WhisperModel,

    /// Language code (e.g. `"en"`) or `"auto"` for language detection.
    pub language: String,

    /// GPU backend preference.
    pub gpu_backend: GpuBackend,

    /// Optional custom path to a GGML model file.
    ///
    /// Empty string means "use the XDG cache default".
    pub model_path: String,
}

/// Summarization settings, mirroring [`SummarizationConfig`].
#[derive(Debug, Clone, PartialEq)]
pub struct SummarizationSettings {
    /// Whether to summarize automatically when transcription finishes.
    pub auto_summarize: bool,

    /// Active LLM backend.
    pub backend: SummarizationBackend,

    /// Ollama server URL (used when `backend == Ollama`).
    pub ollama_url: String,

    /// Ollama model name (used when `backend == Ollama`).
    pub ollama_model: String,

    /// OpenAI-compatible API base URL.
    pub api_url: String,

    /// API key for the OpenAI-compatible endpoint.
    ///
    /// Stored in plaintext in `config.toml`; users should `chmod 600` the
    /// config file.
    pub api_key: String,

    /// Model identifier for the OpenAI-compatible API.
    pub api_model: String,
}

/// Storage settings, mirroring [`StorageConfig`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StorageSettings {
    /// Custom data directory path.
    ///
    /// Empty string means "use the XDG default (`$XDG_DATA_HOME/vox-daemon/`)".
    pub data_dir: String,

    /// Whether to retain raw audio files after transcription.
    pub retain_audio: bool,

    /// Preferred export format.
    pub export_format: ExportFormat,
}

/// Export-target settings, mirroring [`ExportConfig`].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ExportSettings {
    /// `AFFiNE` export plugin settings.
    pub affine: AffineSettings,
}

/// `AFFiNE` export plugin settings, mirroring [`AffineExportConfig`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AffineSettings {
    /// Whether the `AFFiNE` target is enabled.
    pub enabled: bool,
    /// Base URL of the `AFFiNE` server (cloud or self-hosted).
    pub base_url: String,
    /// Personal access token (required for cloud).
    ///
    /// Stored in plaintext in `config.toml`; users should `chmod 600` the
    /// config file.
    pub api_token: String,
    /// Login email (self-hosted only).
    pub email: String,
    /// Login password (self-hosted only). Plaintext — see `api_token`.
    pub password: String,
    /// Optional default workspace id for the Send-to picker.
    pub default_workspace_id: String,
    /// Optional default parent-doc id for the Send-to picker.
    pub default_parent_id: String,
}

impl Default for AffineSettings {
    fn default() -> Self {
        Self::from_config(&AffineExportConfig::default())
    }
}

impl AffineSettings {
    /// Construct from an [`AffineExportConfig`].
    #[must_use]
    pub fn from_config(cfg: &AffineExportConfig) -> Self {
        Self {
            enabled: cfg.enabled,
            base_url: cfg.base_url.clone(),
            api_token: cfg.api_token.clone(),
            email: cfg.email.clone(),
            password: cfg.password.clone(),
            default_workspace_id: cfg.default_workspace_id.clone(),
            default_parent_id: cfg.default_parent_id.clone(),
        }
    }

    /// Convert back to an [`AffineExportConfig`].
    #[must_use]
    pub fn to_config(&self) -> AffineExportConfig {
        AffineExportConfig {
            enabled: self.enabled,
            base_url: self.base_url.clone(),
            api_token: self.api_token.clone(),
            email: self.email.clone(),
            password: self.password.clone(),
            default_workspace_id: self.default_workspace_id.clone(),
            default_parent_id: self.default_parent_id.clone(),
        }
    }
}

/// Notification settings, mirroring [`NotificationConfig`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(clippy::struct_excessive_bools)]
pub struct NotificationSettings {
    /// Master toggle — when `false`, no notifications are sent.
    pub enabled: bool,

    /// Send a notification when a recording starts.
    pub on_record_start: bool,

    /// Send a notification when a recording stops.
    pub on_record_stop: bool,

    /// Send a notification when transcription is ready.
    pub on_transcript_ready: bool,

    /// Send a notification when the AI summary is ready.
    pub on_summary_ready: bool,
}

/// The complete, UI-friendly settings model.
///
/// This is the in-memory representation used by the settings window. It can
/// be constructed from an [`AppConfig`] with [`SettingsModel::from_config`]
/// and converted back with [`SettingsModel::to_config`].
#[derive(Debug, Clone, PartialEq)]
pub struct SettingsModel {
    /// Audio source settings.
    pub audio: AudioSettings,
    /// Transcription settings.
    pub transcription: TranscriptionSettings,
    /// Summarization settings.
    pub summarization: SummarizationSettings,
    /// Storage settings.
    pub storage: StorageSettings,
    /// Notification settings.
    pub notifications: NotificationSettings,
    /// Export-target (Affine, etc.) settings.
    pub export: ExportSettings,
}

impl SettingsModel {
    /// Construct a [`SettingsModel`] from an [`AppConfig`].
    ///
    /// Unknown string values for enum fields fall back to sensible defaults
    /// (logged at `warn` level via `tracing`).
    #[must_use]
    pub fn from_config(config: &AppConfig) -> Self {
        let model = WhisperModel::from_str(&config.transcription.model).unwrap_or_else(|| {
            tracing::warn!(
                "unknown whisper model '{}', defaulting to Base",
                config.transcription.model
            );
            WhisperModel::Base
        });

        let gpu_backend =
            GpuBackend::from_str(&config.transcription.gpu_backend).unwrap_or_else(|| {
                tracing::warn!(
                    "unknown gpu_backend '{}', defaulting to Auto",
                    config.transcription.gpu_backend
                );
                GpuBackend::Auto
            });

        let backend =
            SummarizationBackend::from_str(&config.summarization.backend).unwrap_or_else(|| {
                tracing::warn!(
                    "unknown summarization backend '{}', defaulting to Builtin",
                    config.summarization.backend
                );
                SummarizationBackend::Builtin
            });

        let export_format =
            ExportFormat::from_str(&config.storage.export_format).unwrap_or_else(|| {
                tracing::warn!(
                    "unknown export_format '{}', defaulting to Markdown",
                    config.storage.export_format
                );
                ExportFormat::Markdown
            });

        Self {
            audio: AudioSettings {
                mic_source: config.audio.mic_source.clone(),
                app_source: config.audio.app_source.clone(),
            },
            transcription: TranscriptionSettings {
                model,
                language: config.transcription.language.clone(),
                gpu_backend,
                model_path: config.transcription.model_path.clone(),
            },
            summarization: SummarizationSettings {
                auto_summarize: config.summarization.auto_summarize,
                backend,
                ollama_url: config.summarization.ollama_url.clone(),
                ollama_model: config.summarization.ollama_model.clone(),
                api_url: config.summarization.api_url.clone(),
                api_key: config.summarization.api_key.clone(),
                api_model: config.summarization.api_model.clone(),
            },
            storage: StorageSettings {
                data_dir: config.storage.data_dir.clone(),
                retain_audio: config.storage.retain_audio,
                export_format,
            },
            notifications: NotificationSettings {
                enabled: config.notifications.enabled,
                on_record_start: config.notifications.on_record_start,
                on_record_stop: config.notifications.on_record_stop,
                on_transcript_ready: config.notifications.on_transcript_ready,
                on_summary_ready: config.notifications.on_summary_ready,
            },
            export: ExportSettings {
                affine: AffineSettings::from_config(&config.export.affine),
            },
        }
    }

    /// Convert this [`SettingsModel`] back into an [`AppConfig`].
    ///
    /// The resulting config is suitable for serialization with
    /// [`AppConfig::save`].
    #[must_use]
    pub fn to_config(&self) -> AppConfig {
        AppConfig {
            audio: AudioConfig {
                mic_source: self.audio.mic_source.clone(),
                app_source: self.audio.app_source.clone(),
            },
            transcription: TranscriptionConfig {
                model: self.transcription.model.as_str().to_owned(),
                language: self.transcription.language.clone(),
                gpu_backend: self.transcription.gpu_backend.as_str().to_owned(),
                model_path: self.transcription.model_path.clone(),
                ..TranscriptionConfig::default()
            },
            summarization: SummarizationConfig {
                auto_summarize: self.summarization.auto_summarize,
                backend: self.summarization.backend.as_str().to_owned(),
                ollama_url: self.summarization.ollama_url.clone(),
                ollama_model: self.summarization.ollama_model.clone(),
                api_url: self.summarization.api_url.clone(),
                api_key: self.summarization.api_key.clone(),
                api_model: self.summarization.api_model.clone(),
            },
            storage: StorageConfig {
                data_dir: self.storage.data_dir.clone(),
                retain_audio: self.storage.retain_audio,
                export_format: self.storage.export_format.as_str().to_owned(),
            },
            notifications: NotificationConfig {
                enabled: self.notifications.enabled,
                on_record_start: self.notifications.on_record_start,
                on_record_stop: self.notifications.on_record_stop,
                on_transcript_ready: self.notifications.on_transcript_ready,
                on_summary_ready: self.notifications.on_summary_ready,
            },
            export: ExportConfig {
                affine: self.export.affine.to_config(),
            },
        }
    }
}

impl Default for SettingsModel {
    fn default() -> Self {
        Self::from_config(&AppConfig::default())
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// Tests
// ──────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_settings_model_round_trips() {
        let config = AppConfig::default();
        let model = SettingsModel::from_config(&config);
        let back = model.to_config();
        assert_eq!(config, back);
    }

    #[test]
    fn test_whisper_model_enum_covers_all_strings() {
        for variant in WhisperModel::all() {
            let s = variant.as_str();
            assert_eq!(
                WhisperModel::from_str(s),
                Some(*variant),
                "from_str should round-trip for '{s}'"
            );
        }
    }

    #[test]
    fn test_gpu_backend_enum_covers_all_strings() {
        for variant in GpuBackend::all() {
            let s = variant.as_str();
            assert_eq!(
                GpuBackend::from_str(s),
                Some(*variant),
                "from_str should round-trip for '{s}'"
            );
        }
    }

    #[test]
    fn test_summarization_backend_enum_covers_all_strings() {
        for variant in SummarizationBackend::all() {
            let s = variant.as_str();
            assert_eq!(
                SummarizationBackend::from_str(s),
                Some(*variant),
                "from_str should round-trip for '{s}'"
            );
        }
    }

    #[test]
    fn test_export_format_enum_covers_all_strings() {
        for variant in ExportFormat::all() {
            let s = variant.as_str();
            assert_eq!(
                ExportFormat::from_str(s),
                Some(*variant),
                "from_str should round-trip for '{s}'"
            );
        }
    }

    #[test]
    fn test_unknown_whisper_model_falls_back_to_base() {
        let mut config = AppConfig::default();
        config.transcription.model = "unknown-giant".to_owned();
        let model = SettingsModel::from_config(&config);
        assert_eq!(model.transcription.model, WhisperModel::Base);
    }

    #[test]
    fn test_settings_modification_round_trips() {
        let mut model = SettingsModel::default();
        model.transcription.model = WhisperModel::Large;
        model.transcription.language = "de".to_owned();
        model.summarization.backend = SummarizationBackend::Ollama;
        model.summarization.ollama_url = "http://192.168.1.5:11434".to_owned();
        model.storage.retain_audio = true;
        model.storage.export_format = ExportFormat::Json;
        model.notifications.on_summary_ready = false;

        let config = model.to_config();
        assert_eq!(config.transcription.model, "large");
        assert_eq!(config.transcription.language, "de");
        assert_eq!(config.summarization.backend, "ollama");
        assert_eq!(config.summarization.ollama_url, "http://192.168.1.5:11434");
        assert!(config.storage.retain_audio);
        assert_eq!(config.storage.export_format, "json");
        assert!(!config.notifications.on_summary_ready);

        // Reconstruct from config and verify equality with the original model.
        let model2 = SettingsModel::from_config(&config);
        assert_eq!(model, model2);
    }

    #[test]
    fn test_display_implementations_are_non_empty() {
        for variant in WhisperModel::all() {
            assert!(!variant.to_string().is_empty());
        }
        for variant in GpuBackend::all() {
            assert!(!variant.to_string().is_empty());
        }
        for variant in SummarizationBackend::all() {
            assert!(!variant.to_string().is_empty());
        }
        for variant in ExportFormat::all() {
            assert!(!variant.to_string().is_empty());
        }
    }
}
