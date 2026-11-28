use super::{ClipboardBridge, ClipboardInput, TextClipboardBackend};
use crate::Outbound;
use ironrdp_cliprdr::backend::{ClipboardMessage, CliprdrBackend};
use ironrdp_cliprdr::pdu::{
    ClipboardFormat, ClipboardFormatId, FileDescriptor, OwnedFormatDataResponse,
};
use nyaterm_remote_desktop::RdpControlMessage;
use nyaterm_remote_desktop::clipboard::{
    ClipboardAccess, ClipboardCommand, ClipboardContent, ClipboardEvent,
};
use std::sync::atomic::Ordering;
use std::sync::{Arc, mpsc};

fn bridge() -> (
    Arc<ClipboardBridge>,
    mpsc::Receiver<ClipboardMessage>,
    mpsc::Receiver<Outbound>,
) {
    let (tx, rx) = mpsc::sync_channel(32);
    let bridge = ClipboardBridge::new("test".into(), tx, true);
    let (tx, messages) = mpsc::channel();
    bridge.state.lock().unwrap().input = Some(ClipboardInput::Test(tx));
    bridge.file_available.store(true, Ordering::SeqCst);
    (bridge, messages, rx)
}
fn authorize(
    bridge: &ClipboardBridge,
    epoch: u64,
    access: ClipboardAccess,
    root: &std::path::Path,
) {
    bridge
        .command(ClipboardCommand::Authorize {
            epoch,
            access,
            cache_root: root.into(),
        })
        .unwrap();
}
fn snapshot(bridge: &ClipboardBridge, epoch: u64, revision: u64, content: ClipboardContent) {
    bridge
        .command(ClipboardCommand::Snapshot {
            epoch,
            revision,
            content,
        })
        .unwrap();
}

#[test]
fn no_clipboard_access_before_authorization_or_after_revocation() {
    let (bridge, messages, _) = bridge();
    let temp = tempfile::tempdir().unwrap();
    snapshot(&bridge, 1, 1, ClipboardContent::Text("blocked".into()));
    assert!(messages.try_recv().is_err());
    authorize(&bridge, 1, ClipboardAccess::Active, temp.path());
    snapshot(&bridge, 1, 1, ClipboardContent::Text("allowed".into()));
    assert!(matches!(
        messages.try_recv().unwrap(),
        ClipboardMessage::SendInitiateCopy(_)
    ));
    authorize(&bridge, 2, ClipboardAccess::Disabled, temp.path());
    snapshot(&bridge, 1, 2, ClipboardContent::Text("stale".into()));
    assert!(messages.try_recv().is_err());
    assert!(bridge.local_text().is_empty());
}

#[test]
fn file_text_file_transitions_reannounce_the_same_files() {
    let (bridge, messages, _) = bridge();
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("file");
    std::fs::write(&path, b"fixture").unwrap();
    authorize(&bridge, 1, ClipboardAccess::Active, temp.path());
    snapshot(&bridge, 1, 1, ClipboardContent::Files(vec![path.clone()]));
    assert!(matches!(
        messages.try_recv().unwrap(),
        ClipboardMessage::SendInitiateFileCopy(_)
    ));
    snapshot(&bridge, 1, 2, ClipboardContent::Text("text".into()));
    assert!(matches!(
        messages.try_recv().unwrap(),
        ClipboardMessage::SendInitiateCopy(_)
    ));
    snapshot(&bridge, 1, 3, ClipboardContent::Files(vec![path]));
    assert!(matches!(
        messages.try_recv().unwrap(),
        ClipboardMessage::SendInitiateFileCopy(_)
    ));
    assert!(matches!(
        bridge.current_advertisement(),
        ClipboardMessage::SendInitiateFileCopy(_)
    ));
}

#[test]
fn revoked_remote_response_cannot_be_published_under_a_new_epoch() {
    let (bridge, messages, output) = bridge();
    let temp = tempfile::tempdir().unwrap();
    authorize(&bridge, 1, ClipboardAccess::Active, temp.path());
    let mut backend = TextClipboardBackend {
        bridge: bridge.clone(),
    };
    backend.on_remote_copy(&[ClipboardFormat::new(ClipboardFormatId::CF_UNICODETEXT)]);
    assert!(matches!(
        messages.try_recv().unwrap(),
        ClipboardMessage::SendInitiatePaste(_)
    ));
    assert!(matches!(output.try_recv().unwrap(), Outbound::Control(_)));
    authorize(&bridge, 2, ClipboardAccess::Disabled, temp.path());
    authorize(&bridge, 3, ClipboardAccess::Active, temp.path());
    backend.on_format_data_response(OwnedFormatDataResponse::new_unicode_string("old"));
    assert!(output.try_recv().is_err());
}

#[test]
fn drain_finishes_an_existing_copy_but_starts_no_new_one() {
    let (bridge, messages, output) = bridge();
    let temp = tempfile::tempdir().unwrap();
    authorize(&bridge, 1, ClipboardAccess::Active, temp.path());
    let mut backend = TextClipboardBackend {
        bridge: bridge.clone(),
    };
    backend.on_remote_copy(&[ClipboardFormat::new(ClipboardFormatId::CF_UNICODETEXT)]);
    messages.try_recv().unwrap();
    output.try_recv().unwrap();
    authorize(&bridge, 1, ClipboardAccess::Drain, temp.path());
    backend.on_format_data_response(OwnedFormatDataResponse::new_unicode_string("pending"));
    let Outbound::Control(message) = output.try_recv().unwrap() else {
        panic!("control");
    };
    assert!(matches!(
        *message,
        RdpControlMessage::ManagedClipboardEvent {
            event: ClipboardEvent::Ready { epoch: 1, .. },
            ..
        }
    ));
    backend.on_remote_copy(&[ClipboardFormat::new(ClipboardFormatId::CF_UNICODETEXT)]);
    assert!(messages.try_recv().is_err());
    assert!(output.try_recv().is_err());
}

#[test]
fn completed_cache_is_retained_and_only_relative_paths_are_published() {
    let (bridge, _, output) = bridge();
    let temp = tempfile::tempdir().unwrap();
    authorize(&bridge, 1, ClipboardAccess::Active, temp.path());
    bridge
        .files
        .lock()
        .unwrap()
        .begin_remote(&[FileDescriptor::new("empty").with_file_size(0)], None)
        .unwrap();
    bridge.complete_remote().unwrap();
    let Outbound::Control(message) = output.try_recv().unwrap() else {
        panic!("control");
    };
    let RdpControlMessage::ManagedClipboardEvent {
        event:
            ClipboardEvent::Ready {
                content: ClipboardContent::Files(paths),
                ..
            },
        ..
    } = *message
    else {
        panic!("file publication");
    };
    assert!(!paths[0].is_absolute());
    let path = temp.path().join(&paths[0]);
    assert!(path.exists());
    bridge.stop();
    assert!(
        path.exists(),
        "disconnect must preserve an already published cache"
    );
}

#[test]
fn late_chunks_from_a_cancelled_copy_do_not_fail_the_next_download() {
    use ironrdp_cliprdr::pdu::FileContentsResponse;
    let (bridge, _, _) = bridge();
    let temp = tempfile::tempdir().unwrap();
    authorize(&bridge, 1, ClipboardAccess::Active, temp.path());
    let mut files = bridge.files.lock().unwrap();
    let first = files
        .begin_remote(&[FileDescriptor::new("first").with_file_size(1)], None)
        .unwrap();
    let second = files
        .begin_remote(&[FileDescriptor::new("second").with_file_size(1)], None)
        .unwrap();
    let stale = FileContentsResponse::new_error(first.requests[0].stream_id);
    assert!(!files.handle_response(&stale).unwrap().completed);
    let current = FileContentsResponse::new_data_response(second.requests[0].stream_id, vec![42]);
    assert!(files.handle_response(&current).unwrap().completed);
    assert_eq!(files.remote.as_ref().unwrap().transferred_bytes, 1);
}

#[test]
fn cancellation_renewal_keeps_the_private_cache_directory() {
    let (bridge, _, _) = bridge();
    let temp = tempfile::tempdir().unwrap();
    authorize(&bridge, 1, ClipboardAccess::Active, temp.path());
    authorize(
        &bridge,
        2,
        ClipboardAccess::Disabled,
        std::path::Path::new(""),
    );
    authorize(&bridge, 2, ClipboardAccess::Active, temp.path());
    assert_eq!(
        bridge.files.lock().unwrap().cache_directory.as_deref(),
        Some(temp.path())
    );
}

#[test]
fn remote_copy_carries_the_last_observed_local_revision_without_echoing_publications() {
    let (bridge, messages, output) = bridge();
    let temp = tempfile::tempdir().unwrap();
    authorize(&bridge, 1, ClipboardAccess::Active, temp.path());
    bridge
        .command(ClipboardCommand::Observed {
            epoch: 1,
            revision: 7,
        })
        .unwrap();
    assert!(
        messages.try_recv().is_err(),
        "observations must not announce a local copy"
    );
    let mut backend = TextClipboardBackend {
        bridge: bridge.clone(),
    };
    backend.on_remote_copy(&[ClipboardFormat::new(ClipboardFormatId::CF_UNICODETEXT)]);
    let Outbound::Control(message) = output.try_recv().unwrap() else {
        panic!("control");
    };
    assert!(matches!(
        *message,
        RdpControlMessage::ManagedClipboardEvent {
            event: ClipboardEvent::Started {
                epoch: 1,
                revision: 7,
                ..
            },
            ..
        }
    ));
}
