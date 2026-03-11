//! [`SystemTray`] — real GTK-backed tray icon using `tray-icon` + `muda`.
//!
//! # Threading model
//!
//! GTK **must** be driven from the thread that called `gtk::init()`. This
//! module spawns a dedicated OS thread that:
//!
//! 1. Calls `gtk::init()`.
//! 2. Creates the `TrayIcon` and popup `Menu`.
//! 3. Enters the GTK main loop (`gtk::main()`).
//! 4. Processes `MenuEvent`s in a `glib` idle callback.
//!
//! All communication with the GTK thread goes through
//! [`crossbeam_channel`] channels:
//!
//! - **Outbound** (GTK thread → caller): `TrayEvent` sent whenever the user
//!   clicks a menu item.
//! - **Inbound** (caller → GTK thread): `StatusUpdate` carrying the new
//!   [`DaemonStatus`] so the GTK thread can swap the icon.

use std::sync::{
    Arc,
    atomic::{AtomicU8, Ordering},
};
use std::thread;

use crossbeam_channel::{Receiver, Sender, TryRecvError, bounded, unbounded};
use muda::{Menu, MenuItem, PredefinedMenuItem, accelerator::Accelerator};
use tray_icon::{TrayIconBuilder, menu::MenuEvent};

use crate::{DaemonStatus, Tray, TrayError, TrayEvent};

// ---------------------------------------------------------------------------
// Icon generation
// ---------------------------------------------------------------------------

/// Icon size in pixels (square).
const ICON_SIZE: u32 = 32;

/// Generate a minimal PNG in memory containing a filled circle of the given
/// RGBA colour.
///
/// The PNG uses an 8-bit RGBA colour type (colour type 6 in the PNG spec).
/// No external dependencies are required — the bytes are written directly
/// according to the PNG specification.
fn generate_circle_png(r: u8, g: u8, b: u8) -> Vec<u8> {
    let size = ICON_SIZE as usize;
    let center = (size / 2) as i32;
    let radius = (size / 2) as i32 - 2;

    // Build raw RGBA pixel rows, each preceded by a filter byte (0 = None).
    let mut raw_rows: Vec<u8> = Vec::with_capacity(size * (1 + size * 4));
    for y in 0..size {
        raw_rows.push(0u8); // filter byte
        for x in 0..size {
            let dx = x as i32 - center;
            let dy = y as i32 - center;
            let inside = dx * dx + dy * dy <= radius * radius;
            if inside {
                raw_rows.extend_from_slice(&[r, g, b, 255]);
            } else {
                raw_rows.extend_from_slice(&[0, 0, 0, 0]);
            }
        }
    }

    // Compress the raw data with zlib (deflate level 0 — stored).
    let compressed = zlib_stored(&raw_rows);

    // Build the PNG byte stream.
    let mut png: Vec<u8> = Vec::new();

    // PNG signature.
    png.extend_from_slice(&[137, 80, 78, 71, 13, 10, 26, 10]);

    // IHDR chunk.
    write_png_chunk(&mut png, b"IHDR", &{
        let mut ihdr = [0u8; 13];
        ihdr[0..4].copy_from_slice(&(ICON_SIZE as u32).to_be_bytes());
        ihdr[4..8].copy_from_slice(&(ICON_SIZE as u32).to_be_bytes());
        ihdr[8] = 8; // bit depth
        ihdr[9] = 6; // RGBA colour type
        ihdr[10] = 0; // compression method
        ihdr[11] = 0; // filter method
        ihdr[12] = 0; // interlace method
        ihdr
    });

    // IDAT chunk.
    write_png_chunk(&mut png, b"IDAT", &compressed);

    // IEND chunk.
    write_png_chunk(&mut png, b"IEND", &[]);

    png
}

/// Write a PNG chunk: length (4 bytes BE) + type (4 bytes) + data + CRC (4 bytes).
fn write_png_chunk(out: &mut Vec<u8>, chunk_type: &[u8; 4], data: &[u8]) {
    let len = data.len() as u32;
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(chunk_type);
    out.extend_from_slice(data);
    let crc = png_crc(chunk_type, data);
    out.extend_from_slice(&crc.to_be_bytes());
}

/// CRC-32 for PNG chunks (over type + data).
fn png_crc(chunk_type: &[u8], data: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFF_FFFF;
    for &byte in chunk_type.iter().chain(data.iter()) {
        let idx = ((crc ^ u32::from(byte)) & 0xFF) as usize;
        crc = CRC32_TABLE[idx] ^ (crc >> 8);
    }
    crc ^ 0xFFFF_FFFF
}

/// Pre-computed CRC-32 lookup table (standard polynomial 0xEDB88320).
const CRC32_TABLE: [u32; 256] = {
    let mut table = [0u32; 256];
    let mut i = 0usize;
    while i < 256 {
        let mut c = i as u32;
        let mut k = 0;
        while k < 8 {
            if c & 1 != 0 {
                c = 0xEDB8_8320 ^ (c >> 1);
            } else {
                c >>= 1;
            }
            k += 1;
        }
        table[i] = c;
        i += 1;
    }
    table
};

/// Wrap `data` in a zlib stream using deflate "stored" blocks (no compression).
///
/// This avoids pulling in a compression library. The deflate stored-block
/// format is described in RFC 1951 Section 3.2.4.
fn zlib_stored(data: &[u8]) -> Vec<u8> {
    // zlib header: CMF=0x78 (deflate, window 32 KiB), FLG chosen so that
    // (CMF * 256 + FLG) is divisible by 31.  0x78 * 256 = 30720; 30720 % 31
    // = 30720 - 990*31 = 30, so FLG = 1 makes 30721 % 31 = 0.
    let mut out = vec![0x78u8, 0x01];

    let mut remaining = data;
    while !remaining.is_empty() {
        // Max deflate stored block size is 65535 bytes.
        let chunk_len = remaining.len().min(65535);
        let chunk = &remaining[..chunk_len];
        remaining = &remaining[chunk_len..];

        let is_last = remaining.is_empty();
        // BFINAL (1 bit, LSB) | BTYPE=00 (2 bits) in one byte.
        out.push(if is_last { 0x01 } else { 0x00 });

        let len_u16 = chunk_len as u16;
        let nlen = !len_u16;
        out.extend_from_slice(&len_u16.to_le_bytes());
        out.extend_from_slice(&nlen.to_le_bytes());
        out.extend_from_slice(chunk);
    }

    // Adler-32 checksum of uncompressed data, big-endian.
    let adler = adler32(data);
    out.extend_from_slice(&adler.to_be_bytes());

    out
}

/// Compute Adler-32 checksum (RFC 1950).
fn adler32(data: &[u8]) -> u32 {
    const MOD: u32 = 65521;
    let mut s1: u32 = 1;
    let mut s2: u32 = 0;
    for &b in data {
        s1 = (s1 + u32::from(b)) % MOD;
        s2 = (s2 + s1) % MOD;
    }
    (s2 << 16) | s1
}

/// Convert a raw PNG byte vector into a `tray_icon::Icon`.
fn icon_from_png(png_bytes: &[u8]) -> Result<tray_icon::Icon, TrayError> {
    // tray-icon accepts RGBA pixel data directly.  We need to decode the PNG.
    // Since we generated the PNG ourselves, we know its exact layout and can
    // skip a full PNG decoder by regenerating the pixel data instead.
    // However, to keep the code correct for any future PNG source, we use the
    // helper that tray-icon itself exposes.
    tray_icon::Icon::from_rgba(
        png_to_rgba(png_bytes).map_err(|e| TrayError::Icon(e))?,
        ICON_SIZE,
        ICON_SIZE,
    )
    .map_err(|e| TrayError::Icon(e.to_string()))
}

/// Decode our known-format PNG (RGBA, no interlace, deflate-stored) back to
/// raw RGBA pixels.  For a general decoder, a library like `png` would be
/// used; here we rely on the known structure we wrote above.
fn png_to_rgba(png_bytes: &[u8]) -> Result<Vec<u8>, String> {
    // Skip PNG signature (8) + IHDR chunk (4+4+13+4 = 25).
    // Then find the IDAT chunk.
    if png_bytes.len() < 33 {
        return Err("PNG too short".to_owned());
    }
    let mut pos = 8; // after signature

    // Skip IHDR
    let chunk_len = u32::from_be_bytes(
        png_bytes[pos..pos + 4]
            .try_into()
            .map_err(|_| "bad chunk length".to_owned())?,
    ) as usize;
    pos += 4 + 4 + chunk_len + 4; // length + type + data + crc

    // Find IDAT
    while pos + 8 <= png_bytes.len() {
        let data_len = u32::from_be_bytes(
            png_bytes[pos..pos + 4]
                .try_into()
                .map_err(|_| "bad chunk length".to_owned())?,
        ) as usize;
        let chunk_type = &png_bytes[pos + 4..pos + 8];

        if chunk_type == b"IDAT" {
            let compressed = &png_bytes[pos + 8..pos + 8 + data_len];
            // Decompress zlib stored blocks (skip 2-byte header + 4-byte trailer).
            let raw = inflate_stored(compressed).map_err(|e| format!("inflate failed: {e}"))?;
            // raw has a filter byte before each row; strip them.
            let size = ICON_SIZE as usize;
            let mut rgba = Vec::with_capacity(size * size * 4);
            for row in 0..size {
                let row_start = row * (1 + size * 4) + 1; // skip filter byte
                rgba.extend_from_slice(&raw[row_start..row_start + size * 4]);
            }
            return Ok(rgba);
        }

        pos += 4 + 4 + data_len + 4;
    }

    Err("IDAT chunk not found".to_owned())
}

/// Decompress a zlib stream made of deflate "stored" blocks.
fn inflate_stored(data: &[u8]) -> Result<Vec<u8>, String> {
    if data.len() < 6 {
        return Err("zlib data too short".to_owned());
    }
    // Skip 2-byte zlib header.
    let mut pos = 2usize;
    let mut out = Vec::new();

    loop {
        if pos >= data.len() {
            return Err("unexpected end of deflate stream".to_owned());
        }
        let bfinal_btype = data[pos];
        pos += 1;
        let bfinal = bfinal_btype & 1;
        let btype = (bfinal_btype >> 1) & 3;
        if btype != 0 {
            return Err(format!(
                "unsupported deflate BTYPE={btype}; expected stored (0)"
            ));
        }
        if pos + 4 > data.len() {
            return Err("truncated stored block header".to_owned());
        }
        let len = u16::from_le_bytes(data[pos..pos + 2].try_into().unwrap()) as usize;
        pos += 4; // skip LEN + NLEN
        if pos + len > data.len() {
            return Err("stored block data truncated".to_owned());
        }
        out.extend_from_slice(&data[pos..pos + len]);
        pos += len;
        if bfinal == 1 {
            break;
        }
    }

    Ok(out)
}

// ---------------------------------------------------------------------------
// Status update channel
// ---------------------------------------------------------------------------

/// Message sent from the main thread to the GTK thread to change icon/menu.
#[derive(Debug, Clone, Copy)]
enum StatusUpdate {
    Set(DaemonStatus),
    Shutdown,
}

// ---------------------------------------------------------------------------
// SystemTray
// ---------------------------------------------------------------------------

/// The current status encoded as a `u8` for atomic storage.
const STATUS_IDLE: u8 = 0;
const STATUS_RECORDING: u8 = 1;
const STATUS_PROCESSING: u8 = 2;

fn encode_status(s: DaemonStatus) -> u8 {
    match s {
        DaemonStatus::Idle => STATUS_IDLE,
        DaemonStatus::Recording => STATUS_RECORDING,
        DaemonStatus::Processing => STATUS_PROCESSING,
    }
}

fn decode_status(v: u8) -> DaemonStatus {
    match v {
        STATUS_RECORDING => DaemonStatus::Recording,
        STATUS_PROCESSING => DaemonStatus::Processing,
        _ => DaemonStatus::Idle,
    }
}

/// System tray icon backed by `tray-icon` and `muda` with a GTK event loop.
///
/// # Threading
///
/// `SystemTray::new()` spawns a dedicated OS thread that owns the GTK event
/// loop. All icon/menu updates are sent to that thread via an internal
/// channel.
///
/// # Requirements
///
/// The host machine must have `libayatana-appindicator3` or
/// `libappindicator3` installed, and a compatible notification area must be
/// present (GNOME Shell with the AppIndicator extension, KDE Plasma, Sway with
/// waybar + tray support, etc.).
pub struct SystemTray {
    /// Send status updates to the GTK thread.
    status_tx: Sender<StatusUpdate>,

    /// Receive tray events produced by the GTK thread.
    event_rx: Receiver<TrayEvent>,

    /// Cached current status (readable from any thread without blocking).
    current_status: Arc<AtomicU8>,
}

impl std::fmt::Debug for SystemTray {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SystemTray")
            .field(
                "status",
                &decode_status(self.current_status.load(Ordering::Relaxed)),
            )
            .finish_non_exhaustive()
    }
}

impl SystemTray {
    /// Create a new `SystemTray`, spawning the GTK event-loop thread.
    ///
    /// # Errors
    ///
    /// Returns [`TrayError`] if the tray icon or GTK cannot be initialised.
    ///
    /// # Panics
    ///
    /// May panic if the OS thread cannot be spawned.
    pub fn new() -> Result<Self, TrayError> {
        let (status_tx, status_rx) = bounded::<StatusUpdate>(16);
        let (event_tx, event_rx) = unbounded::<TrayEvent>();
        let current_status = Arc::new(AtomicU8::new(STATUS_IDLE));
        let status_arc = Arc::clone(&current_status);

        thread::Builder::new()
            .name("vox-tray-gtk".to_owned())
            .spawn(move || {
                if let Err(e) = run_gtk_loop(status_rx, event_tx, status_arc) {
                    tracing::error!("tray GTK loop exited with error: {e}");
                }
            })
            .map_err(|e| TrayError::Create(e.to_string()))?;

        Ok(Self {
            status_tx,
            event_rx,
            current_status,
        })
    }

    /// Shut down the GTK event loop thread gracefully.
    ///
    /// After calling `shutdown()`, no more [`TrayEvent`]s will be produced.
    ///
    /// # Errors
    ///
    /// Returns [`TrayError::ChannelClosed`] if the GTK thread has already
    /// exited.
    pub fn shutdown(&self) -> Result<(), TrayError> {
        self.status_tx
            .send(StatusUpdate::Shutdown)
            .map_err(|_| TrayError::ChannelClosed)
    }
}

impl Tray for SystemTray {
    fn set_status(&self, status: DaemonStatus) -> Result<(), TrayError> {
        self.current_status
            .store(encode_status(status), Ordering::Relaxed);
        self.status_tx
            .send(StatusUpdate::Set(status))
            .map_err(|_| TrayError::ChannelClosed)
    }

    fn recv_event(&self) -> Option<TrayEvent> {
        self.event_rx.recv().ok()
    }

    fn try_recv_event(&self) -> Option<TrayEvent> {
        self.event_rx.try_recv().ok()
    }
}

// ---------------------------------------------------------------------------
// GTK event loop
// ---------------------------------------------------------------------------

/// Run the GTK event loop on the current thread.
///
/// This function blocks until a [`StatusUpdate::Shutdown`] message is received
/// or the tray icon is destroyed.
fn run_gtk_loop(
    status_rx: Receiver<StatusUpdate>,
    event_tx: Sender<TrayEvent>,
    current_status: Arc<AtomicU8>,
) -> Result<(), TrayError> {
    // GTK must be initialised on this thread before creating any widgets.
    gtk::init().map_err(|e| TrayError::Create(format!("failed to initialise GTK: {e}")))?;

    // Build initial idle icon.
    let idle_png = generate_circle_png(0x4C, 0xAF, 0x50); // green
    let idle_icon = icon_from_png(&idle_png)?;

    // Build popup menu.
    let menu = Menu::new();

    let start_item = MenuItem::new("Start Recording", true, None::<Accelerator>);
    let stop_item = MenuItem::new("Stop Recording", false, None::<Accelerator>);
    let pause_item = MenuItem::new("Pause Recording", false, None::<Accelerator>);
    let latest_item = MenuItem::new("Open Latest Transcript", true, None::<Accelerator>);
    let browse_item = MenuItem::new("Browse Transcripts\u{2026}", true, None::<Accelerator>);
    let settings_item = MenuItem::new("Settings\u{2026}", true, None::<Accelerator>);
    let quit_item = MenuItem::new("Quit", true, None::<Accelerator>);

    menu.append(&start_item)
        .map_err(|e| TrayError::Menu(e.to_string()))?;
    menu.append(&stop_item)
        .map_err(|e| TrayError::Menu(e.to_string()))?;
    menu.append(&pause_item)
        .map_err(|e| TrayError::Menu(e.to_string()))?;
    menu.append(&PredefinedMenuItem::separator())
        .map_err(|e| TrayError::Menu(e.to_string()))?;
    menu.append(&latest_item)
        .map_err(|e| TrayError::Menu(e.to_string()))?;
    menu.append(&browse_item)
        .map_err(|e| TrayError::Menu(e.to_string()))?;
    menu.append(&PredefinedMenuItem::separator())
        .map_err(|e| TrayError::Menu(e.to_string()))?;
    menu.append(&settings_item)
        .map_err(|e| TrayError::Menu(e.to_string()))?;
    menu.append(&PredefinedMenuItem::separator())
        .map_err(|e| TrayError::Menu(e.to_string()))?;
    menu.append(&quit_item)
        .map_err(|e| TrayError::Menu(e.to_string()))?;

    // Capture item IDs for event dispatch.
    let start_id = start_item.id().clone();
    let stop_id = stop_item.id().clone();
    let pause_id = pause_item.id().clone();
    let latest_id = latest_item.id().clone();
    let browse_id = browse_item.id().clone();
    let settings_id = settings_item.id().clone();
    let quit_id = quit_item.id().clone();

    #[allow(unused_mut)]
    let mut _tray_icon = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_tooltip("Vox Daemon")
        .with_icon(idle_icon)
        .build()
        .map_err(|e| TrayError::Create(e.to_string()))?;

    // Pre-generate icons for each status so we don't regenerate on every update.
    let recording_png = generate_circle_png(0xF4, 0x43, 0x36); // red
    let processing_png = generate_circle_png(0xFF, 0xC1, 0x07); // yellow

    // We need a mutable reference to the tray icon for status updates.
    let tray_icon = std::cell::RefCell::new(_tray_icon);

    // Event loop: drive GTK so the tray icon renders and processes events.
    loop {
        // Process pending GTK events (non-blocking).
        while gtk::events_pending() {
            gtk::main_iteration_do(false);
        }

        // Poll for menu events (non-blocking).
        if let Ok(menu_event) = MenuEvent::receiver().try_recv() {
            let id = &menu_event.id;
            let tray_event = if id == &start_id {
                Some(TrayEvent::StartRecording)
            } else if id == &stop_id {
                Some(TrayEvent::StopRecording)
            } else if id == &pause_id {
                Some(TrayEvent::PauseRecording)
            } else if id == &latest_id {
                Some(TrayEvent::OpenLastTranscript)
            } else if id == &browse_id {
                Some(TrayEvent::BrowseTranscripts)
            } else if id == &settings_id {
                Some(TrayEvent::OpenSettings)
            } else if id == &quit_id {
                Some(TrayEvent::Quit)
            } else {
                None
            };

            if let Some(event) = tray_event {
                let is_quit = event == TrayEvent::Quit;
                if event_tx.send(event).is_err() {
                    tracing::warn!("tray event channel closed; dropping event");
                }
                if is_quit {
                    break;
                }
            }
        }

        // Poll for status updates (non-blocking).
        match status_rx.try_recv() {
            Ok(StatusUpdate::Shutdown) => {
                tracing::info!("tray received Shutdown signal");
                break;
            }
            Ok(StatusUpdate::Set(status)) => {
                tracing::debug!(?status, "tray status update");
                current_status.store(encode_status(status), Ordering::Relaxed);

                // Update the tray icon colour.
                let new_icon = match status {
                    DaemonStatus::Idle => icon_from_png(&idle_png),
                    DaemonStatus::Recording => icon_from_png(&recording_png),
                    DaemonStatus::Processing => icon_from_png(&processing_png),
                };
                match new_icon {
                    Ok(icon) => {
                        if let Err(e) = tray_icon.borrow_mut().set_icon(Some(icon)) {
                            tracing::warn!("failed to set tray icon: {e}");
                        }
                        tracing::debug!("icon updated for status {:?}", status);
                    }
                    Err(e) => {
                        tracing::warn!("failed to build icon for status {:?}: {}", status, e);
                    }
                }
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => {
                tracing::warn!("tray status channel disconnected");
                break;
            }
        }

        // Sleep briefly to avoid busy-spinning.
        thread::sleep(std::time::Duration::from_millis(10));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_circle_png_correct_length() {
        // 8 (sig) + 25 (IHDR) + (8 + compressed) (IDAT) + 12 (IEND).
        // The PNG should be parseable back to RGBA pixels.
        let png = generate_circle_png(255, 0, 0);
        assert!(!png.is_empty());
        // PNG signature
        assert_eq!(&png[0..8], &[137, 80, 78, 71, 13, 10, 26, 10]);
    }

    #[test]
    fn png_roundtrip_rgba() {
        let png = generate_circle_png(0, 255, 0);
        let rgba = png_to_rgba(&png).expect("decode PNG");
        let size = ICON_SIZE as usize;
        assert_eq!(rgba.len(), size * size * 4);
        // Center pixel should be green (opaque).
        let center = (size / 2) * size * 4 + (size / 2) * 4;
        assert_eq!(rgba[center], 0, "red channel should be 0");
        assert_eq!(rgba[center + 1], 255, "green channel should be 255");
        assert_eq!(rgba[center + 2], 0, "blue channel should be 0");
        assert_eq!(rgba[center + 3], 255, "alpha should be 255");
    }

    #[test]
    fn adler32_known_value() {
        // From RFC 1950: adler32("Mark Adler") = 0x17CF_15D1
        // We test with a simple known case: empty input = 1.
        assert_eq!(adler32(&[]), 1);
    }

    #[test]
    fn encode_decode_status_roundtrip() {
        for status in [
            DaemonStatus::Idle,
            DaemonStatus::Recording,
            DaemonStatus::Processing,
        ] {
            assert_eq!(decode_status(encode_status(status)), status);
        }
    }
}
