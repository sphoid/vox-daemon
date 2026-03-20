//! Main iced application — settings window and transcript browser.
//!
//! This module is only compiled when the `ui` feature is enabled.
//!
//! # Entry point
//!
//! Call [`run`] to launch the window. It blocks until the window is closed.
//!
//! # Architecture
//!
//! The application follows iced 0.14's functional pattern:
//! - [`VoxAppState`] — all mutable state
//! - [`Message`] — all events that mutate state
//! - [`update`] — state transitions
//! - [`view`] — widget tree derived from state
//!
//! Two pages are available, selected via [`Page`]:
//! - [`Page::Settings`] — settings form backed by [`SettingsModel`]
//! - [`Page::Browser`] — transcript list + detail viewer with search

use iced::widget::rule;
use iced::widget::{
    Column, button, column, container, pick_list, row, scrollable, text, text_input, toggler,
};
use iced::{Element, Fill, Font, Length, Size, Task, Theme};
use tracing::{error, info, warn};
use uuid::Uuid;
use vox_core::config::AppConfig;
use vox_core::session::{Session, Summary};
use vox_storage::store::{JsonFileStore, SessionStore};

use crate::browser::{SessionListEntry, build_session_list};
use crate::search::search_transcripts;
use crate::settings::{
    ExportFormat, GpuBackend, SettingsModel, SummarizationBackend, WhisperModel,
};
use crate::theme as vox_theme;

// ──────────────────────────────────────────────────────────────────────────────
// AudioSourceOption
// ──────────────────────────────────────────────────────────────────────────────

/// A selectable audio source entry for the mic/app source fields.
///
/// Wraps a PipeWire node ID (or the sentinel `"auto"`) with a human-readable
/// display name suitable for showing in the UI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioSourceOption {
    /// PipeWire node ID as a string, or `"auto"` for the system default.
    pub node_id: String,
    /// Human-readable label shown in the dropdown.
    pub display_name: String,
}

impl std::fmt::Display for AudioSourceOption {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.display_name)
    }
}

impl AudioSourceOption {
    /// The sentinel option that tells the daemon to pick the best source
    /// automatically.
    #[must_use]
    pub fn auto() -> Self {
        Self {
            node_id: "auto".to_owned(),
            display_name: "Auto (system default)".to_owned(),
        }
    }

    /// Build a list containing only the `auto` option.
    ///
    /// Used as a fallback when PipeWire enumeration is unavailable.
    #[must_use]
    pub fn fallback_list() -> Vec<Self> {
        vec![Self::auto()]
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// Page enum
// ──────────────────────────────────────────────────────────────────────────────

/// The two top-level pages of the application.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Page {
    /// Settings window (audio, transcription, summarization, storage, notifications, about).
    Settings,
    /// Transcript browser (list, search, detail view).
    Browser,
}

// ──────────────────────────────────────────────────────────────────────────────
// Message enum
// ──────────────────────────────────────────────────────────────────────────────

/// All events that can mutate the application state.
#[derive(Debug, Clone)]
pub enum Message {
    // ── Navigation ──────────────────────────────────────────────────────────
    /// Switch to a different top-level page.
    NavigateTo(Page),

    // ── Settings: Audio ─────────────────────────────────────────────────────
    /// The mic source dropdown selection changed.
    MicSourceChanged(AudioSourceOption),
    /// The app source dropdown selection changed.
    AppSourceChanged(AudioSourceOption),

    // ── Settings: Transcription ──────────────────────────────────────────────
    /// The model pick-list changed.
    ModelChanged(WhisperModel),
    /// The language text field changed.
    LanguageChanged(String),
    /// The GPU backend pick-list changed.
    GpuBackendChanged(GpuBackend),
    /// The custom model path text field changed.
    ModelPathChanged(String),

    // ── Settings: Summarization ──────────────────────────────────────────────
    /// The auto-summarize toggle changed.
    AutoSummarizeToggled(bool),
    /// The backend pick-list changed.
    BackendChanged(SummarizationBackend),
    /// The Ollama URL text field changed.
    OllamaUrlChanged(String),
    /// The Ollama model name text field changed.
    OllamaModelChanged(String),
    /// The API URL text field changed.
    ApiUrlChanged(String),
    /// The API key text field changed.
    ApiKeyChanged(String),
    /// The API model text field changed.
    ApiModelChanged(String),

    // ── Settings: Storage ────────────────────────────────────────────────────
    /// The data directory text field changed.
    DataDirChanged(String),
    /// The retain-audio toggle changed.
    RetainAudioToggled(bool),
    /// The export format pick-list changed.
    ExportFormatChanged(ExportFormat),

    // ── Settings: Notifications ──────────────────────────────────────────────
    /// Master notifications toggle changed.
    NotificationsEnabledToggled(bool),
    /// "On record start" toggle changed.
    OnRecordStartToggled(bool),
    /// "On record stop" toggle changed.
    OnRecordStopToggled(bool),
    /// "On transcript ready" toggle changed.
    OnTranscriptReadyToggled(bool),
    /// "On summary ready" toggle changed.
    OnSummaryReadyToggled(bool),

    // ── Settings: persistence ────────────────────────────────────────────────
    /// Persist the current settings to disk.
    SaveSettings,
    /// Settings were saved (carries Ok/Err indication via bool).
    SettingsSaved(bool),

    // ── Browser ──────────────────────────────────────────────────────────────
    /// The search query text field changed.
    SearchQueryChanged(String),
    /// The user selected a session from the list.
    SessionSelected(Uuid),
    /// A full session was loaded from disk.
    SessionLoaded(Box<Session>),
    /// The user requested export of the selected session to Markdown.
    ExportSelectedSession,
    /// Export result (the absolute path of the written `.md` file, or an error message).
    ExportResult(Result<String, String>),
    /// The user requested deletion of the selected session.
    DeleteSelectedSession,
    /// Deletion completed (carries success flag).
    DeleteResult(bool),
    /// The user edited a speaker friendly name.
    SpeakerNameEdited {
        /// Index into the current session's `speakers` vec.
        speaker_index: usize,
        /// The new friendly name.
        new_name: String,
    },
    /// Sessions list was refreshed from disk.
    SessionsLoaded(Vec<Session>),
    /// Speaker names were persisted to disk (carries success flag).
    SpeakerNamesSaved(bool),

    // ── Browser: summarization ───────────────────────────────────────────
    /// The user requested manual AI summary generation for the selected session.
    GenerateSummary,
    /// Summary generation completed (carries the summary or an error message).
    SummaryGenerated(Result<Box<Summary>, String>),
    /// The summary was persisted to disk after generation (carries success flag).
    SummarySaved(bool),
}

// ──────────────────────────────────────────────────────────────────────────────
// Application state
// ──────────────────────────────────────────────────────────────────────────────

/// All mutable state for the Vox Daemon GUI application.
pub struct VoxAppState {
    /// Currently displayed page.
    pub page: Page,

    /// Editable settings model.
    pub settings: SettingsModel,

    /// Whether the last settings save succeeded (for status feedback).
    pub settings_save_status: Option<bool>,

    /// Available microphone sources for the audio section pick-list.
    ///
    /// Always contains at least the `"auto"` sentinel option.
    pub available_mic_sources: Vec<AudioSourceOption>,

    /// Available application audio sources for the audio section pick-list.
    ///
    /// Always contains at least the `"auto"` sentinel option.
    pub available_app_sources: Vec<AudioSourceOption>,

    // ── Browser state ────────────────────────────────────────────────────────
    /// All sessions available in the store (lightweight list entries).
    pub session_list: Vec<SessionListEntry>,

    /// All full sessions loaded for search purposes.
    pub all_sessions: Vec<Session>,

    /// Current search query string.
    pub search_query: String,

    /// The UUID of the session currently selected in the list.
    pub selected_session_id: Option<Uuid>,

    /// The full session data for the currently selected session.
    pub selected_session: Option<Session>,

    /// Status message shown in the browser (e.g. "Exported!" or an error).
    pub browser_status: Option<String>,

    /// Whether a summary generation is currently in progress.
    pub summarizing: bool,
}

impl VoxAppState {
    /// Construct the initial state by loading the config and session list from
    /// disk.
    ///
    /// Errors are logged and defaults used in their place so the window always
    /// opens even when storage is corrupt.
    fn new() -> Self {
        let config = AppConfig::load().unwrap_or_else(|e| {
            warn!("failed to load config, using defaults: {e}");
            AppConfig::default()
        });
        let settings = SettingsModel::from_config(&config);

        // Load sessions in the constructor; later refreshes use the
        // SessionsLoaded message.
        let all_sessions = load_sessions_from_store(&config.storage.data_dir);
        let session_list = build_session_list(&all_sessions);

        // Enumerate real PipeWire sources (when the `pw` feature is enabled)
        // and split them into mic-type and app-type lists.
        let all_streams = enumerate_pipewire_sources();
        let mut available_mic_sources = vec![AudioSourceOption::auto()];
        let mut available_app_sources = vec![AudioSourceOption::auto()];
        for stream in &all_streams {
            let opt = AudioSourceOption {
                node_id: stream.node_id.to_string(),
                display_name: stream
                    .description
                    .clone()
                    .unwrap_or_else(|| stream.name.clone()),
            };
            if stream.is_source() {
                available_mic_sources.push(opt);
            } else {
                available_app_sources.push(opt);
            }
        }

        Self {
            page: Page::Browser,
            settings,
            settings_save_status: None,
            available_mic_sources,
            available_app_sources,
            session_list,
            all_sessions,
            search_query: String::new(),
            selected_session_id: None,
            selected_session: None,
            browser_status: None,
            summarizing: false,
        }
    }

    /// Returns the list entries that should be visible given the current search
    /// query.
    ///
    /// When the query is empty the full list is returned. Otherwise only
    /// sessions that contain at least one matching segment are shown.
    #[must_use]
    pub fn visible_sessions(&self) -> Vec<&SessionListEntry> {
        if self.search_query.is_empty() {
            return self.session_list.iter().collect();
        }
        let results = search_transcripts(&self.all_sessions, &self.search_query);
        let result_ids: std::collections::HashSet<Uuid> =
            results.iter().map(|r| r.session_id).collect();
        self.session_list
            .iter()
            .filter(|e| result_ids.contains(&e.id))
            .collect()
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// Constructor
// ──────────────────────────────────────────────────────────────────────────────

/// Construct the initial application state.
///
/// Used as the `boot` argument of `iced::application`.
pub fn new() -> VoxAppState {
    VoxAppState::new()
}

// ──────────────────────────────────────────────────────────────────────────────
// Update
// ──────────────────────────────────────────────────────────────────────────────

/// Process a [`Message`] and return an optional async [`Task`].
#[allow(clippy::too_many_lines)]
pub fn update(state: &mut VoxAppState, message: Message) -> Task<Message> {
    match message {
        // ── Navigation ────────────────────────────────────────────────────
        Message::NavigateTo(page) => {
            state.page = page;
            Task::none()
        }

        // ── Audio settings ────────────────────────────────────────────────
        Message::MicSourceChanged(opt) => {
            state.settings.audio.mic_source = opt.node_id;
            Task::none()
        }
        Message::AppSourceChanged(opt) => {
            state.settings.audio.app_source = opt.node_id;
            Task::none()
        }

        // ── Transcription settings ────────────────────────────────────────
        Message::ModelChanged(m) => {
            state.settings.transcription.model = m;
            Task::none()
        }
        Message::LanguageChanged(s) => {
            state.settings.transcription.language = s;
            Task::none()
        }
        Message::GpuBackendChanged(b) => {
            state.settings.transcription.gpu_backend = b;
            Task::none()
        }
        Message::ModelPathChanged(s) => {
            state.settings.transcription.model_path = s;
            Task::none()
        }

        // ── Summarization settings ────────────────────────────────────────
        Message::AutoSummarizeToggled(v) => {
            state.settings.summarization.auto_summarize = v;
            Task::none()
        }
        Message::BackendChanged(b) => {
            state.settings.summarization.backend = b;
            Task::none()
        }
        Message::OllamaUrlChanged(s) => {
            state.settings.summarization.ollama_url = s;
            Task::none()
        }
        Message::OllamaModelChanged(s) => {
            state.settings.summarization.ollama_model = s;
            Task::none()
        }
        Message::ApiUrlChanged(s) => {
            state.settings.summarization.api_url = s;
            Task::none()
        }
        Message::ApiKeyChanged(s) => {
            state.settings.summarization.api_key = s;
            Task::none()
        }
        Message::ApiModelChanged(s) => {
            state.settings.summarization.api_model = s;
            Task::none()
        }

        // ── Storage settings ──────────────────────────────────────────────
        Message::DataDirChanged(s) => {
            state.settings.storage.data_dir = s;
            Task::none()
        }
        Message::RetainAudioToggled(v) => {
            state.settings.storage.retain_audio = v;
            Task::none()
        }
        Message::ExportFormatChanged(f) => {
            state.settings.storage.export_format = f;
            Task::none()
        }

        // ── Notification settings ─────────────────────────────────────────
        Message::NotificationsEnabledToggled(v) => {
            state.settings.notifications.enabled = v;
            Task::none()
        }
        Message::OnRecordStartToggled(v) => {
            state.settings.notifications.on_record_start = v;
            Task::none()
        }
        Message::OnRecordStopToggled(v) => {
            state.settings.notifications.on_record_stop = v;
            Task::none()
        }
        Message::OnTranscriptReadyToggled(v) => {
            state.settings.notifications.on_transcript_ready = v;
            Task::none()
        }
        Message::OnSummaryReadyToggled(v) => {
            state.settings.notifications.on_summary_ready = v;
            Task::none()
        }

        // ── Settings persistence ──────────────────────────────────────────
        Message::SaveSettings => {
            let config = state.settings.to_config();
            Task::perform(async move { config.save().is_ok() }, Message::SettingsSaved)
        }
        Message::SettingsSaved(ok) => {
            state.settings_save_status = Some(ok);
            if ok {
                info!("settings saved successfully");
            } else {
                error!("failed to save settings");
            }
            Task::none()
        }

        // ── Browser: search ───────────────────────────────────────────────
        Message::SearchQueryChanged(q) => {
            state.search_query = q;
            Task::none()
        }

        // ── Browser: session selection ────────────────────────────────────
        Message::SessionSelected(id) => {
            state.selected_session_id = Some(id);
            state.browser_status = None;
            let data_dir = state.settings.storage.data_dir.clone();
            Task::perform(
                async move {
                    let store = JsonFileStore::new(&data_dir).map_err(|e| e.to_string())?;
                    store.load(id).map_err(|e| e.to_string())
                },
                |result| match result {
                    Ok(session) => Message::SessionLoaded(Box::new(session)),
                    Err(e) => {
                        error!("failed to load session: {e}");
                        Message::DeleteResult(false)
                    }
                },
            )
        }
        Message::SessionLoaded(session) => {
            state.selected_session = Some(*session);
            Task::none()
        }

        // ── Browser: export ───────────────────────────────────────────────
        Message::ExportSelectedSession => {
            let Some(id) = state.selected_session_id else {
                return Task::none();
            };
            let data_dir = state.settings.storage.data_dir.clone();
            Task::perform(
                async move {
                    let store = JsonFileStore::new(&data_dir).map_err(|e| e.to_string())?;
                    let md = store.export_markdown(id).map_err(|e| e.to_string())?;

                    // Write the markdown alongside the session JSON in the data
                    // directory (one level above the sessions/ subdir).
                    let out_path = vox_core::paths::data_dir_or(&data_dir)
                        .join(format!("{id}.md"));
                    std::fs::write(&out_path, md.as_bytes())
                        .map_err(|e| e.to_string())?;

                    let path_str = out_path
                        .to_str()
                        .unwrap_or("<non-utf8 path>")
                        .to_owned();
                    Ok(path_str)
                },
                Message::ExportResult,
            )
        }
        Message::ExportResult(result) => {
            match result {
                Ok(path) => {
                    info!("session exported to {path}");
                    state.browser_status = Some(format!("Exported to {path}"));
                }
                Err(e) => {
                    error!("export failed: {e}");
                    state.browser_status = Some(format!("Export failed: {e}"));
                }
            }
            Task::none()
        }

        // ── Browser: delete ───────────────────────────────────────────────
        Message::DeleteSelectedSession => {
            let Some(id) = state.selected_session_id else {
                return Task::none();
            };
            let data_dir = state.settings.storage.data_dir.clone();
            Task::perform(
                async move {
                    let store = JsonFileStore::new(&data_dir).map_err(|e| e.to_string())?;
                    store.delete(id).map_err(|e| e.to_string())
                },
                |result| Message::DeleteResult(result.is_ok()),
            )
        }
        Message::DeleteResult(ok) => {
            if ok {
                state.selected_session = None;
                state.selected_session_id = None;
                state.browser_status = Some("Session deleted.".to_owned());
                let data_dir = state.settings.storage.data_dir.clone();
                Task::perform(
                    async move { load_sessions_from_store(&data_dir) },
                    Message::SessionsLoaded,
                )
            } else {
                state.browser_status = Some("Delete failed.".to_owned());
                Task::none()
            }
        }

        // ── Browser: speaker rename ───────────────────────────────────────
        Message::SpeakerNameEdited {
            speaker_index,
            new_name,
        } => {
            if let Some(ref mut session) = state.selected_session {
                if let Some(mapping) = session.speakers.get_mut(speaker_index) {
                    mapping.friendly_name = new_name;
                } else {
                    warn!("speaker_index {speaker_index} out of range");
                }

                // Persist the updated session to disk immediately.
                let session_clone = session.clone();
                let data_dir = state.settings.storage.data_dir.clone();
                return Task::perform(
                    async move {
                        JsonFileStore::new(&data_dir)
                            .and_then(|store| store.save(&session_clone))
                            .is_ok()
                    },
                    Message::SpeakerNamesSaved,
                );
            }
            Task::none()
        }

        // ── Speaker names persistence result ──────────────────────────────
        Message::SpeakerNamesSaved(ok) => {
            if ok {
                info!("speaker names persisted to disk");
            } else {
                error!("failed to persist speaker names to disk");
                state.browser_status = Some("Failed to save speaker names.".to_owned());
            }
            Task::none()
        }

        // ── Sessions refresh ──────────────────────────────────────────────
        Message::SessionsLoaded(sessions) => {
            state.session_list = build_session_list(&sessions);
            state.all_sessions = sessions;
            Task::none()
        }

        // ── Browser: summarization ───────────────────────────────────────
        Message::GenerateSummary => {
            let Some(ref session) = state.selected_session else {
                return Task::none();
            };
            if session.summary.is_some() || session.transcript.is_empty() {
                return Task::none();
            }
            state.summarizing = true;
            state.browser_status = Some("Generating summary…".to_owned());
            let transcript = session.transcript.clone();
            let config = AppConfig::load().unwrap_or_else(|e| {
                warn!("failed to load config for summarization: {e}");
                AppConfig::default()
            });
            Task::perform(
                async move {
                    let summarizer = vox_summarize::create_summarizer(&config.summarization)
                        .map_err(|e| e.to_string())?;
                    let summary = summarizer
                        .summarize(&transcript)
                        .await
                        .map_err(|e| e.to_string())?;
                    Ok(Box::new(summary))
                },
                Message::SummaryGenerated,
            )
        }
        Message::SummaryGenerated(result) => {
            state.summarizing = false;
            match result {
                Ok(summary) => {
                    info!("summary generated successfully");
                    if let Some(ref mut session) = state.selected_session {
                        session.summary = Some(*summary);
                        // Persist the session with the new summary.
                        let session_clone = session.clone();
                        let data_dir = state.settings.storage.data_dir.clone();
                        state.browser_status = Some("Summary generated.".to_owned());
                        // Also update the list entry preview.
                        if let Some(entry) = state
                            .session_list
                            .iter_mut()
                            .find(|e| e.id == session_clone.id)
                        {
                            *entry = SessionListEntry::from_session(&session_clone);
                        }
                        return Task::perform(
                            async move {
                                JsonFileStore::new(&data_dir)
                                    .and_then(|store| store.save(&session_clone))
                                    .is_ok()
                            },
                            Message::SummarySaved,
                        );
                    }
                    Task::none()
                }
                Err(e) => {
                    error!("summary generation failed: {e}");
                    state.browser_status = Some(format!("Summarization failed: {e}"));
                    Task::none()
                }
            }
        }
        Message::SummarySaved(ok) => {
            if ok {
                info!("summary persisted to disk");
            } else {
                error!("failed to persist summary to disk");
                state.browser_status = Some("Failed to save summary.".to_owned());
            }
            Task::none()
        }
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// View
// ──────────────────────────────────────────────────────────────────────────────

/// Build the widget tree for the current state.
pub fn view(state: &VoxAppState) -> Element<'_, Message> {
    let nav = row![
        button("Browser")
            .on_press(Message::NavigateTo(Page::Browser))
            .style(if state.page == Page::Browser {
                button::primary
            } else {
                button::secondary
            }),
        button("Settings")
            .on_press(Message::NavigateTo(Page::Settings))
            .style(if state.page == Page::Settings {
                button::primary
            } else {
                button::secondary
            }),
    ]
    .spacing(vox_theme::SPACING);

    let content: Element<'_, Message> = match state.page {
        Page::Settings => view_settings(state),
        Page::Browser => view_browser(state),
    };

    container(
        column![nav, rule::horizontal(1), content]
            .spacing(vox_theme::SECTION_SPACING)
            .padding(vox_theme::PADDING),
    )
    .width(Fill)
    .height(Fill)
    .into()
}

// ──────────────────────────────────────────────────────────────────────────────
// Settings view helpers
// ──────────────────────────────────────────────────────────────────────────────

fn view_settings(state: &VoxAppState) -> Element<'_, Message> {
    let s = &state.settings;

    // ── Audio section ─────────────────────────────────────────────────────
    let selected_mic = state
        .available_mic_sources
        .iter()
        .find(|o| o.node_id == s.audio.mic_source)
        .cloned();
    let selected_app = state
        .available_app_sources
        .iter()
        .find(|o| o.node_id == s.audio.app_source)
        .cloned();

    let audio_section = section_column(
        "Audio Sources",
        column![
            pick_row(
                "Microphone",
                pick_list(
                    state.available_mic_sources.clone(),
                    selected_mic,
                    Message::MicSourceChanged,
                )
                .placeholder("Choose a source…")
                .width(Fill),
            ),
            pick_row(
                "Application audio",
                pick_list(
                    state.available_app_sources.clone(),
                    selected_app,
                    Message::AppSourceChanged,
                )
                .placeholder("Choose a source…")
                .width(Fill),
            ),
        ],
    );

    // ── Transcription section ─────────────────────────────────────────────
    let transcription_section = section_column(
        "Transcription",
        column![
            pick_row(
                "Whisper model",
                pick_list(
                    WhisperModel::all(),
                    Some(s.transcription.model),
                    Message::ModelChanged,
                )
                .width(Fill),
            ),
            stacked_text_input(
                "Language code",
                "e.g. \"en\" or \"auto\" for detection",
                &s.transcription.language,
                Message::LanguageChanged,
            ),
            pick_row(
                "GPU backend",
                pick_list(
                    GpuBackend::all(),
                    Some(s.transcription.gpu_backend),
                    Message::GpuBackendChanged,
                )
                .width(Fill),
            ),
            stacked_text_input(
                "Custom model path",
                "Leave empty to use the XDG cache default",
                &s.transcription.model_path,
                Message::ModelPathChanged,
            ),
        ],
    );

    // ── Summarization section ─────────────────────────────────────────────
    let summarization_section = section_column(
        "Summarization",
        column![
            pick_row(
                "Backend",
                pick_list(
                    SummarizationBackend::all(),
                    Some(s.summarization.backend),
                    Message::BackendChanged,
                )
                .width(Fill),
            ),
            toggle_row(
                "Auto-summarize when transcription finishes",
                s.summarization.auto_summarize,
                Message::AutoSummarizeToggled,
            ),
            stacked_text_input(
                "Ollama server URL",
                "e.g. http://localhost:11434",
                &s.summarization.ollama_url,
                Message::OllamaUrlChanged,
            ),
            stacked_text_input(
                "Ollama model name",
                "e.g. llama3",
                &s.summarization.ollama_model,
                Message::OllamaModelChanged,
            ),
            stacked_text_input(
                "API URL (OpenAI-compatible)",
                "e.g. https://api.openai.com/v1",
                &s.summarization.api_url,
                Message::ApiUrlChanged,
            ),
            column![
                text("API key").size(13u32),
                text_input("Stored in plaintext — chmod 600 your config file", &s.summarization.api_key)
                    .on_input(Message::ApiKeyChanged)
                    .secure(true)
                    .width(Fill),
            ]
            .spacing(vox_theme::SPACING / 2.0),
            stacked_text_input(
                "API model name",
                "e.g. gpt-4o",
                &s.summarization.api_model,
                Message::ApiModelChanged,
            ),
        ],
    );

    // ── Storage section ───────────────────────────────────────────────────
    let storage_section = section_column(
        "Storage",
        column![
            stacked_text_input(
                "Data directory",
                "Leave empty to use the XDG default ($XDG_DATA_HOME/vox-daemon/)",
                &s.storage.data_dir,
                Message::DataDirChanged,
            ),
            pick_row(
                "Export format",
                pick_list(
                    ExportFormat::all(),
                    Some(s.storage.export_format),
                    Message::ExportFormatChanged,
                )
                .width(Fill),
            ),
            toggle_row(
                "Retain raw audio files after transcription",
                s.storage.retain_audio,
                Message::RetainAudioToggled,
            ),
        ],
    );

    // ── Notifications section ─────────────────────────────────────────────
    let notifications_section = section_column(
        "Notifications",
        column![
            toggle_row(
                "Enable notifications",
                s.notifications.enabled,
                Message::NotificationsEnabledToggled,
            ),
            toggle_row(
                "On recording start",
                s.notifications.on_record_start,
                Message::OnRecordStartToggled,
            ),
            toggle_row(
                "On recording stop",
                s.notifications.on_record_stop,
                Message::OnRecordStopToggled,
            ),
            toggle_row(
                "On transcript ready",
                s.notifications.on_transcript_ready,
                Message::OnTranscriptReadyToggled,
            ),
            toggle_row(
                "On summary ready",
                s.notifications.on_summary_ready,
                Message::OnSummaryReadyToggled,
            ),
        ],
    );

    // ── About section ─────────────────────────────────────────────────────
    let about_section = section_column(
        "About",
        column![
            text(format!("Vox Daemon v{}", env!("CARGO_PKG_VERSION"))),
            text("License: MIT"),
            text("Source: https://github.com/<user>/vox-daemon"),
        ],
    );

    // ── Save button & status ──────────────────────────────────────────────
    let status_str = match state.settings_save_status {
        Some(true) => "Settings saved.",
        Some(false) => "Error saving settings!",
        None => "",
    };

    let save_row = row![
        button("Save Settings")
            .on_press(Message::SaveSettings)
            .style(button::primary),
        text(status_str),
    ]
    .spacing(vox_theme::SPACING)
    .align_y(iced::Alignment::Center);

    scrollable(
        container(
            column![
                audio_section,
                transcription_section,
                summarization_section,
                storage_section,
                notifications_section,
                about_section,
                save_row,
            ]
            .spacing(vox_theme::SECTION_SPACING)
            .width(Fill),
        )
        .max_width(600)
        .center_x(Fill),
    )
    .into()
}

// ──────────────────────────────────────────────────────────────────────────────
// Browser view helpers
// ──────────────────────────────────────────────────────────────────────────────

fn view_browser(state: &VoxAppState) -> Element<'_, Message> {
    // ── Search bar ────────────────────────────────────────────────────────
    let search_bar = text_input("Search transcripts…", &state.search_query)
        .on_input(Message::SearchQueryChanged)
        .width(Fill);

    // ── Session list ──────────────────────────────────────────────────────
    let visible = state.visible_sessions();
    let list_items: Column<'_, Message> =
        visible
            .iter()
            .fold(Column::new().spacing(vox_theme::SPACING), |col, entry| {
                let is_selected = state.selected_session_id == Some(entry.id);
                let preview = entry.summary_preview.as_deref().unwrap_or("No summary yet");
                let item = button(
                    column![
                        text(entry.formatted_date()).size(13u32),
                        row![
                            text(entry.formatted_duration()).size(12u32),
                            text(format!(" · {} segments", entry.segment_count)).size(12u32),
                        ]
                        .spacing(4u32),
                        text(preview).size(11u32),
                    ]
                    .spacing(2u32),
                )
                .on_press(Message::SessionSelected(entry.id))
                .width(Fill)
                .style(if is_selected {
                    button::primary
                } else {
                    button::secondary
                });
                col.push(item)
            });

    let session_list = scrollable(list_items.width(Fill)).height(Fill);

    // ── Detail panel ──────────────────────────────────────────────────────
    let detail: Element<'_, Message> = if let Some(ref session) = state.selected_session {
        view_session_detail(session, state.summarizing)
    } else {
        container(text("Select a session to view its transcript."))
            .center_x(Fill)
            .center_y(Fill)
            .into()
    };

    // ── Status bar ────────────────────────────────────────────────────────
    let status = state.browser_status.as_deref().unwrap_or("");

    column![
        search_bar,
        row![
            container(session_list).width(300u32).height(Fill),
            container(detail).width(Fill).height(Fill),
        ]
        .spacing(vox_theme::SPACING)
        .height(Fill),
        text(status).size(12u32),
    ]
    .spacing(vox_theme::SPACING)
    .height(Fill)
    .into()
}

fn view_session_detail(session: &Session, summarizing: bool) -> Element<'_, Message> {
    // ── Action buttons ────────────────────────────────────────────────────
    let mut actions = row![
        button("Export to Markdown").on_press(Message::ExportSelectedSession),
        button("Delete Session").on_press(Message::DeleteSelectedSession),
    ]
    .spacing(vox_theme::SPACING);

    // Show "Generate Summary" button only when no summary exists yet.
    if session.summary.is_none() && !session.transcript.is_empty() {
        let summarize_btn = if summarizing {
            button("Generating…")
        } else {
            button("Generate Summary").on_press(Message::GenerateSummary)
        };
        actions = actions.push(summarize_btn);
    }

    // ── Summary section (if present) ─────────────────────────────────────
    let summary_section: Column<'_, Message> = if let Some(ref summary) = session.summary {
        let mut col = Column::new().spacing(vox_theme::SPACING);
        col = col.push(text("AI Summary").size(14u32));
        col = col.push(text(&summary.overview).size(13u32));

        if !summary.key_points.is_empty() {
            col = col.push(text("Key Points:").size(13u32));
            for point in &summary.key_points {
                col = col.push(
                    text(format!("  \u{2022} {point}")).size(12u32),
                );
            }
        }

        if !summary.action_items.is_empty() {
            col = col.push(text("Action Items:").size(13u32));
            for item in &summary.action_items {
                let label = match &item.owner {
                    Some(owner) => format!("  \u{2022} {} ({})", item.description, owner),
                    None => format!("  \u{2022} {}", item.description),
                };
                col = col.push(text(label).size(12u32));
            }
        }

        if !summary.decisions.is_empty() {
            col = col.push(text("Decisions:").size(13u32));
            for decision in &summary.decisions {
                col = col.push(
                    text(format!("  \u{2022} {decision}")).size(12u32),
                );
            }
        }

        col = col.push(
            text(format!(
                "Generated {} via {} ({})",
                summary.generated_at.format("%Y-%m-%d %H:%M UTC"),
                summary.backend,
                summary.model,
            ))
            .size(11u32),
        );

        col
    } else {
        Column::new()
    };

    // ── Speaker name editor ───────────────────────────────────────────────
    let speakers_editor: Column<'_, Message> = session.speakers.iter().enumerate().fold(
        Column::new().spacing(vox_theme::SPACING),
        |col, (i, mapping)| {
            col.push(
                row![
                    text(format!("{}:", mapping.id)).width(120u32),
                    text_input("Friendly name", &mapping.friendly_name).on_input(move |name| {
                        Message::SpeakerNameEdited {
                            speaker_index: i,
                            new_name: name,
                        }
                    }),
                ]
                .spacing(vox_theme::SPACING),
            )
        },
    );

    // ── Transcript ────────────────────────────────────────────────────────
    let transcript_items: Column<'_, Message> =
        session
            .transcript
            .iter()
            .fold(Column::new().spacing(4u32), |col, seg| {
                let timestamp = format!(
                    "[{:02}:{:02}]",
                    seg.start_time as u64 / 60,
                    seg.start_time as u64 % 60
                );
                let speaker_color = vox_theme::speaker_color(&seg.speaker);
                col.push(
                    row![
                        text(timestamp)
                            .size(11u32)
                            .font(Font::MONOSPACE)
                            .width(60u32),
                        text(seg.speaker.clone())
                            .size(12u32)
                            .width(80u32)
                            .color(speaker_color),
                        text(seg.text.clone()).size(13u32),
                    ]
                    .spacing(8u32),
                )
            });

    let mut content = column![
        text(format!(
            "Session — {}",
            session.created_at.format("%Y-%m-%d %H:%M UTC")
        ))
        .size(16u32),
        actions,
    ]
    .spacing(vox_theme::SPACING)
    .padding(vox_theme::PADDING);

    // Show summary section between actions and speaker names if present.
    if session.summary.is_some() {
        content = content.push(rule::horizontal(1));
        content = content.push(summary_section);
    }

    content = content.push(rule::horizontal(1));
    content = content.push(text("Speaker Names:").size(13u32));
    content = content.push(speakers_editor);
    content = content.push(rule::horizontal(1));
    content = content.push(text("Transcript:").size(13u32));
    content = content.push(transcript_items);

    scrollable(content).into()
}

// ──────────────────────────────────────────────────────────────────────────────
// Widget helpers
// ──────────────────────────────────────────────────────────────────────────────

/// Wrap content in a titled section column with an 18pt header.
///
/// The section title is rendered at a slightly larger size so it reads
/// distinctly from field labels, and a horizontal rule is drawn beneath it for
/// visual separation.
fn section_column<'a>(title: &'a str, content: Column<'a, Message>) -> Element<'a, Message> {
    column![
        text(title).size(18u32),
        rule::horizontal(1),
        content.spacing(vox_theme::SPACING),
    ]
    .spacing(vox_theme::SPACING)
    .into()
}

/// A text input with a short label above it and a description as the placeholder.
///
/// Uses a vertical (stacked) layout so the field spans the full available width
/// without clipping the label, regardless of window size.
fn stacked_text_input<'a, F>(
    label: &'a str,
    placeholder: &'a str,
    value: &'a str,
    on_change: F,
) -> Element<'a, Message>
where
    F: Fn(String) -> Message + 'a,
{
    column![
        text(label).size(13u32),
        text_input(placeholder, value)
            .on_input(on_change)
            .width(Fill),
    ]
    .spacing(vox_theme::SPACING / 2.0)
    .into()
}

/// A labelled pick-list row using proportional widths.
///
/// The label occupies one third of the row and the picker takes the remaining
/// two thirds, both scaling proportionally with the window width so neither
/// component is clipped.
fn pick_row<'a>(label: &'a str, picker: impl Into<Element<'a, Message>>) -> Element<'a, Message> {
    row![
        text(label)
            .size(13u32)
            .width(Length::FillPortion(1)),
        container(picker.into()).width(Length::FillPortion(2)),
    ]
    .spacing(vox_theme::SPACING)
    .align_y(iced::Alignment::Center)
    .into()
}

/// A labelled toggler row using proportional widths.
///
/// The label occupies three quarters of the row; the toggler sits at the right.
fn toggle_row<'a, F>(label: &'a str, value: bool, on_toggle: F) -> Element<'a, Message>
where
    F: Fn(bool) -> Message + 'a,
{
    row![
        text(label)
            .size(13u32)
            .width(Length::FillPortion(3)),
        toggler(value)
            .on_toggle(on_toggle),
    ]
    .spacing(vox_theme::SPACING)
    .align_y(iced::Alignment::Center)
    .into()
}

// ──────────────────────────────────────────────────────────────────────────────
// Theme helper
// ──────────────────────────────────────────────────────────────────────────────

/// Return the application theme, adapting to the system's dark/light preference.
///
/// Detection heuristics (in priority order):
/// 1. `GTK_THEME` env var — if it contains `"dark"` (case-insensitive), use
///    the dark theme.
/// 2. `COLORFGBG` env var — a value ending in `;0` or `;black` indicates a
///    dark terminal background, which is a reasonable proxy.
/// 3. Fall back to the light theme.
///
/// iced 0.14 does not expose automatic system-theme integration, so this is a
/// best-effort heuristic suitable for GNOME/GTK desktops and KDE with
/// `GTK_THEME` set.
pub fn theme(_state: &VoxAppState) -> Theme {
    let is_dark = std::env::var("GTK_THEME")
        .map(|t| t.to_ascii_lowercase().contains("dark"))
        .unwrap_or(false)
        || std::env::var("COLORFGBG")
            .map(|v| v.ends_with(";0") || v.ends_with(";black"))
            .unwrap_or(false);

    if is_dark {
        Theme::Dark
    } else {
        Theme::Light
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// Public entry point
// ──────────────────────────────────────────────────────────────────────────────

/// Launch the Vox Daemon GUI window.
///
/// Blocks until the window is closed.
///
/// # Errors
///
/// Returns an `iced::Error` if the window cannot be created (e.g. no display
/// server available).
pub fn run() -> iced::Result {
    iced::application(new, update, view)
        .title("Vox Daemon")
        .theme(theme)
        .window(iced::window::Settings {
            size: Size::new(
                vox_theme::WINDOW_MIN_WIDTH as f32,
                vox_theme::WINDOW_MIN_HEIGHT as f32,
            ),
            ..Default::default()
        })
        .run()
}

// ──────────────────────────────────────────────────────────────────────────────
// Internal helpers
// ──────────────────────────────────────────────────────────────────────────────

/// Enumerate PipeWire audio sources for the settings dropdowns.
///
/// Returns an empty vec when the `pw` feature is not enabled or when the
/// PipeWire daemon is unreachable.
fn enumerate_pipewire_sources() -> Vec<vox_capture::StreamInfo> {
    #[cfg(feature = "pw")]
    {
        match vox_capture::PipeWireSource::enumerate_streams(
            &vox_capture::StreamFilter::default(),
        ) {
            Ok(streams) => streams,
            Err(e) => {
                warn!("failed to enumerate PipeWire sources: {e}");
                vec![]
            }
        }
    }
    #[cfg(not(feature = "pw"))]
    {
        vec![]
    }
}

/// Load all sessions from the store, returning an empty vec on error.
fn load_sessions_from_store(data_dir: &str) -> Vec<Session> {
    match JsonFileStore::new(data_dir) {
        Ok(store) => store.list().unwrap_or_else(|e| {
            warn!("failed to list sessions: {e}");
            vec![]
        }),
        Err(e) => {
            warn!("failed to open session store: {e}");
            vec![]
        }
    }
}
