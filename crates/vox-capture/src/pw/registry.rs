//! PipeWire registry query for stream enumeration.
//!
//! [`list_streams`] performs a one-shot registry roundtrip to enumerate all
//! audio nodes visible to the running PipeWire session. It blocks the calling
//! thread (the PipeWire thread) until the daemon has flushed its initial
//! objects or a timeout elapses.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use pipewire::{context::ContextBox, main_loop::MainLoopBox, types::ObjectType};
use tracing::{debug, warn};

use crate::error::CaptureError;
use crate::types::{StreamInfo, StreamRole};

/// Perform a synchronous registry query and return all visible audio nodes.
///
/// This function creates a temporary PipeWire connection, collects the initial
/// global objects emitted by the daemon, waits up to `timeout` for the burst
/// to complete, and then tears down the connection.
///
/// It is intended to be called from the PipeWire thread (or any thread that
/// is not a Tokio worker), but it is also safe to call from unit tests
/// because it spins its own event loop.
///
/// # Errors
///
/// Returns [`CaptureError::Connection`] if the PipeWire daemon is unreachable.
pub fn list_streams(timeout: Duration) -> Result<Vec<StreamInfo>, CaptureError> {
    pipewire::init();

    // Use MainLoopBox::new (the owned smart-pointer constructor).
    let main_loop = MainLoopBox::new(None).map_err(|e| {
        CaptureError::Connection(format!("failed to create MainLoop for enumeration: {e}"))
    })?;

    // ContextBox::new takes &Loop (from main_loop.loop_()), plus Option<PropertiesBox>.
    let context = ContextBox::new(main_loop.loop_(), None).map_err(|e| {
        CaptureError::Connection(format!("failed to create Context for enumeration: {e}"))
    })?;

    let core = context.connect(None).map_err(|e| {
        CaptureError::Connection(format!(
            "failed to connect to PipeWire for enumeration: {e}"
        ))
    })?;

    // Accumulate discovered nodes here.
    let nodes: std::rc::Rc<std::cell::RefCell<HashMap<u32, StreamInfo>>> =
        std::rc::Rc::new(std::cell::RefCell::new(HashMap::new()));

    let nodes_clone = std::rc::Rc::clone(&nodes);
    let ml = main_loop.loop_();

    let registry = core
        .get_registry()
        .map_err(|e| CaptureError::Connection(format!("failed to get registry: {e}")))?;

    // The global callback receives &GlobalObject<&spa::utils::dict::DictRef>.
    // global.props is Option<&spa::utils::dict::DictRef>; access keys via .get().
    let _listener = registry
        .add_listener_local()
        .global(move |global| {
            if global.type_ != ObjectType::Node {
                return;
            }
            // global.props is Option<&spa::utils::dict::DictRef>
            let props = match global.props {
                Some(p) => p,
                None => return,
            };

            let name = props
                .get("node.name")
                .or_else(|| props.get("object.id"))
                .unwrap_or("unknown")
                .to_owned();
            let description = props
                .get("node.description")
                .or_else(|| props.get("node.nick"))
                .map(str::to_owned);
            let app = props.get("application.name").map(str::to_owned);
            let media_class = props.get("media.class").map(str::to_owned);

            let suggested_role = media_class.as_deref().and_then(suggest_role);

            let info = StreamInfo {
                node_id: global.id,
                name,
                description,
                application_name: app,
                media_class,
                suggested_role,
            };
            debug!(node_id = global.id, name = %info.name, "discovered node");
            nodes_clone.borrow_mut().insert(global.id, info);
        })
        .register();

    // Register the done listener BEFORE issuing the sync, so we cannot miss
    // the done event if the daemon responds very quickly.
    let deadline = Instant::now() + timeout;
    let done = std::rc::Rc::new(std::cell::Cell::new(false));
    let done_clone = std::rc::Rc::clone(&done);

    // core done callback receives (u32, AsyncSeq) in v0.9.2.
    let _core_listener = core
        .add_listener_local()
        .done(move |_id, _seq| {
            done_clone.set(true);
        })
        .register();

    // Issue a sync roundtrip. When the daemon processes it, the `done`
    // callback fires and `done` becomes true.
    let _pending = core
        .sync(0)
        .map_err(|e| CaptureError::Connection(format!("failed to issue PipeWire sync: {e}")))?;

    while !done.get() && Instant::now() < deadline {
        ml.iterate(Duration::from_millis(10));
    }

    if !done.get() {
        warn!("PipeWire registry enumeration timed out after {timeout:?}");
    }

    let result = nodes.borrow().values().cloned().collect();
    Ok(result)
}

/// Heuristically assign a [`StreamRole`] based on the PipeWire media class.
fn suggest_role(media_class: &str) -> Option<StreamRole> {
    if media_class.contains("Source") {
        Some(StreamRole::Microphone)
    } else if media_class.contains("Stream/Input/Audio") {
        Some(StreamRole::Application)
    } else {
        None
    }
}
