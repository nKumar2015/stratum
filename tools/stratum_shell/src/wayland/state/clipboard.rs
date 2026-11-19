//! Watching the clipboard for the history (`crate::clipboard`).
//!
//! `ext-data-control-v1` reports every new selection with the types it is
//! offered in. The shell picks one (`clipboard::choose`, which also skips a
//! password manager's copies), asks the owner to write it into a pipe, and
//! hands the read end to one reader thread, which records copies in the
//! order they were made. A read gives up after a few seconds without data
//! or past `[clipboard] max_entry_mb`, so a misbehaving owner costs a
//! thread's wait, never the event loop.

use std::io::Read;
use std::os::fd::AsFd;
use std::sync::{mpsc, Arc, Mutex};

use smithay_client_toolkit::dispatch2::Dispatch2;
use wayland_client::backend::ObjectData;
use wayland_client::globals::GlobalList;
use wayland_client::Proxy;
use wayland_protocols::ext::data_control::v1::client::ext_data_control_device_v1::{self, ExtDataControlDeviceV1};
use wayland_protocols::ext::data_control::v1::client::ext_data_control_manager_v1::ExtDataControlManagerV1;
use wayland_protocols::ext::data_control::v1::client::ext_data_control_offer_v1::{self, ExtDataControlOfferV1};

use super::launcher::LauncherEvent;
use super::*;
use crate::clipboard::store::Store;

/// How long a copy's owner may go without writing before the read gives up.
const READ_TIMEOUT_MS: i32 = 5000;

/// One copy for the reader thread.
struct Job {
    mime: String,
    pipe: std::io::PipeReader,
}

/// The watch: the global, the device once there is a seat, the store, and
/// the reader thread's queue.
pub(super) struct ClipboardWatch {
    manager: ExtDataControlManagerV1,
    device: Option<ExtDataControlDeviceV1>,
    pub(super) store: Arc<Mutex<Store>>,
    jobs: mpsc::Sender<Job>,
}

impl App {
    /// Start keeping the history, if `[clipboard] enabled` and the
    /// compositor offers data control. Read at start.
    pub(super) fn start_clipboard(&mut self, globals: &GlobalList) {
        let config = &self.config.clipboard;
        if !config.enabled {
            return;
        }
        let Ok(manager) = globals.bind::<ExtDataControlManagerV1, _, _>(&self.queue_handle, 1..=1, ManagerData) else {
            warn!("the compositor has no ext-data-control-v1: no clipboard history");
            return;
        };
        let Some(dir) = Store::default_dir() else {
            warn!("no state directory: no clipboard history");
            return;
        };
        let store = Arc::new(Mutex::new(Store::open(&dir, config.max_entries)));
        let (jobs, queue) = mpsc::channel::<Job>();
        let reader_store = Arc::clone(&store);
        let events = self.launcher_events.clone();
        let max_bytes = config.max_entry_mb.saturating_mul(1 << 20);
        let spawned = std::thread::Builder::new().name("stratum-clipboard".into()).spawn(move || {
            for job in queue {
                let Some(data) = read_pipe(job.pipe, max_bytes) else {
                    continue;
                };
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |d| d.as_secs());
                let added = reader_store.lock().map(|mut store| store.add(&job.mime, &data, now));
                match added {
                    Ok(Ok(_)) => {
                        if let Some(events) = &events {
                            let _ = events.send(LauncherEvent::ClipsChanged);
                        }
                    }
                    Ok(Err(err)) => warn!("could not record a copy: {err}"),
                    Err(_) => break,
                }
            }
        });
        if let Err(err) = spawned {
            warn!("could not start the clipboard reader: {err}");
            return;
        }
        info!("keeping the clipboard history in {}", dir.display());
        self.clipboard = Some(ClipboardWatch { manager, device: None, store, jobs });
        self.watch_clipboard_seat();
    }

    /// Get the data device once there is a seat; called again when one
    /// appears.
    pub(super) fn watch_clipboard_seat(&mut self) {
        let Some(watch) = &mut self.clipboard else {
            return;
        };
        if watch.device.is_some() {
            return;
        }
        if let Some(seat) = self.seat_state.seats().next() {
            watch.device = Some(watch.manager.get_data_device(&seat, &self.queue_handle, DeviceData));
        }
    }

    /// A new selection: queue the chosen type for reading.
    fn clipboard_selection(&mut self, offer: ExtDataControlOfferV1, conn: &Connection) {
        let mimes = offer
            .data::<OfferData>()
            .and_then(|d| d.mimes.lock().ok().map(|m| m.clone()))
            .unwrap_or_default();
        let chosen = crate::clipboard::choose(&mimes).map(str::to_string);
        let (Some(mime), Some(watch)) = (chosen, &self.clipboard) else {
            offer.destroy();
            return;
        };
        match std::io::pipe() {
            Ok((reader, writer)) => {
                offer.receive(mime.clone(), writer.as_fd());
                // libwayland has its own copy of the descriptor: closing
                // ours lets the read end see the owner's close as the end.
                drop(writer);
                if let Err(err) = conn.flush() {
                    warn!("could not ask for the copy: {err}");
                }
                let _ = watch.jobs.send(Job { mime, pipe: reader });
            }
            Err(err) => warn!("no pipe for the copy: {err}"),
        }
        offer.destroy();
    }

    /// The history, for the launcher's clipboard mode.
    pub(super) fn clipboard_store(&self) -> Option<Arc<Mutex<Store>>> {
        self.clipboard.as_ref().map(|w| Arc::clone(&w.store))
    }

    /// `[clipboard] max_entries` changed in a reload.
    pub(super) fn clipboard_reconfigured(&mut self) {
        if let Some(watch) = &self.clipboard {
            if let Ok(mut store) = watch.store.lock() {
                store.set_max_entries(self.config.clipboard.max_entries);
            }
        }
    }
}

/// Read a copy to its end: `None` if it is empty, larger than `max`, or its
/// owner stops writing for [`READ_TIMEOUT_MS`].
fn read_pipe(mut pipe: std::io::PipeReader, max: usize) -> Option<Vec<u8>> {
    use std::os::fd::AsRawFd;
    let mut data = Vec::new();
    let mut chunk = [0u8; 64 * 1024];
    loop {
        let mut poll = libc::pollfd { fd: pipe.as_raw_fd(), events: libc::POLLIN, revents: 0 };
        // SAFETY: one valid pollfd for the duration of the call.
        let ready = unsafe { libc::poll(&mut poll, 1, READ_TIMEOUT_MS) };
        if ready <= 0 {
            warn!("a copy's owner stopped writing; not recorded");
            return None;
        }
        match pipe.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                if data.len() + n > max {
                    info!("a copy over {} MiB was not recorded", max >> 20);
                    return None;
                }
                data.extend_from_slice(&chunk[..n]);
            }
            Err(err) if err.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => return None,
        }
    }
    (!data.is_empty()).then_some(data)
}

pub(super) struct ManagerData;

impl Dispatch2<ExtDataControlManagerV1, App> for ManagerData {
    fn event(
        &self,
        _: &mut App,
        _: &ExtDataControlManagerV1,
        _: <ExtDataControlManagerV1 as wayland_client::Proxy>::Event,
        _: &Connection,
        _: &QueueHandle<App>,
    ) {
    }
}

pub(super) struct DeviceData;

impl Dispatch2<ExtDataControlDeviceV1, App> for DeviceData {
    fn event(
        &self,
        app: &mut App,
        _: &ExtDataControlDeviceV1,
        event: ext_data_control_device_v1::Event,
        conn: &Connection,
        _: &QueueHandle<App>,
    ) {
        match event {
            ext_data_control_device_v1::Event::Selection { id: Some(offer) } => app.clipboard_selection(offer, conn),
            // The primary selection (middle-click paste) is not kept.
            ext_data_control_device_v1::Event::PrimarySelection { id: Some(offer) } => offer.destroy(),
            ext_data_control_device_v1::Event::Finished => {
                warn!("the compositor ended clipboard watching");
                if let Some(watch) = &mut app.clipboard {
                    if let Some(device) = watch.device.take() {
                        device.destroy();
                    }
                }
            }
            _ => {}
        }
    }

    fn event_created_child(opcode: u16, qh: &QueueHandle<App>) -> Arc<dyn ObjectData> {
        match opcode {
            // data_offer: the offer's types arrive before the selection
            // that names it.
            0 => qh.make_data::<ExtDataControlOfferV1, _>(OfferData::default()),
            _ => panic!("ext_data_control_device_v1 has no child-creating event {opcode}"),
        }
    }
}

#[derive(Default)]
pub(super) struct OfferData {
    mimes: Mutex<Vec<String>>,
}

impl Dispatch2<ExtDataControlOfferV1, App> for OfferData {
    fn event(
        &self,
        _: &mut App,
        _: &ExtDataControlOfferV1,
        event: ext_data_control_offer_v1::Event,
        _: &Connection,
        _: &QueueHandle<App>,
    ) {
        if let ext_data_control_offer_v1::Event::Offer { mime_type } = event {
            if let Ok(mut mimes) = self.mimes.lock() {
                mimes.push(mime_type);
            }
        }
    }
}
