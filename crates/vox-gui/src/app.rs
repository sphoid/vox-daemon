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
use iced::{Element, Fill, Font, Size, Task, Theme};
use tracing::{error, info, warn};
use uuid::Uuid;
use vox_core::config::AppConfig;
use vox_core::session::Session;
use vox_storage::store::{JsonFileStore, SessionStore};

use crate::browser::{SessionListEntry, build_session_list};
use crate::search::search_transcripts;
use crate::settings::{
    ExportFormat, GpuBackend, SettingsModel, SummarizationBackend, WhisperModel,
};
use crate::theme as vox_theme;

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
    /// The mic source text field changed.
    MicSourceChanged(String),
    /// The app source text field changed.
    AppSourceChanged(String),

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
    /// Export result (the Markdown string, or an error message).
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

        Self {
            page: Page::Browser,
            settings,
            settings_save_status: None,
            session_list,
            all_sessions,
            search_query: String::new(),
            selected_session_id: None,
            selected_session: None,
            browser_status: None,
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
        Message::MicSourceChanged(s) => {
            state.settings.audio.mic_source = s;
            Task::none()
        }
        Message::AppSourceChanged(s) => {
            state.settings.audio.app_source = s;
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
                    store.export_markdown(id).map_err(|e| e.to_string())
                },
                Message::ExportResult,
            )
        }
        Message::ExportResult(result) => {
            match result {
                Ok(md) => {
                    info!("export produced {} bytes of Markdown", md.len());
                    state.browser_status = Some(format!("Exported ({} bytes)", md.len()));
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
            }
            Task::none()
        }

        // ── Sessions refresh ──────────────────────────────────────────────
        Message::SessionsLoaded(sessions) => {
            state.session_list = build_session_list(&sessions);
            state.all_sessions = sessions;
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
    let audio_section = section_column(
        "Audio Sources",
        column![
            labeled_text_input(
                "Microphone source (PipeWire node ID or \"auto\")",
                &s.audio.mic_source,
                Message::MicSourceChanged,
            ),
            labeled_text_input(
                "Application audio source (PipeWire node ID or \"auto\")",
                &s.audio.app_source,
                Message::AppSourceChanged,
            ),
        ],
    );

    // ── Transcription section ─────────────────────────────────────────────
    let transcription_section = section_column(
        "Transcription",
        column![
            row![
                text("Whisper model:").width(200u32),
                pick_list(
                    WhisperModel::all(),
                    Some(s.transcription.model),
                    Message::ModelChanged,
                )
            ]
            .spacing(vox_theme::SPACING),
            labeled_text_input(
                "Language code (e.g. \"en\" or \"auto\")",
                &s.transcription.language,
                Message::LanguageChanged,
            ),
            row![
                text("GPU backend:").width(200u32),
                pick_list(
                    GpuBackend::all(),
                    Some(s.transcription.gpu_backend),
                    Message::GpuBackendChanged,
                )
            ]
            .spacing(vox_theme::SPACING),
            labeled_text_input(
                "Custom model path (leave empty for default cache)",
                &s.transcription.model_path,
                Message::ModelPathChanged,
            ),
        ],
    );

    // ── Summarization section ─────────────────────────────────────────────
    let summarization_section = section_column(
        "Summarization",
        column![
            row![
                text("Backend:").width(200u32),
                pick_list(
                    SummarizationBackend::all(),
                    Some(s.summarization.backend),
                    Message::BackendChanged,
                )
            ]
            .spacing(vox_theme::SPACING),
            row![
                text("Auto-summarize:").width(200u32),
                toggler(s.summarization.auto_summarize).on_toggle(Message::AutoSummarizeToggled),
            ]
            .spacing(vox_theme::SPACING),
            labeled_text_input(
                "Ollama server URL",
                &s.summarization.ollama_url,
                Message::OllamaUrlChanged,
            ),
            labeled_text_input(
                "Ollama model name",
                &s.summarization.ollama_model,
                Message::OllamaModelChanged,
            ),
            labeled_text_input(
                "API URL (OpenAI-compatible)",
                &s.summarization.api_url,
                Message::ApiUrlChanged,
            ),
            text_input("API key (stored in plaintext)", &s.summarization.api_key)
                .on_input(Message::ApiKeyChanged)
                .secure(true),
            labeled_text_input(
                "API model name",
                &s.summarization.api_model,
                Message::ApiModelChanged,
            ),
        ],
    );

    // ── Storage section ───────────────────────────────────────────────────
    let storage_section = section_column(
        "Storage",
        column![
            labeled_text_input(
                "Data directory (leave empty for XDG default)",
                &s.storage.data_dir,
                Message::DataDirChanged,
            ),
            row![
                text("Export format:").width(200u32),
                pick_list(
                    ExportFormat::all(),
                    Some(s.storage.export_format),
                    Message::ExportFormatChanged,
                )
            ]
            .spacing(vox_theme::SPACING),
            row![
                text("Retain raw audio:").width(200u32),
                toggler(s.storage.retain_audio).on_toggle(Message::RetainAudioToggled),
            ]
            .spacing(vox_theme::SPACING),
        ],
    );

    // ── Notifications section ─────────────────────────────────────────────
    let notifications_section = section_column(
        "Notifications",
        column![
            row![
                text("Enable notifications:").width(240u32),
                toggler(s.notifications.enabled).on_toggle(Message::NotificationsEnabledToggled),
            ]
            .spacing(vox_theme::SPACING),
            row![
                text("On recording start:").width(240u32),
                toggler(s.notifications.on_record_start).on_toggle(Message::OnRecordStartToggled),
            ]
            .spacing(vox_theme::SPACING),
            row![
                text("On recording stop:").width(240u32),
                toggler(s.notifications.on_record_stop).on_toggle(Message::OnRecordStopToggled),
            ]
            .spacing(vox_theme::SPACING),
            row![
                text("On transcript ready:").width(240u32),
                toggler(s.notifications.on_transcript_ready)
                    .on_toggle(Message::OnTranscriptReadyToggled),
            ]
            .spacing(vox_theme::SPACING),
            row![
                text("On summary ready:").width(240u32),
                toggler(s.notifications.on_summary_ready).on_toggle(Message::OnSummaryReadyToggled),
            ]
            .spacing(vox_theme::SPACING),
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
        button("Save Settings").on_press(Message::SaveSettings),
        text(status_str),
    ]
    .spacing(vox_theme::SPACING);

    scrollable(
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
        view_session_detail(session)
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

fn view_session_detail(session: &Session) -> Element<'_, Message> {
    // ── Action buttons ────────────────────────────────────────────────────
    let actions = row![
        button("Export to Markdown").on_press(Message::ExportSelectedSession),
        button("Delete Session").on_press(Message::DeleteSelectedSession),
    ]
    .spacing(vox_theme::SPACING);

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
                col.push(
                    row![
                        text(timestamp)
                            .size(11u32)
                            .font(Font::MONOSPACE)
                            .width(60u32),
                        text(seg.speaker.clone()).size(12u32).width(80u32),
                        text(seg.text.clone()).size(13u32),
                    ]
                    .spacing(8u32),
                )
            });

    scrollable(
        column![
            text(format!(
                "Session — {}",
                session.created_at.format("%Y-%m-%d %H:%M UTC")
            ))
            .size(16u32),
            actions,
            rule::horizontal(1),
            text("Speaker Names:").size(13u32),
            speakers_editor,
            rule::horizontal(1),
            text("Transcript:").size(13u32),
            transcript_items,
        ]
        .spacing(vox_theme::SPACING)
        .padding(vox_theme::PADDING),
    )
    .into()
}

// ──────────────────────────────────────────────────────────────────────────────
// Widget helpers
// ──────────────────────────────────────────────────────────────────────────────

/// Wrap content in a titled section column.
fn section_column<'a>(title: &'a str, content: Column<'a, Message>) -> Element<'a, Message> {
    column![
        text(title).size(15u32),
        rule::horizontal(1),
        content.spacing(vox_theme::SPACING),
    ]
    .spacing(vox_theme::SPACING)
    .into()
}

/// A full-width text input field with a placeholder label.
fn labeled_text_input<'a, F>(label: &'a str, value: &'a str, on_change: F) -> Element<'a, Message>
where
    F: Fn(String) -> Message + 'a,
{
    text_input(label, value)
        .on_input(on_change)
        .width(Fill)
        .into()
}

// ──────────────────────────────────────────────────────────────────────────────
// Theme helper
// ──────────────────────────────────────────────────────────────────────────────

/// Return the application theme.
///
/// Uses the system-detected theme when `linux-theme-detection` is enabled,
/// otherwise falls back to `TokyoNight` (dark).
pub fn theme(_state: &VoxAppState) -> Theme {
    Theme::TokyoNight
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
