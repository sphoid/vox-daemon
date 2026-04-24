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

use std::sync::{Arc, OnceLock};

use iced::widget::rule;
use iced::widget::{
    Column, button, column, container, pick_list, row, scrollable, text, text_input, toggler,
};
use iced::{Color, Element, Fill, Font, Length, Size, Task, Theme};
use tracing::{error, info, warn};
use uuid::Uuid;
use vox_core::config::AppConfig;
use vox_core::session::{Session, Summary};
use vox_export::{
    ExportRequest, ExportResult, ExportTarget, Folder as ExportFolder, Workspace as ExportWorkspace,
};
use vox_storage::store::{JsonFileStore, SessionStore};

use crate::browser::{SessionListEntry, build_session_list};
use crate::search::search_transcripts;
use crate::settings::{
    ExportContent, ExportFormat, GpuBackend, SettingsModel, SummarizationBackend, WhisperModel,
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

/// Holds the initial page set via [`run_with_page`].
static INITIAL_PAGE: OnceLock<Page> = OnceLock::new();

/// When `true`, the browser auto-selects the most recent session on startup.
static SELECT_LATEST: OnceLock<bool> = OnceLock::new();

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

    // ── Settings: Export (Affine) ────────────────────────────────────
    /// The `AFFiNE`export enabled toggle changed.
    AffineEnabledToggled(bool),
    /// The `AFFiNE`base URL text field changed.
    AffineBaseUrlChanged(String),
    /// The `AFFiNE`API token text field changed.
    AffineApiTokenChanged(String),
    /// The `AFFiNE`login email text field changed.
    AffineEmailChanged(String),
    /// The `AFFiNE`login password text field changed.
    AffinePasswordChanged(String),

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
    /// Open the export modal dialog.
    OpenExportModal,
    /// Close the export modal without exporting.
    CloseExportModal,
    /// The "what to export" pick-list in the modal changed.
    ExportContentChanged(ExportContent),
    /// The "format" pick-list in the modal changed.
    ExportModalFormatChanged(ExportFormat),
    /// The user confirmed the export (triggers file dialog + write).
    ConfirmExport,
    /// Export completed (carries the file path or an error message).
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

    // ── Browser: send-to (plugin export) ─────────────────────────────
    /// Open the Send-to modal — triggers `list_workspaces` on the target.
    OpenSendToModal,
    /// Close the Send-to modal without sending.
    CloseSendToModal,
    /// Document-title text field in the Send-to modal changed.
    SendToTitleChanged(String),
    /// Content pick-list in the Send-to modal changed.
    SendToContentChanged(ExportContent),
    /// Workspace list finished loading.
    SendToWorkspacesLoaded(Result<Vec<ExportWorkspace>, String>),
    /// User chose a workspace in the Send-to modal.
    SendToWorkspaceSelected(ExportWorkspace),
    /// Folder list finished loading.
    SendToFoldersLoaded(Result<Vec<ExportFolder>, String>),
    /// User picked a folder (parent doc) or `None` for workspace root.
    SendToFolderSelected(Option<ExportFolder>),
    /// Toggle between "pick existing folder" and "create new folder" modes.
    SendToCreateFolderToggle,
    /// New-folder title text field changed.
    SendToCreateFolderTitleChanged(String),
    /// Commit the new-folder creation (calls `target.create_folder`).
    SendToCreateFolderCommit,
    /// New folder creation finished.
    SendToFolderCreated(Result<ExportFolder, String>),
    /// User confirmed the send — triggers `target.export`.
    ConfirmSendTo,
    /// Send finished; carries the remote URL (on success) or an error message.
    SendToResult(Result<String, String>),

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

/// State for the export modal overlay.
#[derive(Debug, Clone)]
pub struct ExportModalState {
    /// What content to include in the export.
    pub content: ExportContent,
    /// Export file format.
    pub format: ExportFormat,
}

impl Default for ExportModalState {
    fn default() -> Self {
        Self {
            content: ExportContent::TranscriptAndSummary,
            format: ExportFormat::Markdown,
        }
    }
}

/// State for the Send-to modal overlay (plugin-based export).
///
/// One `SendToModalState` exists per open-and-send interaction. The enabled
/// target (`target`) is cloned in each request to allow concurrent background
/// loads (workspace list, folder list, send) while the user continues
/// interacting with the modal.
#[allow(clippy::struct_excessive_bools)]
pub struct SendToModalState {
    /// The plugin target the user is sending to. Currently always `AffineTarget`.
    pub target: Arc<dyn ExportTarget>,
    /// User-editable document title.
    pub title: String,
    /// Which content to include in the rendered Markdown.
    pub content: ExportContent,
    /// All workspaces visible on the remote service.
    pub workspaces: Vec<ExportWorkspace>,
    /// `true` while a workspace load is in flight.
    pub workspaces_loading: bool,
    /// The selected workspace.
    pub selected_workspace: Option<ExportWorkspace>,
    /// All folders (parent docs) within the selected workspace.
    pub folders: Vec<ExportFolder>,
    /// `true` while a folder load is in flight.
    pub folders_loading: bool,
    /// The selected folder, or `None` for "workspace root".
    pub selected_folder: Option<ExportFolder>,
    /// `true` when the user is typing a new folder name.
    pub creating_folder: bool,
    /// Title of the folder being created.
    pub new_folder_title: String,
    /// `true` while the final send is in flight.
    pub sending: bool,
    /// Last error message (if any) to surface inline.
    pub error: Option<String>,
}

impl std::fmt::Debug for SendToModalState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // `target` is a trait object so we cannot derive Debug; print its id
        // and a summary of the interesting state. `finish_non_exhaustive`
        // keeps the lint happy without forcing us to enumerate every field.
        f.debug_struct("SendToModalState")
            .field("target", &self.target.id())
            .field("title", &self.title)
            .field("workspaces", &self.workspaces.len())
            .field("folders", &self.folders.len())
            .field("sending", &self.sending)
            .finish_non_exhaustive()
    }
}

impl SendToModalState {
    /// Construct a fresh modal for the given target and default values.
    #[must_use]
    pub fn new(
        target: Arc<dyn ExportTarget>,
        default_title: String,
        default_content: ExportContent,
    ) -> Self {
        Self {
            target,
            title: default_title,
            content: default_content,
            workspaces: Vec::new(),
            workspaces_loading: true,
            selected_workspace: None,
            folders: Vec::new(),
            folders_loading: false,
            selected_folder: None,
            creating_folder: false,
            new_folder_title: String::new(),
            sending: false,
            error: None,
        }
    }
}

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

    /// When `Some`, the export modal is open.
    pub export_modal: Option<ExportModalState>,

    /// When `Some`, the Send-to (plugin export) modal is open.
    pub send_to_modal: Option<SendToModalState>,
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

        // When launched with --page=latest, auto-select the most recent session.
        let select_latest = SELECT_LATEST.get().copied().unwrap_or(false);
        let (selected_session_id, selected_session) = if select_latest {
            all_sessions
                .first()
                .map(|s| (Some(s.id), Some(s.clone())))
                .unwrap_or((None, None))
        } else {
            (None, None)
        };

        Self {
            page: INITIAL_PAGE.get().copied().unwrap_or(Page::Settings),
            settings,
            settings_save_status: None,
            available_mic_sources,
            available_app_sources,
            session_list,
            all_sessions,
            search_query: String::new(),
            selected_session_id,
            selected_session,
            browser_status: None,
            summarizing: false,
            export_modal: None,
            send_to_modal: None,
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

        // ── Export settings (Affine) ──────────────────────────────────────
        Message::AffineEnabledToggled(v) => {
            state.settings.export.affine.enabled = v;
            Task::none()
        }
        Message::AffineBaseUrlChanged(s) => {
            state.settings.export.affine.base_url = s;
            Task::none()
        }
        Message::AffineApiTokenChanged(s) => {
            state.settings.export.affine.api_token = s;
            Task::none()
        }
        Message::AffineEmailChanged(s) => {
            state.settings.export.affine.email = s;
            Task::none()
        }
        Message::AffinePasswordChanged(s) => {
            state.settings.export.affine.password = s;
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

        // ── Browser: export modal ────────────────────────────────────────
        Message::OpenExportModal => {
            state.export_modal = Some(ExportModalState::default());
            Task::none()
        }
        Message::CloseExportModal => {
            state.export_modal = None;
            Task::none()
        }
        Message::ExportContentChanged(content) => {
            if let Some(ref mut modal) = state.export_modal {
                modal.content = content;
            }
            Task::none()
        }
        Message::ExportModalFormatChanged(format) => {
            if let Some(ref mut modal) = state.export_modal {
                modal.format = format;
            }
            Task::none()
        }
        Message::ConfirmExport => {
            let Some(ref modal) = state.export_modal else {
                return Task::none();
            };
            let Some(ref session) = state.selected_session else {
                return Task::none();
            };

            let format = modal.format;
            let include_transcript = matches!(
                modal.content,
                ExportContent::Transcript | ExportContent::TranscriptAndSummary
            );
            let include_summary = matches!(
                modal.content,
                ExportContent::Summary | ExportContent::TranscriptAndSummary
            );

            let session = session.clone();
            let date = session.created_at.format("%Y-%m-%d").to_string();
            let default_filename = format!("meeting-{date}.{}", format.extension());
            let format_str = format.as_str().to_owned();
            let format_label = format.to_string();
            let extension = format.extension().to_owned();

            state.export_modal = None;

            Task::perform(
                async move {
                    let options = vox_storage::RenderOptions {
                        include_transcript,
                        include_summary,
                    };
                    let content = vox_storage::render_export(&session, &format_str, &options)?;

                    let dialog = rfd::AsyncFileDialog::new()
                        .set_title("Export Session")
                        .set_file_name(&default_filename)
                        .add_filter(&format_label, &[&extension]);

                    let handle = dialog.save_file().await;
                    let Some(handle) = handle else {
                        return Err("Export cancelled.".to_owned());
                    };

                    let path = handle.path().to_path_buf();
                    std::fs::write(&path, content.as_bytes()).map_err(|e| e.to_string())?;

                    Ok(path.to_str().unwrap_or("<non-utf8 path>").to_owned())
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
                    if e == "Export cancelled." {
                        info!("export cancelled by user");
                    } else {
                        error!("export failed: {e}");
                    }
                    state.browser_status = Some(e);
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

        // ── Browser: send-to (plugin export) ─────────────────────────
        Message::OpenSendToModal => {
            let Some(ref session) = state.selected_session else {
                return Task::none();
            };
            let config = state.settings.to_config();
            let mut targets = vox_export::build_targets(&config.export);
            if targets.is_empty() {
                state.browser_status = Some(
                    "No export targets enabled — configure one in Settings → Export.".to_owned(),
                );
                return Task::none();
            }
            let target: Arc<dyn ExportTarget> = Arc::from(targets.remove(0));
            let default_title = default_export_title(session);
            let modal = SendToModalState::new(
                target.clone(),
                default_title,
                ExportContent::TranscriptAndSummary,
            );
            state.send_to_modal = Some(modal);
            Task::perform(
                async move { target.list_workspaces().await.map_err(|e| e.to_string()) },
                Message::SendToWorkspacesLoaded,
            )
        }
        Message::CloseSendToModal => {
            state.send_to_modal = None;
            Task::none()
        }
        Message::SendToTitleChanged(t) => {
            if let Some(ref mut modal) = state.send_to_modal {
                modal.title = t;
            }
            Task::none()
        }
        Message::SendToContentChanged(c) => {
            if let Some(ref mut modal) = state.send_to_modal {
                modal.content = c;
            }
            Task::none()
        }
        Message::SendToWorkspacesLoaded(result) => {
            if let Some(ref mut modal) = state.send_to_modal {
                modal.workspaces_loading = false;
                match result {
                    Ok(ws) => {
                        modal.workspaces = ws;
                        modal.error = None;
                    }
                    Err(e) => modal.error = Some(format!("Failed to load workspaces: {e}")),
                }
            }
            Task::none()
        }
        Message::SendToWorkspaceSelected(ws) => {
            let Some(ref mut modal) = state.send_to_modal else {
                return Task::none();
            };
            modal.selected_workspace = Some(ws.clone());
            modal.folders.clear();
            modal.selected_folder = None;
            modal.folders_loading = true;
            let target = modal.target.clone();
            let ws_id = ws.id;
            Task::perform(
                async move { target.list_folders(&ws_id).await.map_err(|e| e.to_string()) },
                Message::SendToFoldersLoaded,
            )
        }
        Message::SendToFoldersLoaded(result) => {
            if let Some(ref mut modal) = state.send_to_modal {
                modal.folders_loading = false;
                match result {
                    Ok(fs) => {
                        modal.folders = fs;
                        modal.error = None;
                    }
                    Err(e) => modal.error = Some(format!("Failed to load folders: {e}")),
                }
            }
            Task::none()
        }
        Message::SendToFolderSelected(f) => {
            if let Some(ref mut modal) = state.send_to_modal {
                modal.selected_folder = f;
                modal.creating_folder = false;
            }
            Task::none()
        }
        Message::SendToCreateFolderToggle => {
            if let Some(ref mut modal) = state.send_to_modal {
                modal.creating_folder = !modal.creating_folder;
                if modal.creating_folder {
                    modal.selected_folder = None;
                } else {
                    modal.new_folder_title.clear();
                }
            }
            Task::none()
        }
        Message::SendToCreateFolderTitleChanged(t) => {
            if let Some(ref mut modal) = state.send_to_modal {
                modal.new_folder_title = t;
            }
            Task::none()
        }
        Message::SendToCreateFolderCommit => {
            let Some(ref modal) = state.send_to_modal else {
                return Task::none();
            };
            let Some(ref ws) = modal.selected_workspace else {
                return Task::none();
            };
            if modal.new_folder_title.trim().is_empty() {
                return Task::none();
            }
            let target = modal.target.clone();
            let ws_id = ws.id.clone();
            let title = modal.new_folder_title.clone();
            Task::perform(
                async move {
                    target
                        .create_folder(&ws_id, None, &title)
                        .await
                        .map_err(|e| e.to_string())
                },
                Message::SendToFolderCreated,
            )
        }
        Message::SendToFolderCreated(result) => {
            if let Some(ref mut modal) = state.send_to_modal {
                match result {
                    Ok(folder) => {
                        modal.folders.push(folder.clone());
                        modal.selected_folder = Some(folder);
                        modal.creating_folder = false;
                        modal.new_folder_title.clear();
                        modal.error = None;
                    }
                    Err(e) => modal.error = Some(format!("Failed to create folder: {e}")),
                }
            }
            Task::none()
        }
        Message::ConfirmSendTo => {
            let Some(ref mut modal) = state.send_to_modal else {
                return Task::none();
            };
            let Some(ref session) = state.selected_session else {
                return Task::none();
            };
            let Some(ref ws) = modal.selected_workspace else {
                modal.error = Some("Pick a workspace first.".to_owned());
                return Task::none();
            };
            if modal.title.trim().is_empty() {
                modal.error = Some("Title is required.".to_owned());
                return Task::none();
            }

            modal.sending = true;
            modal.error = None;

            let target = modal.target.clone();
            let title = modal.title.trim().to_owned();
            let workspace_id = ws.id.clone();
            let parent_id = modal.selected_folder.as_ref().map(|f| f.id.clone());
            let include_transcript = matches!(
                modal.content,
                ExportContent::Transcript | ExportContent::TranscriptAndSummary
            );
            let include_summary = matches!(
                modal.content,
                ExportContent::Summary | ExportContent::TranscriptAndSummary
            );
            let session = session.clone();

            Task::perform(
                async move {
                    let options = vox_storage::RenderOptions {
                        include_transcript,
                        include_summary,
                    };
                    let markdown = vox_storage::render_export(&session, "markdown", &options)?;
                    let request = ExportRequest {
                        workspace_id,
                        parent_id,
                        title,
                        content_markdown: &markdown,
                        session: &session,
                    };
                    let ExportResult {
                        remote_url,
                        remote_id,
                    } = target.export(request).await.map_err(|e| e.to_string())?;
                    Ok(remote_url.unwrap_or(remote_id))
                },
                Message::SendToResult,
            )
        }
        Message::SendToResult(result) => {
            match result {
                Ok(url) => {
                    info!(%url, "send-to export succeeded");
                    state.send_to_modal = None;
                    state.browser_status = Some(format!("Sent to {url}"));
                }
                Err(e) => {
                    error!("send-to export failed: {e}");
                    if let Some(ref mut modal) = state.send_to_modal {
                        modal.sending = false;
                        modal.error = Some(e);
                    }
                }
            }
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
    // Send-to modal takes over the window when open (checked before export
    // modal so the two cannot visually stack).
    if let Some(ref modal) = state.send_to_modal {
        return container(view_send_to_form(modal))
            .width(Fill)
            .height(Fill)
            .padding(vox_theme::PADDING)
            .into();
    }

    // When the export form is open, it takes over the entire window.
    if let Some(ref modal) = state.export_modal {
        let has_transcript = state
            .selected_session
            .as_ref()
            .is_some_and(|s| !s.transcript.is_empty());
        let has_summary = state
            .selected_session
            .as_ref()
            .is_some_and(|s| s.summary.is_some());

        return container(view_export_form(modal, has_transcript, has_summary))
            .width(Fill)
            .height(Fill)
            .padding(vox_theme::PADDING)
            .into();
    }

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
// Export form (shown inline in the detail panel)
// ──────────────────────────────────────────────────────────────────────────────

/// Build the export form that replaces the session detail when open.
fn view_export_form<'a>(
    modal: &ExportModalState,
    has_transcript: bool,
    has_summary: bool,
) -> Element<'a, Message> {
    // Determine whether the Export button should be enabled.
    let content_available = match modal.content {
        ExportContent::Transcript => has_transcript,
        ExportContent::Summary => has_summary,
        ExportContent::TranscriptAndSummary => has_transcript || has_summary,
    };

    let content_hint: Element<'_, Message> = if !content_available {
        text("Selected content is not available for this session.")
            .size(11u32)
            .color(Color {
                r: 0.85,
                g: 0.25,
                b: 0.25,
                a: 1.0,
            })
            .into()
    } else {
        column![].into()
    };

    let export_btn = if content_available {
        button("Export").on_press(Message::ConfirmExport)
    } else {
        button("Export")
    };

    column![
        text("Export Session").size(16u32),
        rule::horizontal(1),
        // What to export
        column![
            text("What to export").size(12u32),
            pick_list(
                ExportContent::all(),
                Some(modal.content),
                Message::ExportContentChanged,
            )
            .width(300u32),
        ]
        .spacing(4),
        // Format
        column![
            text("Format").size(12u32),
            pick_list(
                ExportFormat::all(),
                Some(modal.format),
                Message::ExportModalFormatChanged,
            )
            .width(300u32),
        ]
        .spacing(4),
        content_hint,
        // Buttons
        row![
            button("Cancel")
                .on_press(Message::CloseExportModal)
                .style(button::secondary),
            export_btn.style(button::primary),
        ]
        .spacing(vox_theme::SPACING),
    ]
    .spacing(vox_theme::SECTION_SPACING)
    .padding(vox_theme::PADDING)
    .into()
}

// ──────────────────────────────────────────────────────────────────────────────
// Send-to (plugin export) view
// ──────────────────────────────────────────────────────────────────────────────

/// Wrapper around an optional folder for the pick-list widget (iced's
/// `pick_list` wants a single `T: Clone + Eq + Display`).
#[derive(Debug, Clone, PartialEq, Eq)]
enum FolderChoice {
    Root,
    Existing(ExportFolder),
}

impl std::fmt::Display for FolderChoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Root => f.write_str("— Workspace root —"),
            Self::Existing(folder) => f.write_str(&folder.title),
        }
    }
}

/// Default document title for a session: `meeting-YYYY-MM-DD`.
fn default_export_title(session: &Session) -> String {
    let date = session.created_at.format("%Y-%m-%d").to_string();
    format!("meeting-{date}")
}

#[allow(clippy::too_many_lines)]
fn view_send_to_form(modal: &SendToModalState) -> Element<'_, Message> {
    let header = text(format!("Send to {}", modal.target.display_name())).size(16u32);

    // ── Title ────────────────────────────────────────────────────────────
    let title_field = column![
        text("Document title").size(12u32),
        text_input("meeting-YYYY-MM-DD", &modal.title)
            .on_input(Message::SendToTitleChanged)
            .width(Fill),
    ]
    .spacing(4);

    // ── Content toggle (reuses the existing enum) ────────────────────────
    let content_field = column![
        text("Content").size(12u32),
        pick_list(
            ExportContent::all(),
            Some(modal.content),
            Message::SendToContentChanged,
        )
        .width(Fill),
    ]
    .spacing(4);

    // ── Workspace picker ─────────────────────────────────────────────────
    let workspace_picker: Element<'_, Message> = if modal.workspaces_loading {
        text("Loading workspaces…").size(12u32).into()
    } else if modal.workspaces.is_empty() {
        text("No workspaces available.").size(12u32).into()
    } else {
        pick_list(
            modal.workspaces.clone(),
            modal.selected_workspace.clone(),
            Message::SendToWorkspaceSelected,
        )
        .placeholder("Choose a workspace…")
        .width(Fill)
        .into()
    };
    let workspace_field = column![text("Workspace").size(12u32), workspace_picker].spacing(4);

    // ── Folder picker + Create-new toggle ────────────────────────────────
    let folder_field: Element<'_, Message> = if modal.selected_workspace.is_none() {
        column![
            text("Folder").size(12u32),
            text("Pick a workspace first.").size(11u32),
        ]
        .spacing(4)
        .into()
    } else if modal.folders_loading {
        column![
            text("Folder").size(12u32),
            text("Loading folders…").size(11u32),
        ]
        .spacing(4)
        .into()
    } else if modal.creating_folder {
        let commit_btn = if modal.new_folder_title.trim().is_empty() {
            button("Create")
        } else {
            button("Create").on_press(Message::SendToCreateFolderCommit)
        };
        column![
            text("New folder").size(12u32),
            text_input("Folder title", &modal.new_folder_title)
                .on_input(Message::SendToCreateFolderTitleChanged)
                .width(Fill),
            row![
                commit_btn.style(button::primary),
                button("Cancel")
                    .on_press(Message::SendToCreateFolderToggle)
                    .style(button::secondary),
            ]
            .spacing(vox_theme::SPACING),
        ]
        .spacing(4)
        .into()
    } else {
        let mut choices: Vec<FolderChoice> = vec![FolderChoice::Root];
        choices.extend(modal.folders.iter().cloned().map(FolderChoice::Existing));
        let selected = match &modal.selected_folder {
            Some(f) => Some(FolderChoice::Existing(f.clone())),
            None => Some(FolderChoice::Root),
        };
        column![
            text("Folder").size(12u32),
            pick_list(choices, selected, |choice| {
                Message::SendToFolderSelected(match choice {
                    FolderChoice::Root => None,
                    FolderChoice::Existing(f) => Some(f),
                })
            })
            .width(Fill),
            button("+ Create new folder")
                .on_press(Message::SendToCreateFolderToggle)
                .style(button::text),
        ]
        .spacing(4)
        .into()
    };

    // ── Error / status line ──────────────────────────────────────────────
    let status: Element<'_, Message> = if let Some(ref err) = modal.error {
        text(err.clone())
            .size(11u32)
            .color(Color {
                r: 0.85,
                g: 0.25,
                b: 0.25,
                a: 1.0,
            })
            .into()
    } else if modal.sending {
        text("Sending…").size(11u32).into()
    } else {
        column![].into()
    };

    // ── Send / Cancel buttons ────────────────────────────────────────────
    let can_send =
        !modal.sending && modal.selected_workspace.is_some() && !modal.title.trim().is_empty();
    let send_btn = if can_send {
        button("Send").on_press(Message::ConfirmSendTo)
    } else {
        button("Send")
    };

    let buttons = row![
        button("Cancel")
            .on_press(Message::CloseSendToModal)
            .style(button::secondary),
        send_btn.style(button::primary),
    ]
    .spacing(vox_theme::SPACING);

    column![
        header,
        rule::horizontal(1),
        title_field,
        content_field,
        workspace_field,
        folder_field,
        status,
        buttons,
    ]
    .spacing(vox_theme::SECTION_SPACING)
    .padding(vox_theme::PADDING)
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
                text_input(
                    "Stored in plaintext — chmod 600 your config file",
                    &s.summarization.api_key
                )
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

    // ── Export (Affine) section ───────────────────────────────────────────
    let affine_section = section_column(
        "Export — AFFiNE",
        column![
            toggle_row(
                "Enable AFFiNE export",
                s.export.affine.enabled,
                Message::AffineEnabledToggled,
            ),
            stacked_text_input(
                "AFFiNE base URL",
                "e.g. https://app.affine.pro or https://affine.example.com",
                &s.export.affine.base_url,
                Message::AffineBaseUrlChanged,
            ),
            column![
                text("API token (required for AFFiNE Cloud)").size(13u32),
                text_input(
                    "ut_… — generate under Settings → Integrations",
                    &s.export.affine.api_token,
                )
                .on_input(Message::AffineApiTokenChanged)
                .secure(true)
                .width(Fill),
            ]
            .spacing(vox_theme::SPACING / 2.0),
            stacked_text_input(
                "Email (self-hosted only)",
                "Used when api_token is empty",
                &s.export.affine.email,
                Message::AffineEmailChanged,
            ),
            column![
                text("Password (self-hosted only)").size(13u32),
                text_input(
                    "Stored in plaintext — chmod 600 your config",
                    &s.export.affine.password
                )
                .on_input(Message::AffinePasswordChanged)
                .secure(true)
                .width(Fill),
            ]
            .spacing(vox_theme::SPACING / 2.0),
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
                affine_section,
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
        button("Export").on_press(Message::OpenExportModal),
        button("Send to…").on_press(Message::OpenSendToModal),
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
                col = col.push(text(format!("  \u{2022} {point}")).size(12u32));
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
                col = col.push(text(format!("  \u{2022} {decision}")).size(12u32));
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
        text(label).size(13u32).width(Length::FillPortion(1)),
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
        text(label).size(13u32).width(Length::FillPortion(3)),
        toggler(value).on_toggle(on_toggle),
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

    if is_dark { Theme::Dark } else { Theme::Light }
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
    run_with_page(Page::Settings, false)
}

/// Launch the GUI window opening to the given [`Page`].
///
/// When `select_latest` is `true` and the page is [`Page::Browser`], the
/// most recent session is automatically selected on startup.
///
/// # Errors
///
/// Returns an `iced::Error` if the window cannot be created.
pub fn run_with_page(page: Page, select_latest: bool) -> iced::Result {
    // Ignore error if already set (first writer wins).
    let _ = INITIAL_PAGE.set(page);
    let _ = SELECT_LATEST.set(select_latest);

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
        match vox_capture::PipeWireSource::enumerate_streams(&vox_capture::StreamFilter::default())
        {
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
