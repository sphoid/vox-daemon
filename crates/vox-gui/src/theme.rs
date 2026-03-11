//! Theme and styling constants for the Vox Daemon GUI.
//!
//! This module is only compiled when the `ui` feature is enabled.
//! It provides the colour palette, spacing constants, and font sizes used
//! across all views.

use iced::Color;

// ──────────────────────────────────────────────────────────────────────────────
// Speaker colours
// ──────────────────────────────────────────────────────────────────────────────

/// Colour used for the local user's transcript segments.
pub const SPEAKER_YOU_COLOR: Color = Color {
    r: 0.18,
    g: 0.52,
    b: 0.92,
    a: 1.0,
};

/// Colour used for remote participants' transcript segments.
pub const SPEAKER_REMOTE_COLOR: Color = Color {
    r: 0.18,
    g: 0.72,
    b: 0.42,
    a: 1.0,
};

/// Colour used for any speaker whose label does not match a known role.
pub const SPEAKER_UNKNOWN_COLOR: Color = Color {
    r: 0.60,
    g: 0.60,
    b: 0.60,
    a: 1.0,
};

// ──────────────────────────────────────────────────────────────────────────────
// Spacing & sizing
// ──────────────────────────────────────────────────────────────────────────────

/// Standard padding used inside cards and panels (pixels).
///
/// Stored as `f32` so it can be passed to both `spacing` (`impl Into<Pixels>`)
/// and `padding` (`impl Into<Padding>`) without conversion.
pub const PADDING: f32 = 12.0;

/// Standard spacing between sibling widgets (pixels).
pub const SPACING: f32 = 8.0;

/// Large spacing used between sections (pixels).
pub const SECTION_SPACING: f32 = 20.0;

/// Minimum width of the settings window (pixels).
pub const WINDOW_MIN_WIDTH: u32 = 640;

/// Minimum height of the settings window (pixels).
pub const WINDOW_MIN_HEIGHT: u32 = 480;

// ──────────────────────────────────────────────────────────────────────────────
// Helpers
// ──────────────────────────────────────────────────────────────────────────────

/// Return the display colour for a given speaker label.
///
/// `"You"` → [`SPEAKER_YOU_COLOR`], `"Remote"` (or any remote variant) →
/// [`SPEAKER_REMOTE_COLOR`], anything else → [`SPEAKER_UNKNOWN_COLOR`].
#[must_use]
pub fn speaker_color(speaker: &str) -> Color {
    match speaker {
        "You" => SPEAKER_YOU_COLOR,
        s if s.starts_with("Remote") || s.starts_with("Speaker") => SPEAKER_REMOTE_COLOR,
        _ => SPEAKER_UNKNOWN_COLOR,
    }
}
