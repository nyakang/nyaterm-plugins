const MAX_CLIPBOARD_TEXT_BYTES: usize = 4 * 1024 * 1024;
use nyaterm_remote_desktop::clipboard::{
    ClipboardAccess, ClipboardCommand, ClipboardContent, ClipboardEvent,
};
use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

use ironrdp_client::rdp::RdpInputSender;
use ironrdp_cliprdr::backend::{ClipboardMessage, CliprdrBackend};
use ironrdp_cliprdr::pdu::{
    ClipboardFormat, ClipboardFormatId, ClipboardGeneralCapabilityFlags, FORMAT_NAME_FILE_LIST,
    FileContentsRequest, FileContentsResponse, FormatDataRequest, FormatDataResponse, LockDataId,
    OwnedFormatDataResponse,
};
use ironrdp_cliprdr::{Cliprdr, CliprdrClient};
use ironrdp_core::impl_as_any;
use nyaterm_remote_desktop::{
    RdpClipboardTransferProgress, RdpClipboardTransferStatus, RdpControlMessage,
};

use super::Outbound;

mod fetch;
mod files;

use fetch::{FetchKind, FetchReply, RemoteFetchState};
use files::{FileState, build_local_snapshot, file_list_fingerprint};

const REMOTE_FETCH_TIMEOUT: Duration = Duration::from_secs(5);

pub(super) struct ClipboardBridge {
    session_id: String,
    output_tx: mpsc::SyncSender<Outbound>,
    state: Mutex<ClipboardState>,
    files: Mutex<FileState>,
    fetch: Mutex<RemoteFetchState>,
    file_enabled: bool,
    file_available: AtomicBool,
    watcher_started: AtomicBool,
    shutdown: AtomicBool,
    managed: Mutex<ManagedState>,
    operations: Mutex<()>,
    commands: Mutex<Option<mpsc::SyncSender<ClipboardCommand>>>,
}

struct ManagedState {
    epoch: u64,
    access: ClipboardAccess,
    request: u64,
    revision: u64,
}
impl Default for ManagedState {
    fn default() -> Self {
        Self {
            epoch: 0,
            access: ClipboardAccess::Disabled,
            request: 0,
            revision: 0,
        }
    }
}

#[derive(Default)]
struct ClipboardState {
    local_text: String,
    local_files: bool,
    input: Option<ClipboardInput>,
}

#[derive(Clone)]
enum ClipboardInput {
    Rdp(RdpInputSender),
    #[cfg(test)]
    Test(mpsc::Sender<ClipboardMessage>),
}

impl ClipboardInput {
    fn send_clipboard(&self, message: ClipboardMessage) -> anyhow::Result<()> {
        match self {
            Self::Rdp(sender) => sender.send_clipboard(message).map_err(|_| ()),
            #[cfg(test)]
            Self::Test(sender) => sender.send(message).map_err(|_| ()),
        }
        .map_err(|_| anyhow::anyhow!("IronRDP clipboard input channel closed"))
    }
}

impl fmt::Debug for ClipboardBridge {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ClipboardBridge")
            .field("session_id", &self.session_id)
            .finish_non_exhaustive()
    }
}

impl ClipboardBridge {
    pub(super) fn new(
        session_id: String,
        output_tx: mpsc::SyncSender<Outbound>,
        file_enabled: bool,
    ) -> Arc<Self> {
        files::cleanup_stale_cache(&files::cache_root(), files::CACHE_RETENTION);
        Arc::new(Self {
            session_id,
            output_tx,
            state: Mutex::new(ClipboardState::default()),
            files: Mutex::new(FileState::default()),
            fetch: Mutex::new(RemoteFetchState::default()),
            file_enabled,
            file_available: AtomicBool::new(false),
            watcher_started: AtomicBool::new(false),
            shutdown: AtomicBool::new(false),
            managed: Mutex::new(ManagedState::default()),
            operations: Mutex::new(()),
            commands: Mutex::new(None),
        })
    }

    pub(super) fn set_input_sender(&self, sender: RdpInputSender) {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .input = Some(ClipboardInput::Rdp(sender));
    }

    pub(super) fn set_local_text(&self, text: String) -> anyhow::Result<()> {
        if text.len() > MAX_CLIPBOARD_TEXT_BYTES {
            anyhow::bail!("clipboard text exceeds the 4 MiB limit");
        }
        let sender = {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            state.local_text = text;
            state.local_files = false;
            state.input.clone()
        };
        if let Some(sender) = sender {
            sender.send_clipboard(ClipboardMessage::SendInitiateCopy(vec![
                ClipboardFormat::new(ClipboardFormatId::CF_UNICODETEXT),
            ]))?;
        }
        Ok(())
    }

    pub(super) fn cliprdr_client(self: &Arc<Self>) -> CliprdrClient {
        Cliprdr::new(Box::new(TextClipboardBackend {
            bridge: Arc::clone(self),
        }))
    }

    fn send_clipboard_message(&self, message: ClipboardMessage) {
        let sender = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .input
            .clone();
        if let Some(sender) = sender {
            let _ = sender.send_clipboard(message);
        }
    }

    fn send_paste(&self, format: ClipboardFormatId) {
        self.send_clipboard_message(ClipboardMessage::SendInitiatePaste(format));
    }

    fn queue_remote_fetch(&self, desired: Option<(FetchKind, ClipboardFormatId)>) {
        let next = self
            .fetch
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .begin_format_list(desired);
        if let Some(format) = next {
            self.send_paste(format);
        }
    }

    fn finish_remote_fetch(&self, kind: FetchKind, is_error: bool) -> FetchReply {
        let reply = self
            .fetch
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .finish_reply(kind, is_error);
        if let Some(format) = reply.next {
            self.send_paste(format);
        }
        reply
    }

    pub(super) fn check_remote_fetch_timeout(&self) {
        self.expire_remote_fetch(REMOTE_FETCH_TIMEOUT);
    }

    fn expire_remote_fetch(&self, timeout: Duration) {
        let timed_out = self
            .fetch
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .expire(timeout);
        if timed_out {
            let _ = self.output_tx.send(Outbound::control(RdpControlMessage::Error {
                session_id: self.session_id.clone(),
                error: nyaterm_remote_desktop::RdpError::new(
                    nyaterm_remote_desktop::RdpErrorKind::Clipboard,
                    "RDP clipboard response timed out; waiting for the outstanding response. Reconnect if clipboard copies remain unavailable.",
                ),
                fatal: false,
            }));
        }
    }

    fn emit(&self, event: ClipboardEvent) -> bool {
        let message = RdpControlMessage::ManagedClipboardEvent {
            session_id: self.session_id.clone(),
            event,
        };
        // Never let an oversized clipboard payload terminate the IPC writer.
        if nyaterm_remote_desktop::encode_control(&message).is_err() {
            let _ = self
                .output_tx
                .send(Outbound::control(RdpControlMessage::Error {
                    session_id: self.session_id.clone(),
                    error: nyaterm_remote_desktop::RdpError::new(
                        nyaterm_remote_desktop::RdpErrorKind::Clipboard,
                        "Clipboard contents exceed the IPC size limit",
                    ),
                    fatal: false,
                }));
            return false;
        }
        self.output_tx.send(Outbound::control(message)).is_ok()
    }

    fn publish_remote_text(&self, text: String) {
        if text.len() > MAX_CLIPBOARD_TEXT_BYTES {
            return;
        }
        let state = self.managed.lock().unwrap_or_else(|p| p.into_inner());
        if state.access != ClipboardAccess::Disabled {
            self.emit(ClipboardEvent::Ready {
                epoch: state.epoch,
                request: state.request,
                content: ClipboardContent::Text(text),
                transfer_id: None,
            });
        }
    }

    pub(super) fn enqueue(self: &Arc<Self>, command: ClipboardCommand) -> anyhow::Result<()> {
        if matches!(command, ClipboardCommand::Authorize { .. }) {
            return self.command(command);
        }
        let mut commands = self.commands.lock().unwrap_or_else(|p| p.into_inner());
        if commands.is_none() {
            let (tx, rx) = mpsc::sync_channel(8);
            let bridge = Arc::clone(self);
            std::thread::Builder::new()
                .name("rdp-clipboard-snapshots".into())
                .spawn(move || {
                    while let Ok(command) = rx.recv() {
                        if bridge.shutdown.load(Ordering::SeqCst) {
                            break;
                        }
                        let ClipboardCommand::Snapshot {
                            epoch, revision, ..
                        } = &command
                        else {
                            continue;
                        };
                        let (epoch, revision) = (*epoch, *revision);
                        let result = bridge.command(command);
                        bridge.emit(ClipboardEvent::Applied {
                            epoch,
                            revision,
                            accepted: result.is_ok(),
                        });
                        if let Err(error) = result {
                            let _ = bridge.output_tx.send(Outbound::control(
                                RdpControlMessage::Error {
                                    session_id: bridge.session_id.clone(),
                                    error: nyaterm_remote_desktop::RdpError::new(
                                        nyaterm_remote_desktop::RdpErrorKind::Clipboard,
                                        error.to_string(),
                                    ),
                                    fatal: false,
                                },
                            ));
                        }
                    }
                })?;
            *commands = Some(tx);
        }
        commands
            .as_ref()
            .expect("clipboard command queue")
            .try_send(command)
            .map_err(|_| anyhow::anyhow!("Clipboard snapshot queue is full or closed"))
    }

    pub(super) fn command(&self, command: ClipboardCommand) -> anyhow::Result<()> {
        // Directory scanning can be slow. Do it on the bounded snapshot worker,
        // outside the protocol/authorization gate, and revalidate before using it.
        let prepared = if let ClipboardCommand::Snapshot {
            epoch,
            content: ClipboardContent::Files(paths),
            ..
        } = &command
        {
            let state = self.managed.lock().unwrap_or_else(|p| p.into_inner());
            if *epoch != state.epoch || state.access != ClipboardAccess::Active {
                return Ok(());
            }
            let request = state.request;
            drop(state);
            Some((
                build_local_snapshot(paths).map_err(anyhow::Error::msg)?,
                request,
            ))
        } else {
            None
        };
        let _operation = self.operations.lock().unwrap_or_else(|p| p.into_inner());
        let mut state = self.managed.lock().unwrap_or_else(|p| p.into_inner());
        if prepared
            .as_ref()
            .is_some_and(|(_, request)| *request != state.request)
        {
            return Ok(());
        }
        match command {
            ClipboardCommand::Authorize {
                epoch,
                access,
                cache_root,
            } => {
                if epoch < state.epoch {
                    return Ok(());
                }
                if access != ClipboardAccess::Disabled && !cache_root.is_absolute() {
                    anyhow::bail!("clipboard cache root must be absolute");
                }
                if epoch != state.epoch || access == ClipboardAccess::Disabled {
                    self.cancel_remote("Clipboard ownership changed");
                    self.queue_remote_fetch(None);
                    state.revision = 0;
                    state.request = state.request.wrapping_add(1);
                    // Revocation also prevents serving old local file snapshots.
                    let mut files = self.files.lock().unwrap_or_else(|p| p.into_inner());
                    files.clear_local();
                    let mut clipboard = self.state.lock().unwrap_or_else(|p| p.into_inner());
                    clipboard.local_text.clear();
                    clipboard.local_files = false;
                    files.cache_directory = Some(cache_root.clone());
                }
                if access != ClipboardAccess::Disabled {
                    self.files
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .cache_directory = Some(cache_root);
                }
                state.epoch = epoch;
                state.access = access;
            }
            ClipboardCommand::Snapshot {
                epoch,
                revision,
                content,
            } => {
                if epoch != state.epoch
                    || state.access != ClipboardAccess::Active
                    || revision <= state.revision
                {
                    return Ok(());
                }
                self.cancel_remote("Local clipboard changed");
                self.queue_remote_fetch(None);
                match content {
                    ClipboardContent::Text(text) => self.set_local_text(text)?,
                    ClipboardContent::Files(paths) => {
                        if !self.file_available.load(Ordering::SeqCst) {
                            anyhow::bail!("Server does not support clipboard files");
                        }
                        let (snapshot, _) = prepared.expect("prepared file snapshot");
                        let hash = file_list_fingerprint(&paths);
                        let descriptors = self
                            .files
                            .lock()
                            .unwrap_or_else(|p| p.into_inner())
                            .set_local_snapshot(snapshot, hash);
                        self.state
                            .lock()
                            .unwrap_or_else(|p| p.into_inner())
                            .local_files = true;
                        self.send_clipboard_message(ClipboardMessage::SendInitiateFileCopy(
                            descriptors,
                        ));
                    }
                    ClipboardContent::Empty => {
                        let mut clipboard = self.state.lock().unwrap_or_else(|p| p.into_inner());
                        clipboard.local_files = false;
                        clipboard.local_text.clear();
                        drop(clipboard);
                        self.send_clipboard_message(ClipboardMessage::SendInitiateCopy(Vec::new()));
                    }
                }
                state.revision = revision;
            }
        }
        Ok(())
    }

    fn complete_remote(&self) -> Result<(), String> {
        let state = self.managed.lock().unwrap_or_else(|p| p.into_inner());
        if state.access == ClipboardAccess::Disabled {
            return Err("Clipboard ownership revoked".into());
        }
        let (id, paths) = self
            .files
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .complete_remote()?;
        if !self.emit(ClipboardEvent::Ready {
            epoch: state.epoch,
            request: state.request,
            content: ClipboardContent::Files(paths),
            transfer_id: Some(id),
        }) {
            return Err(
                "Clipboard file list exceeds IPC limits or the application disconnected".into(),
            );
        }
        Ok(())
    }

    fn local_text(&self) -> String {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .local_text
            .clone()
    }

    fn transfer_progress(
        &self,
        status: RdpClipboardTransferStatus,
        error: Option<String>,
    ) -> Option<RdpClipboardTransferProgress> {
        self.files.lock().ok().and_then(|files| {
            files
                .remote
                .as_ref()
                .map(|transfer| RdpClipboardTransferProgress {
                    id: transfer.id.clone(),
                    name: transfer.name.clone(),
                    status,
                    total_bytes: transfer.total_bytes,
                    transferred_bytes: transfer.transferred_bytes,
                    total_files: transfer.total_files,
                    completed_files: transfer.completed_files,
                    error,
                })
        })
    }

    fn send_transfer_progress(&self, progress: RdpClipboardTransferProgress) {
        let _ = self
            .output_tx
            .send(Outbound::control(RdpControlMessage::ClipboardTransfer {
                session_id: self.session_id.clone(),
                progress,
            }));
    }

    fn publish_transfer(&self, status: RdpClipboardTransferStatus, error: Option<String>) {
        if let Some(progress) = self.transfer_progress(status, error) {
            self.send_transfer_progress(progress);
        }
    }

    fn cancel_remote(&self, reason: &str) {
        self.publish_transfer(
            RdpClipboardTransferStatus::Cancelled,
            Some(reason.to_string()),
        );
        self.files
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .cancel_remote();
    }

    pub(super) fn cancel_transfer(&self, transfer_id: &str) {
        let progress = self.transfer_progress(
            RdpClipboardTransferStatus::Cancelled,
            Some("RDP clipboard transfer cancelled".to_string()),
        );
        let cancelled = self
            .files
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .cancel_if_id(transfer_id);
        if cancelled && let Some(progress) = progress {
            self.send_transfer_progress(progress);
        }
    }

    pub(super) fn stop(&self) {
        let _operation = self.operations.lock().unwrap_or_else(|p| p.into_inner());
        self.shutdown.store(true, Ordering::SeqCst);
        self.commands
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .take();
        self.cancel_remote("RDP session disconnected");
        self.files
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .shutdown();
    }

    fn start_watcher(self: &Arc<Self>) {
        if !self.file_enabled || self.watcher_started.swap(true, Ordering::SeqCst) {
            return;
        }
        let bridge = Arc::clone(self);
        std::thread::spawn(move || {
            while !bridge.shutdown.load(Ordering::SeqCst) {
                let operation = bridge.operations.lock().unwrap_or_else(|p| p.into_inner());
                let expired = bridge
                    .files
                    .lock()
                    .ok()
                    .is_some_and(|files| files.is_expired());
                if expired {
                    bridge.publish_transfer(
                        RdpClipboardTransferStatus::Cancelled,
                        Some("RDP clipboard transfer timed out".to_string()),
                    );
                    bridge
                        .files
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .expire();
                }
                drop(operation);
                std::thread::sleep(Duration::from_millis(750));
            }
        });
    }

    fn current_advertisement(&self) -> ClipboardMessage {
        let local_files = self
            .state
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .local_files;
        if local_files
            && self.file_available.load(Ordering::SeqCst)
            && let Some(descriptors) = self
                .files
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .current_descriptors()
        {
            return ClipboardMessage::SendInitiateFileCopy(descriptors);
        }
        self.initial_advertisement()
    }

    fn initial_advertisement(&self) -> ClipboardMessage {
        let formats = if self.local_text().is_empty() {
            Vec::new()
        } else {
            vec![ClipboardFormat::new(ClipboardFormatId::CF_UNICODETEXT)]
        };
        ClipboardMessage::SendInitiateCopy(formats)
    }

    fn advertise_initial(&self) {
        // The initial CLIPRDR FormatList is required even when the clipboard is empty.
        // File copy is valid only after IronRDP receives the FormatList response.
        self.send_clipboard_message(self.initial_advertisement());
    }

    fn advertise_current(&self) {
        self.send_clipboard_message(self.current_advertisement());
    }
}

#[derive(Debug)]
struct TextClipboardBackend {
    bridge: Arc<ClipboardBridge>,
}

impl_as_any!(TextClipboardBackend);

impl CliprdrBackend for TextClipboardBackend {
    fn temporary_directory(&self) -> &str {
        ".nyaterm-cliprdr"
    }

    fn client_capabilities(&self) -> ClipboardGeneralCapabilityFlags {
        let mut capabilities = ClipboardGeneralCapabilityFlags::USE_LONG_FORMAT_NAMES;
        if self.bridge.file_enabled {
            capabilities |= ClipboardGeneralCapabilityFlags::STREAM_FILECLIP_ENABLED
                | ClipboardGeneralCapabilityFlags::FILECLIP_NO_FILE_PATHS
                | ClipboardGeneralCapabilityFlags::CAN_LOCK_CLIPDATA;
        }
        capabilities
    }

    fn on_ready(&mut self) {
        self.bridge.advertise_current();
        self.bridge.start_watcher();
    }

    fn on_request_format_list(&mut self) {
        self.bridge.advertise_initial();
    }

    fn on_process_negotiated_capabilities(
        &mut self,
        capabilities: ClipboardGeneralCapabilityFlags,
    ) {
        self.bridge.file_available.store(
            self.bridge.file_enabled
                && capabilities.contains(ClipboardGeneralCapabilityFlags::STREAM_FILECLIP_ENABLED),
            Ordering::SeqCst,
        );
    }

    fn on_remote_copy(&mut self, available_formats: &[ClipboardFormat]) {
        let _operation = self
            .bridge
            .operations
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let mut state = self
            .bridge
            .managed
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        self.bridge.cancel_remote("Remote clipboard changed");
        if state.access != ClipboardAccess::Active {
            self.bridge.queue_remote_fetch(None);
            return;
        }
        state.request = state.request.wrapping_add(1);
        self.bridge.emit(ClipboardEvent::Started {
            epoch: state.epoch,
            request: state.request,
        });
        let file_format = self
            .bridge
            .file_available
            .load(Ordering::SeqCst)
            .then(|| {
                available_formats.iter().find(|format| {
                    format
                        .name
                        .as_ref()
                        .is_some_and(|name| name.value() == FORMAT_NAME_FILE_LIST)
                })
            })
            .flatten()
            .map(|format| (FetchKind::FileList, format.id()));
        let desired = file_format.or_else(|| {
            available_formats
                .iter()
                .find(|format| format.id() == ClipboardFormatId::CF_UNICODETEXT)
                .map(|format| (FetchKind::Text, format.id()))
        });
        self.bridge.queue_remote_fetch(desired);
    }

    fn on_format_data_request(&mut self, request: FormatDataRequest) {
        let active = self
            .bridge
            .managed
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .access
            == ClipboardAccess::Active;
        let response = if active && request.format == ClipboardFormatId::CF_UNICODETEXT {
            OwnedFormatDataResponse::new_unicode_string(&self.bridge.local_text())
        } else {
            OwnedFormatDataResponse::new_error()
        };
        self.bridge
            .send_clipboard_message(ClipboardMessage::SendFormatData(response));
    }

    fn on_format_data_response(&mut self, response: FormatDataResponse<'_>) {
        let _operation = self
            .bridge
            .operations
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let reply = self
            .bridge
            .finish_remote_fetch(FetchKind::Text, response.is_error());
        if !reply.accepted {
            return;
        }
        if let Ok(text) = response.to_unicode_string() {
            self.bridge.publish_remote_text(text);
        }
    }

    fn on_file_contents_request(&mut self, request: FileContentsRequest) {
        let state = self
            .bridge
            .managed
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        if state.access != ClipboardAccess::Active {
            self.bridge
                .send_clipboard_message(ClipboardMessage::SendFileContentsResponse(
                    FileContentsResponse::new_error(request.stream_id),
                ));
            return;
        }
        let response = self
            .bridge
            .files
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .serve(&request)
            .unwrap_or_else(|_| FileContentsResponse::new_error(request.stream_id));
        self.bridge
            .send_clipboard_message(ClipboardMessage::SendFileContentsResponse(response));
    }

    fn on_file_contents_response(&mut self, response: FileContentsResponse<'_>) {
        let _operation = self
            .bridge
            .operations
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let failure_progress = self
            .bridge
            .transfer_progress(RdpClipboardTransferStatus::Failed, None);
        let result = self
            .bridge
            .files
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .handle_response(&response);
        match result {
            Ok(update) => {
                self.bridge
                    .publish_transfer(RdpClipboardTransferStatus::Running, None);
                for request in update.requests {
                    self.bridge
                        .send_clipboard_message(ClipboardMessage::SendFileContentsRequest(request));
                }
                if update.completed {
                    let completed = self
                        .bridge
                        .transfer_progress(RdpClipboardTransferStatus::Completed, None);
                    match self.bridge.complete_remote() {
                        Ok(()) => {} // Application reports completion after publication.
                        Err(error) => {
                            if let Some(mut progress) = completed {
                                progress.status = RdpClipboardTransferStatus::Failed;
                                progress.error = Some(error);
                                self.bridge.send_transfer_progress(progress);
                            }
                        }
                    }
                }
            }
            Err(error) => {
                if let Some(mut progress) = failure_progress {
                    progress.error = Some(error);
                    self.bridge.send_transfer_progress(progress);
                }
            }
        }
    }

    fn on_lock(&mut self, data_id: LockDataId) {
        self.bridge
            .files
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .lock(data_id);
    }

    fn on_unlock(&mut self, data_id: LockDataId) {
        self.bridge
            .files
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .unlock(data_id);
    }

    fn on_remote_file_list(
        &mut self,
        files: &[ironrdp_cliprdr::pdu::FileDescriptor],
        clip_data_id: Option<u32>,
    ) {
        let _operation = self
            .bridge
            .operations
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let reply = self.bridge.finish_remote_fetch(FetchKind::FileList, false);
        if !reply.accepted {
            return;
        }
        if self.bridge.shutdown.load(Ordering::SeqCst)
            || self
                .bridge
                .managed
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .access
                == ClipboardAccess::Disabled
            || !self.bridge.file_available.load(Ordering::SeqCst)
        {
            return;
        }
        let result = self
            .bridge
            .files
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .begin_remote(files, clip_data_id);
        match result {
            Ok(update) => {
                self.bridge
                    .publish_transfer(RdpClipboardTransferStatus::Running, None);
                for request in update.requests {
                    self.bridge
                        .send_clipboard_message(ClipboardMessage::SendFileContentsRequest(request));
                }
                if update.completed {
                    let completed = self
                        .bridge
                        .transfer_progress(RdpClipboardTransferStatus::Completed, None);
                    match self.bridge.complete_remote() {
                        Ok(()) => {} // Application reports completion after publication.
                        Err(error) => {
                            if let Some(mut progress) = completed {
                                progress.status = RdpClipboardTransferStatus::Failed;
                                progress.error = Some(error);
                                self.bridge.send_transfer_progress(progress);
                            }
                        }
                    }
                }
            }
            Err(error) => {
                let _ = self
                    .bridge
                    .output_tx
                    .send(Outbound::control(RdpControlMessage::Error {
                        session_id: self.bridge.session_id.clone(),
                        error: nyaterm_remote_desktop::RdpError::new(
                            nyaterm_remote_desktop::RdpErrorKind::Clipboard,
                            error,
                        ),
                        fatal: false,
                    }));
            }
        }
    }

    fn on_outgoing_locks_expired(&mut self, ids: &[LockDataId]) {
        let progress = self.bridge.transfer_progress(
            RdpClipboardTransferStatus::Cancelled,
            Some("RDP clipboard lock expired".to_string()),
        );
        let cancelled = self
            .bridge
            .files
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .cancel_lock(ids);
        if cancelled.is_some()
            && let Some(progress) = progress
        {
            self.bridge.send_transfer_progress(progress);
        }
    }

    fn on_outgoing_locks_cleared(&mut self, ids: &[LockDataId]) {
        self.on_outgoing_locks_expired(ids);
    }
}

#[cfg(test)]
mod protocol_tests;

#[cfg(test)]
mod tests {
    use std::sync::mpsc;

    use ironrdp_cliprdr::backend::{ClipboardMessage, CliprdrBackend};
    use ironrdp_cliprdr::pdu::{ClipboardFormatId, ClipboardGeneralCapabilityFlags};

    use super::{ClipboardBridge, TextClipboardBackend};
    use crate::clipboard::MAX_CLIPBOARD_TEXT_BYTES;

    #[test]
    fn helper_factory_enables_file_capability_for_every_legacy_clipboard_mode() {
        use crate::{CertificateGate, build_config};
        use nyaterm_remote_desktop::RdpClipboardMode;
        use std::sync::Arc;

        for mode in [
            RdpClipboardMode::Disabled,
            RdpClipboardMode::TextOnly,
            RdpClipboardMode::TextAndFiles,
        ] {
            let mut source = crate::tests::helper_config();
            source.clipboard.mode = mode;
            let (output_tx, _output_rx) = mpsc::sync_channel(1);
            let (_config, bridge, _) = build_config(
                "fixture",
                &source,
                output_tx,
                Arc::new(CertificateGate::default()),
            )
            .unwrap();
            let bridge = bridge.expect("default redirection keeps the clipboard on");
            let backend = TextClipboardBackend { bridge };
            assert!(
                backend
                    .client_capabilities()
                    .contains(ClipboardGeneralCapabilityFlags::STREAM_FILECLIP_ENABLED)
            );
        }
    }

    #[test]
    fn headless_clipboard_accepts_text_and_rejects_oversize_payloads() {
        let (output_tx, _output_rx) = mpsc::sync_channel(1);
        let bridge = ClipboardBridge::new("session".to_string(), output_tx, false);
        bridge.set_local_text("hello".to_string()).unwrap();
        assert_eq!(bridge.local_text(), "hello");
        assert!(
            bridge
                .set_local_text("x".repeat(MAX_CLIPBOARD_TEXT_BYTES + 1))
                .is_err()
        );
        assert_eq!(bridge.local_text(), "hello");
    }

    #[test]
    fn initial_clipboard_advertisement_includes_an_empty_format_list() {
        let (output_tx, _output_rx) = mpsc::sync_channel(1);
        let bridge = ClipboardBridge::new("session".to_string(), output_tx, false);
        assert!(matches!(
            bridge.current_advertisement(),
            ClipboardMessage::SendInitiateCopy(formats) if formats.is_empty()
        ));

        bridge.set_local_text("hello".to_string()).unwrap();
        assert!(matches!(
            bridge.current_advertisement(),
            ClipboardMessage::SendInitiateCopy(formats)
                if formats.len() == 1 && formats[0].id() == ClipboardFormatId::CF_UNICODETEXT
        ));
    }

    #[test]
    fn initial_advertisement_uses_text_format_even_with_file_capability() {
        let (output_tx, _output_rx) = mpsc::sync_channel(1);
        let bridge = ClipboardBridge::new("session".to_string(), output_tx, true);
        bridge.set_local_text("hello".to_string()).unwrap();
        bridge
            .file_available
            .store(true, std::sync::atomic::Ordering::SeqCst);

        assert!(matches!(
            bridge.initial_advertisement(),
            ClipboardMessage::SendInitiateCopy(formats)
                if formats.len() == 1 && formats[0].id() == ClipboardFormatId::CF_UNICODETEXT
        ));
    }

    #[test]
    fn file_capability_is_opt_in_and_requires_server_negotiation() {
        let (output_tx, _output_rx) = mpsc::sync_channel(1);
        let bridge = ClipboardBridge::new("session".to_string(), output_tx, true);
        let mut backend = TextClipboardBackend {
            bridge: bridge.clone(),
        };
        assert!(
            backend
                .client_capabilities()
                .contains(ClipboardGeneralCapabilityFlags::STREAM_FILECLIP_ENABLED)
        );
        backend.on_process_negotiated_capabilities(ClipboardGeneralCapabilityFlags::empty());
        assert!(
            !bridge
                .file_available
                .load(std::sync::atomic::Ordering::SeqCst)
        );
        backend.on_process_negotiated_capabilities(
            ClipboardGeneralCapabilityFlags::STREAM_FILECLIP_ENABLED,
        );
        assert!(
            bridge
                .file_available
                .load(std::sync::atomic::Ordering::SeqCst)
        );

        let (output_tx, _output_rx) = mpsc::sync_channel(1);
        let disabled = ClipboardBridge::new("text-only".to_string(), output_tx, false);
        let backend = TextClipboardBackend { bridge: disabled };
        assert!(
            !backend
                .client_capabilities()
                .contains(ClipboardGeneralCapabilityFlags::STREAM_FILECLIP_ENABLED)
        );
    }
}

#[cfg(test)]
mod managed_tests;
