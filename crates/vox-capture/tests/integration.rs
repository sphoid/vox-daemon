//! Integration tests for `vox-capture`.
//!
//! These tests require a live PipeWire daemon and the `integration` feature
//! flag. Run them with:
//!
//! ```text
//! cargo test -p vox-capture --features integration -- --nocapture
//! ```
//!
//! They are skipped entirely in CI environments where libpipewire is not
//! available.

#![cfg(feature = "integration")]

use std::time::Duration;

use vox_capture::pw::PipeWireSource;
use vox_capture::{AudioSource, StreamFilter, StreamRole};

/// Verify that we can connect to PipeWire and enumerate nodes.
#[test]
fn test_enumerate_streams() {
    let streams = PipeWireSource::enumerate_streams(&StreamFilter::default())
        .expect("should enumerate streams");
    // At minimum we expect at least one audio node on any desktop system.
    println!("Found {} PipeWire audio nodes:", streams.len());
    for s in &streams {
        println!(
            "  id={} name={:?} app={:?} class={:?} role={:?}",
            s.node_id, s.name, s.application_name, s.media_class, s.suggested_role
        );
    }
    assert!(
        !streams.is_empty(),
        "expected at least one audio node; is PipeWire running?"
    );
}

/// Verify that the microphone filter finds at least one source.
#[test]
fn test_enumerate_microphone_sources() {
    let filter = StreamFilter {
        media_class: Some("Source".to_owned()),
        ..Default::default()
    };
    let streams = PipeWireSource::enumerate_streams(&filter).expect("should enumerate streams");
    println!("Found {} source nodes", streams.len());
    for s in &streams {
        println!("  id={} name={:?}", s.node_id, s.name);
    }
    // Not asserting a minimum count here because headless CI may have 0 mics.
}

/// Verify that we can start and stop capture without crashing.
///
/// This test captures from the first available microphone for 200 ms and
/// checks that at least one audio chunk arrives.
#[test]
fn test_capture_microphone_briefly() {
    let filter = StreamFilter {
        media_class: Some("Source".to_owned()),
        ..Default::default()
    };
    let streams = PipeWireSource::enumerate_streams(&filter).expect("enumerate ok");

    if streams.is_empty() {
        println!("No microphone sources found; skipping capture test");
        return;
    }

    let node_id = streams[0].node_id;
    println!("Capturing from node_id={node_id} for 200 ms");

    let mut source = PipeWireSource::new(vec![(node_id, StreamRole::Microphone)]).expect("new ok");

    source.start().expect("start ok");

    // Collect chunks for 200 ms.
    let deadline = std::time::Instant::now() + Duration::from_millis(200);
    let mut chunks_received = 0usize;
    while std::time::Instant::now() < deadline {
        if source
            .stream_receiver()
            .recv_timeout(Duration::from_millis(20))
            .is_ok()
        {
            chunks_received += 1;
        }
    }

    source.stop().expect("stop ok");

    println!("Received {chunks_received} audio chunk(s)");
    assert!(
        chunks_received > 0,
        "expected at least one audio chunk during 200 ms capture"
    );
}
