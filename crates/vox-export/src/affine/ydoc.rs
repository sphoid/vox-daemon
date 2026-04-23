//! Yjs-CRDT building blocks for `AFFiNE`'s document format.
//!
//! `AFFiNE` stores each document as a Yjs `Doc` with two top-level maps:
//!
//! - `blocks` — keyed by block id, each value is a `Y.Map` with
//!   `sys:id`, `sys:flavour`, `sys:version`, `sys:parent`, `sys:children`,
//!   plus flavour-specific `prop:*` fields. The root flavour is
//!   `affine:page`, with `affine:surface` (required sibling) and
//!   `affine:note` (content container) underneath.
//! - `meta` — per-doc metadata (`id`, `title`, `createDate`, `tags`).
//!
//! Each workspace also has its own root doc (stored under the workspace id
//! itself) whose `meta.pages` array lists every doc in the workspace. New
//! docs only appear in the sidebar once they are registered there — see
//! [`build_workspace_meta_append`].

use yrs::{
    Any, Array, ArrayPrelim, Doc, GetString, Map, MapPrelim, Out, ReadTxn, Text, Transact,
    TransactionMut, updates::decoder::Decode,
};

use crate::error::ExportError;

// `AFFiNE` block schema version numbers, mirroring the constants used by the
// TypeScript implementation as of Affine 0.26.
const VERSION_PAGE: i64 = 2;
const VERSION_SURFACE: i64 = 5;
const VERSION_NOTE: i64 = 1;
const VERSION_PARAGRAPH: i64 = 1;
const VERSION_EMBED_LINKED_DOC: i64 = 1;

/// Set the Yjs-CRDT `sys:id`, `sys:flavour`, `sys:version`, `sys:parent`
/// fields that every `AFFiNE` block carries.
fn set_sys_fields(
    txn: &mut TransactionMut<'_>,
    block: &yrs::MapRef,
    id: &str,
    flavour: &str,
    version: i64,
) {
    block.insert(txn, "sys:id", id);
    block.insert(txn, "sys:flavour", flavour);
    block.insert(txn, "sys:version", version);
    block.insert(txn, "sys:parent", Any::Null);
}

/// Build a fresh `AFFiNE` document as a Yjs v1 update ready to be pushed via
/// `space:push-doc-update`.
///
/// The document contains a page → surface + note hierarchy, and if
/// `content_markdown` is non-empty, a single paragraph block holding the
/// markdown as plain text.
///
/// # Errors
///
/// Returns [`ExportError::Transport`] only if the underlying yrs transaction
/// cannot be opened — practically infallible for a fresh in-memory doc.
pub fn build_doc_update(
    doc_id: &str,
    title: &str,
    content_markdown: &str,
) -> Result<Vec<u8>, ExportError> {
    let doc = Doc::new();
    let blocks = doc.get_or_insert_map("blocks");
    let meta = doc.get_or_insert_map("meta");

    let page_id = uuid::Uuid::new_v4().to_string();
    let surface_id = uuid::Uuid::new_v4().to_string();
    let note_id = uuid::Uuid::new_v4().to_string();

    {
        let mut txn = doc.transact_mut();

        // ── affine:page (root) ───────────────────────────────────────────
        let page = blocks.insert(&mut txn, page_id.as_str(), MapPrelim::default());
        set_sys_fields(&mut txn, &page, &page_id, "affine:page", VERSION_PAGE);
        let title_text = page.insert(&mut txn, "prop:title", yrs::TextPrelim::new(""));
        title_text.insert(&mut txn, 0, title);
        let page_children: yrs::ArrayRef =
            page.insert(&mut txn, "sys:children", ArrayPrelim::default());

        // ── affine:surface (required sibling) ────────────────────────────
        let surface = blocks.insert(&mut txn, surface_id.as_str(), MapPrelim::default());
        set_sys_fields(
            &mut txn,
            &surface,
            &surface_id,
            "affine:surface",
            VERSION_SURFACE,
        );
        surface.insert(&mut txn, "sys:children", ArrayPrelim::default());
        let elements = surface.insert(&mut txn, "prop:elements", MapPrelim::default());
        elements.insert(&mut txn, "type", "$blocksuite:internal:native$");
        elements.insert(&mut txn, "value", MapPrelim::default());
        page_children.push_back(&mut txn, surface_id.clone());

        // ── affine:note (content container) ──────────────────────────────
        let note = blocks.insert(&mut txn, note_id.as_str(), MapPrelim::default());
        set_sys_fields(&mut txn, &note, &note_id, "affine:note", VERSION_NOTE);
        note.insert(&mut txn, "prop:displayMode", "both");
        note.insert(&mut txn, "prop:xywh", "[0,0,800,95]");
        note.insert(&mut txn, "prop:index", "a0");
        note.insert(&mut txn, "prop:hidden", false);
        let bg = note.insert(&mut txn, "prop:background", MapPrelim::default());
        bg.insert(&mut txn, "light", "#ffffff");
        bg.insert(&mut txn, "dark", "#252525");
        let note_children: yrs::ArrayRef =
            note.insert(&mut txn, "sys:children", ArrayPrelim::default());
        page_children.push_back(&mut txn, note_id.clone());

        // ── one affine:paragraph with the markdown body ──────────────────
        if !content_markdown.is_empty() {
            let para_id = uuid::Uuid::new_v4().to_string();
            let para = blocks.insert(&mut txn, para_id.as_str(), MapPrelim::default());
            set_sys_fields(
                &mut txn,
                &para,
                &para_id,
                "affine:paragraph",
                VERSION_PARAGRAPH,
            );
            para.insert(&mut txn, "sys:children", ArrayPrelim::default());
            para.insert(&mut txn, "prop:type", "text");
            let text = para.insert(&mut txn, "prop:text", yrs::TextPrelim::new(""));
            text.insert(&mut txn, 0, content_markdown);
            note_children.push_back(&mut txn, para_id);
        }

        // ── meta ────────────────────────────────────────────────────────
        meta.insert(&mut txn, "id", doc_id);
        meta.insert(&mut txn, "title", title);
        meta.insert(
            &mut txn,
            "createDate",
            chrono::Utc::now().timestamp_millis(),
        );
        meta.insert(&mut txn, "tags", ArrayPrelim::default());
    }

    let txn = doc.transact();
    let state_vec = yrs::StateVector::default();
    Ok(txn.encode_state_as_update_v1(&state_vec))
}

/// Build a workspace-meta delta that appends `{id, title, createDate, tags}`
/// to the workspace's root doc's `meta.pages` array so the new doc appears
/// in the sidebar.
///
/// Unlike [`build_doc_update`], this produces an update for an existing Yjs
/// doc — callers apply it by diffing against an empty state vector because
/// the client did not previously know the workspace doc's contents.
#[must_use]
pub fn build_workspace_meta_append(doc_id: &str, title: &str) -> Vec<u8> {
    let doc = Doc::new();
    let meta = doc.get_or_insert_map("meta");
    {
        let mut txn = doc.transact_mut();
        let pages: yrs::ArrayRef = meta.insert(&mut txn, "pages", ArrayPrelim::default());
        let entry = MapPrelim::default();
        let entry_ref = pages.push_back(&mut txn, entry);
        entry_ref.insert(&mut txn, "id", doc_id);
        entry_ref.insert(&mut txn, "title", title);
        entry_ref.insert(
            &mut txn,
            "createDate",
            chrono::Utc::now().timestamp_millis(),
        );
        entry_ref.insert(&mut txn, "tags", ArrayPrelim::default());
    }
    let txn = doc.transact();
    txn.encode_state_as_update_v1(&yrs::StateVector::default())
}

/// Extract the human-readable workspace name from a workspace root doc's
/// Yjs v1 update bytes (as returned by `space:load-doc`).
///
/// `AFFiNE` stores the workspace name as a plain string under
/// `meta.name`; newer clients store it as a `Y.Text`. Both shapes are
/// handled. Returns `None` if the update cannot be decoded or the field is
/// absent — callers should fall back to a placeholder (e.g.
/// [`super::graphql::fallback_workspace_name`]).
#[must_use]
pub fn extract_workspace_name(update_bytes: &[u8]) -> Option<String> {
    let update = yrs::Update::decode_v1(update_bytes).ok()?;
    let doc = Doc::new();
    {
        let mut txn = doc.transact_mut();
        txn.apply_update(update).ok()?;
    }
    let meta = doc.get_or_insert_map("meta");
    let txn = doc.transact();
    match meta.get(&txn, "name") {
        Some(Out::Any(Any::String(s))) => {
            let owned = s.to_string();
            if owned.is_empty() { None } else { Some(owned) }
        }
        Some(Out::YText(text)) => {
            let owned = text.get_string(&txn);
            if owned.is_empty() { None } else { Some(owned) }
        }
        _ => None,
    }
}

/// Build an `affine:embed-linked-doc` block appended to `parent_doc_id`'s
/// content, pointing at `child_doc_id`.
///
/// `AFFiNE` renders these as inline links/cards in the parent doc. Adding one
/// is how we visually "nest" a new doc under an existing parent.
///
/// # Errors
///
/// Returns [`ExportError::Transport`] if the yrs transaction cannot open
/// (practically infallible).
pub fn build_embed_linked_doc_block(
    _parent_doc_id: &str,
    child_doc_id: &str,
) -> Result<Vec<u8>, ExportError> {
    let doc = Doc::new();
    let blocks = doc.get_or_insert_map("blocks");

    let block_id = uuid::Uuid::new_v4().to_string();
    {
        let mut txn = doc.transact_mut();
        let block = blocks.insert(&mut txn, block_id.as_str(), MapPrelim::default());
        set_sys_fields(
            &mut txn,
            &block,
            &block_id,
            "affine:embed-linked-doc",
            VERSION_EMBED_LINKED_DOC,
        );
        block.insert(&mut txn, "sys:children", ArrayPrelim::default());
        block.insert(&mut txn, "prop:pageId", child_doc_id);
        block.insert(&mut txn, "prop:params", MapPrelim::default());
    }
    let txn = doc.transact();
    Ok(txn.encode_state_as_update_v1(&yrs::StateVector::default()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_doc_update_produces_non_empty_bytes() {
        let u = build_doc_update("doc1", "Hello", "body here").expect("build");
        assert!(!u.is_empty());
        // Yjs v1 updates start with a non-zero structure section length.
        assert!(u.len() > 10, "update unexpectedly short: {} bytes", u.len());
    }

    #[test]
    fn build_doc_update_with_empty_content_still_valid() {
        let u = build_doc_update("doc1", "Hello", "").expect("build");
        assert!(!u.is_empty());
    }

    #[test]
    fn workspace_meta_append_is_non_empty() {
        let u = build_workspace_meta_append("doc1", "Hello");
        assert!(!u.is_empty());
    }

    #[test]
    fn embed_linked_doc_is_non_empty() {
        let u = build_embed_linked_doc_block("parent", "child").expect("build");
        assert!(!u.is_empty());
    }

    #[test]
    fn extract_workspace_name_reads_plain_string() {
        // Build a synthetic workspace root doc with `meta.name` set as a
        // plain string, encode it, then confirm extract_workspace_name
        // reads it back.
        let doc = Doc::new();
        let meta = doc.get_or_insert_map("meta");
        {
            let mut txn = doc.transact_mut();
            meta.insert(&mut txn, "name", "My Workspace");
        }
        let bytes = doc
            .transact()
            .encode_state_as_update_v1(&yrs::StateVector::default());

        assert_eq!(
            extract_workspace_name(&bytes).as_deref(),
            Some("My Workspace")
        );
    }

    #[test]
    fn extract_workspace_name_reads_ytext_value() {
        // Some AFFiNE clients store the name as a Y.Text instead of a plain
        // string. Confirm we handle both.
        let doc = Doc::new();
        let meta = doc.get_or_insert_map("meta");
        {
            let mut txn = doc.transact_mut();
            let text = meta.insert(&mut txn, "name", yrs::TextPrelim::new(""));
            text.insert(&mut txn, 0, "Rich Name");
        }
        let bytes = doc
            .transact()
            .encode_state_as_update_v1(&yrs::StateVector::default());

        assert_eq!(extract_workspace_name(&bytes).as_deref(), Some("Rich Name"));
    }

    #[test]
    fn extract_workspace_name_absent_returns_none() {
        // A doc with no `meta.name` at all.
        let doc = Doc::new();
        {
            let _ = doc.get_or_insert_map("meta");
        }
        let bytes = doc
            .transact()
            .encode_state_as_update_v1(&yrs::StateVector::default());
        assert!(extract_workspace_name(&bytes).is_none());
    }

    #[test]
    fn extract_workspace_name_rejects_malformed_bytes() {
        assert!(extract_workspace_name(b"not a yjs update").is_none());
    }

    #[test]
    fn updates_can_be_applied_to_fresh_doc() {
        use yrs::updates::decoder::Decode;
        let update_bytes = build_doc_update("doc1", "T", "body").expect("build");
        let applied = Doc::new();
        let mut txn = applied.transact_mut();
        let update = yrs::Update::decode_v1(&update_bytes).expect("decode");
        txn.apply_update(update).expect("apply");
        drop(txn);

        let blocks = applied.get_or_insert_map("blocks");
        let txn = applied.transact();
        assert!(blocks.len(&txn) >= 3, "expected at least page/surface/note");
    }
}
