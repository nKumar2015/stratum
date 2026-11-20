//! Pictures of windows for the overview and the window switcher, taken
//! through Hyprland's `hyprland-toplevel-export-v1`.
//!
//! Capturing runs on a thread of its own, with an event queue of its own on
//! the shell's Wayland connection, so a copy (10–30 ms in the compositor)
//! never holds up a frame. The thread starts the first time pictures are
//! wanted and then waits on its channel; it holds memory only while
//! watching: one shared-memory buffer the size of the largest window, which
//! it gives back on `stop`.
//!
//! Windows are taken one after another into that buffer and scaled down
//! there to `long_edge` pixels; only the small picture leaves the thread.
//! A window on a hidden workspace can be taken too: Hyprland draws it for
//! the copy.

use std::collections::HashMap;
use std::os::fd::AsRawFd;
use std::sync::mpsc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use smithay_client_toolkit::delegate_dispatch2;
use smithay_client_toolkit::shm::raw::RawPool;
use smithay_client_toolkit::shm::{Shm, ShmHandler};
use wayland_client::globals::GlobalList;
use wayland_client::protocol::{wl_buffer, wl_shm};
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle, WEnum};

#[allow(dead_code, non_upper_case_globals, non_camel_case_types, unused_imports, clippy::all)]
pub mod protocol {
    use wayland_client;
    use wayland_client::protocol::*;

    pub mod __interfaces {
        use wayland_client::backend as wayland_backend;
        use wayland_client::protocol::__interfaces::*;
        wayland_scanner::generate_interfaces!("protocols/hyprland-toplevel-export-v1.xml");
    }
    use self::__interfaces::*;

    wayland_scanner::generate_client_code!("protocols/hyprland-toplevel-export-v1.xml");
}

use protocol::hyprland_toplevel_export_frame_v1::{self as frame, HyprlandToplevelExportFrameV1 as Frame};
use protocol::hyprland_toplevel_export_manager_v1::HyprlandToplevelExportManagerV1 as Manager;

/// How long a copy may take before the window is skipped for this round.
const COPY_TIMEOUT: Duration = Duration::from_secs(1);

/// A window's picture, premultiplied RGBA8.
#[derive(Clone, Debug)]
pub struct Thumb {
    /// New for every picture, for the renderer's texture cache.
    pub key: u64,
    pub width: u32,
    pub height: u32,
    pub pixels: Arc<[u8]>,
}

/// What to keep pictures of.
#[derive(Clone, Debug, PartialEq)]
pub struct Watch {
    /// Window addresses, the first taken first.
    pub windows: Vec<u64>,
    /// The longer side of a picture, in pixels.
    pub long_edge: u32,
    /// How often each window is taken again; `None` takes each once.
    pub refresh: Option<Duration>,
}

/// A window's new picture, delivered to the event loop.
pub struct Captured {
    pub address: u64,
    pub thumb: Thumb,
}

enum Command {
    Watch(Watch),
    Stop,
}

/// The shell's side: sends the thread what to watch.
pub struct WindowCapture {
    tx: Option<mpsc::Sender<Command>>,
    /// What the thread is given when it starts, the first time pictures are
    /// wanted.
    parts: Option<Parts>,
    events: calloop::channel::Sender<Captured>,
    /// The last watch sent, so an unchanged one is not sent again.
    watching: Option<Watch>,
}

struct Parts {
    conn: Connection,
    queue: EventQueue<Worker>,
    manager: Manager,
    shm: Shm,
}

impl WindowCapture {
    /// Bind the export manager on a queue of its own. `None` when the
    /// compositor does not offer it (not Hyprland): windows are then shown
    /// by their icons alone.
    pub fn bind(conn: &Connection, globals: &GlobalList, events: calloop::channel::Sender<Captured>) -> Option<Self> {
        let queue = conn.new_event_queue::<Worker>();
        let qh = queue.handle();
        let manager: Manager = globals.bind(&qh, 1..=1, FrameData).ok()?;
        let shm = Shm::bind(globals, &qh).ok()?;
        Some(Self {
            tx: None,
            parts: Some(Parts { conn: conn.clone(), queue, manager, shm }),
            events,
            watching: None,
        })
    }

    /// Keep pictures of `watch`'s windows until [`Self::stop`]. A repeated
    /// watch changes nothing; a new one keeps the pictures already taken of
    /// windows it still names.
    pub fn watch(&mut self, watch: Watch) {
        if self.watching.as_ref() == Some(&watch) {
            return;
        }
        self.watching = Some(watch.clone());
        self.send(Command::Watch(watch));
    }

    /// Stop taking pictures and free the buffer.
    pub fn stop(&mut self) {
        if self.watching.take().is_some() {
            self.send(Command::Stop);
        }
    }

    fn send(&mut self, command: Command) {
        if self.tx.is_none() {
            let Some(parts) = self.parts.take() else {
                return;
            };
            let (tx, rx) = mpsc::channel();
            let events = self.events.clone();
            let spawned = std::thread::Builder::new()
                .name("stratum-capture".into())
                .spawn(move || run(parts, rx, events));
            if let Err(err) = spawned {
                tracing::warn!("window pictures unavailable: {err}");
                return;
            }
            self.tx = Some(tx);
        }
        if let Some(tx) = &self.tx {
            let _ = tx.send(command);
        }
    }
}

/// The thread: waits for a watch, then takes its windows in turn.
fn run(parts: Parts, rx: mpsc::Receiver<Command>, events: calloop::channel::Sender<Captured>) {
    let Parts { conn, mut queue, manager, shm } = parts;
    let qh = queue.handle();
    let mut worker = Worker { shm, frame: FrameState::default() };
    let mut pool: Option<RawPool> = None;
    let mut watch: Option<Watch> = None;
    // When each window was last taken.
    let mut taken: HashMap<u64, Instant> = HashMap::new();
    let mut next_key: u64 = 0x5743_0000_0000_0000;

    loop {
        // Wait for a command: for good while idle or when no window will be
        // due again, else until the next one is.
        let wait = watch.as_ref().and_then(|w| next_due(w, &taken, Instant::now()));
        let command = match wait {
            None => match rx.recv() {
                Ok(command) => Some(command),
                Err(_) => return,
            },
            Some(wait) => match rx.recv_timeout(wait) {
                Ok(command) => Some(command),
                Err(mpsc::RecvTimeoutError::Timeout) => None,
                Err(mpsc::RecvTimeoutError::Disconnected) => return,
            },
        };
        match command {
            Some(Command::Watch(next)) => {
                taken.retain(|address, _| next.windows.contains(address));
                watch = Some(next);
                continue;
            }
            Some(Command::Stop) => {
                watch = None;
                taken.clear();
                // The buffer is as large as the largest window: give it back.
                pool = None;
                continue;
            }
            None => {}
        }
        let Some(current) = &watch else { continue };
        let now = Instant::now();
        let Some(address) = due(current, &taken, now) else { continue };
        taken.insert(address, now);
        let long_edge = current.long_edge;
        match capture(&conn, &mut queue, &qh, &manager, &mut worker, &mut pool, address, long_edge) {
            Ok(Some((width, height, pixels))) => {
                next_key += 1;
                let thumb = Thumb { key: next_key, width, height, pixels: Arc::from(pixels) };
                if events.send(Captured { address, thumb }).is_err() {
                    return;
                }
            }
            Ok(None) => {}
            Err(err) => {
                tracing::warn!("window pictures stopped: {err}");
                return;
            }
        }
    }
}

/// The window to take now: the first never taken, else the one taken
/// longest ago if its refresh is due.
fn due(watch: &Watch, taken: &HashMap<u64, Instant>, now: Instant) -> Option<u64> {
    if let Some(first) = watch.windows.iter().find(|a| !taken.contains_key(a)) {
        return Some(*first);
    }
    let refresh = watch.refresh?;
    watch
        .windows
        .iter()
        .filter_map(|a| taken.get(a).map(|t| (*a, *t)))
        .filter(|(_, t)| now.duration_since(*t) >= refresh)
        .min_by_key(|(_, t)| *t)
        .map(|(a, _)| a)
}

/// How long until a window is due, or `None` if none ever will be.
fn next_due(watch: &Watch, taken: &HashMap<u64, Instant>, now: Instant) -> Option<Duration> {
    if watch.windows.iter().any(|a| !taken.contains_key(a)) {
        return Some(Duration::ZERO);
    }
    let refresh = watch.refresh?;
    watch
        .windows
        .iter()
        .filter_map(|a| taken.get(a))
        .map(|t| refresh.saturating_sub(now.duration_since(*t)))
        .min()
}

/// Take one window and scale it down. `Ok(None)` when this window could
/// not be taken (closed, or the copy failed); `Err` when the connection is
/// gone.
#[allow(clippy::too_many_arguments)]
fn capture(
    conn: &Connection,
    queue: &mut EventQueue<Worker>,
    qh: &QueueHandle<Worker>,
    manager: &Manager,
    worker: &mut Worker,
    pool: &mut Option<RawPool>,
    address: u64,
    long_edge: u32,
) -> Result<Option<(u32, u32, Vec<u8>)>, String> {
    worker.frame = FrameState::default();
    // The handle is the address's low 32 bits, as the protocol says.
    let frame = manager.capture_toplevel(0, address as u32, qh, FrameData);
    let deadline = Instant::now() + COPY_TIMEOUT;
    if !wait(conn, queue, worker, deadline, |f| f.buffer_done || f.failed)? || worker.frame.failed {
        frame.destroy();
        return Ok(None);
    }
    let Some((format, width, height, stride)) = worker.frame.buffer else {
        frame.destroy();
        return Ok(None);
    };
    if width == 0 || height == 0 || stride < width * 4 || !readable(format) {
        frame.destroy();
        return Ok(None);
    }
    let len = stride as usize * height as usize;
    let pool = match pool {
        Some(pool) => {
            pool.resize(len).map_err(|err| err.to_string())?;
            pool
        }
        None => pool.insert(RawPool::new(len, &worker.shm).map_err(|err| err.to_string())?),
    };
    let buffer = pool.create_buffer(0, width as i32, height as i32, stride as i32, format, FrameData, qh);
    frame.copy(&buffer, 1);
    let copied = wait(conn, queue, worker, deadline, |f| f.ready || f.failed)? && worker.frame.ready;
    frame.destroy();
    buffer.destroy();
    if !copied {
        return Ok(None);
    }
    let bytes = &pool.mmap()[..len];
    let picture = shrink(bytes, format, width, height, stride, long_edge, worker.frame.y_invert);
    Ok(Some(picture))
}

/// Dispatch the worker's queue until `done` or `deadline`. `Ok(false)` on
/// the deadline.
fn wait(
    conn: &Connection,
    queue: &mut EventQueue<Worker>,
    worker: &mut Worker,
    deadline: Instant,
    done: impl Fn(&FrameState) -> bool,
) -> Result<bool, String> {
    loop {
        queue.dispatch_pending(worker).map_err(|err| err.to_string())?;
        if done(&worker.frame) {
            return Ok(true);
        }
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Ok(false);
        }
        conn.flush().map_err(|err| err.to_string())?;
        // Another thread (the event loop) may read the socket meanwhile; a
        // guard that cannot be had means events are already queued here.
        let Some(guard) = queue.prepare_read() else { continue };
        let mut poll = libc::pollfd { fd: guard.connection_fd().as_raw_fd(), events: libc::POLLIN, revents: 0 };
        let ms = left.as_millis().min(i32::MAX as u128) as i32;
        // SAFETY: one valid pollfd for the duration of the call.
        let ready = unsafe { libc::poll(&mut poll, 1, ms.max(1)) };
        if ready > 0 {
            match guard.read() {
                Ok(_) => {}
                Err(wayland_client::backend::WaylandError::Io(err)) if err.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(err) => return Err(err.to_string()),
            }
        }
        // Dropping the guard otherwise cancels the read.
    }
}

/// The formats [`shrink`] reads.
fn readable(format: wl_shm::Format) -> bool {
    use wl_shm::Format::*;
    matches!(format, Argb8888 | Xrgb8888 | Abgr8888 | Xbgr8888 | Argb2101010 | Xrgb2101010 | Abgr2101010 | Xbgr2101010)
}

/// Scale a captured buffer down to fit `long_edge` and turn it into
/// premultiplied RGBA8 (the buffer's alpha is premultiplied already).
fn shrink(
    bytes: &[u8],
    format: wl_shm::Format,
    width: u32,
    height: u32,
    stride: u32,
    long_edge: u32,
    y_invert: bool,
) -> (u32, u32, Vec<u8>) {
    use wl_shm::Format::*;
    let fit = (long_edge.max(1) as f32 / width.max(height) as f32).min(1.0);
    let (tw, th) = (((width as f32 * fit).round() as u32).max(1), ((height as f32 * fit).round() as u32).max(1));
    let packed = stride == width * 4;
    let ten_bit = matches!(format, Argb2101010 | Xrgb2101010 | Abgr2101010 | Xbgr2101010);
    let bgr = matches!(format, Abgr2101010 | Xbgr2101010);
    // Eight bits a channel: averaged as they are, the channels swapped on
    // the small result. Ten: unpacked to eight first.
    let source: std::borrow::Cow<[u8]> = if packed && !ten_bit {
        std::borrow::Cow::Borrowed(&bytes[..(width * height * 4) as usize])
    } else {
        let mut rows = Vec::with_capacity((width * height * 4) as usize);
        for row in bytes.chunks(stride as usize).take(height as usize) {
            let row = &row[..(width * 4) as usize];
            if ten_bit {
                for px in row.chunks_exact(4) {
                    rows.extend_from_slice(&unpack_2101010(u32::from_le_bytes([px[0], px[1], px[2], px[3]]), bgr));
                }
            } else {
                rows.extend_from_slice(row);
            }
        }
        std::borrow::Cow::Owned(rows)
    };
    let view = image::ImageBuffer::<image::Rgba<u8>, &[u8]>::from_raw(width, height, &source[..])
        .expect("the buffer holds width × height pixels");
    let mut small = if (tw, th) == (width, height) {
        image::ImageBuffer::from_raw(width, height, source.to_vec()).expect("same size")
    } else {
        image::imageops::thumbnail(&view, tw, th)
    };
    if y_invert {
        image::imageops::flip_vertical_in_place(&mut small);
    }
    let mut pixels = small.into_raw();
    // Little-endian `xRGB` is B, G, R, x in memory; `xBGR` is R, G, B, x.
    // 10-bit formats were unpacked to R, G, B, A above.
    let swap = matches!(format, Argb8888 | Xrgb8888);
    let opaque = matches!(format, Xrgb8888 | Xbgr8888 | Xrgb2101010 | Xbgr2101010);
    for px in pixels.chunks_exact_mut(4) {
        if swap {
            px.swap(0, 2);
        }
        if opaque {
            px[3] = 255;
        }
    }
    (tw, th, pixels)
}

/// One 10-bit pixel as RGBA8. The high channel (bits 20–29) is red in
/// `[AX]RGB2101010` and blue in `[AX]BGR2101010`.
fn unpack_2101010(value: u32, bgr: bool) -> [u8; 4] {
    let channel = |shift: u32| (((value >> shift) & 0x3ff) >> 2) as u8;
    let a = (((value >> 30) & 0x3) * 85) as u8;
    let (high, mid, low) = (channel(20), channel(10), channel(0));
    if bgr {
        [low, mid, high, a]
    } else {
        [high, mid, low, a]
    }
}

/// The worker's state: the frame in progress.
struct Worker {
    shm: Shm,
    frame: FrameState,
}

#[derive(Default)]
struct FrameState {
    buffer: Option<(wl_shm::Format, u32, u32, u32)>,
    buffer_done: bool,
    y_invert: bool,
    ready: bool,
    failed: bool,
}

/// User data for the worker's own objects (manager, frames, buffers).
struct FrameData;

impl ShmHandler for Worker {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

delegate_dispatch2!(Worker);

impl Dispatch<Manager, FrameData> for Worker {
    fn event(_: &mut Self, _: &Manager, _: <Manager as Proxy>::Event, _: &FrameData, _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<wl_buffer::WlBuffer, FrameData> for Worker {
    fn event(_: &mut Self, _: &wl_buffer::WlBuffer, _: wl_buffer::Event, _: &FrameData, _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<Frame, FrameData> for Worker {
    fn event(worker: &mut Self, _: &Frame, event: frame::Event, _: &FrameData, _: &Connection, _: &QueueHandle<Self>) {
        let f = &mut worker.frame;
        match event {
            frame::Event::Buffer { format: WEnum::Value(format), width, height, stride } => {
                // The first readable format offered.
                if f.buffer.is_none() && readable(format) {
                    f.buffer = Some((format, width, height, stride));
                }
            }
            frame::Event::BufferDone => f.buffer_done = true,
            frame::Event::Flags { flags: WEnum::Value(flags) } => {
                f.y_invert = flags.contains(frame::Flags::YInvert);
            }
            frame::Event::Ready { .. } => f.ready = true,
            frame::Event::Failed => f.failed = true,
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn watch(windows: &[u64], refresh_ms: Option<u64>) -> Watch {
        Watch { windows: windows.to_vec(), long_edge: 64, refresh: refresh_ms.map(Duration::from_millis) }
    }

    #[test]
    fn windows_never_taken_come_first_in_order() {
        let now = Instant::now();
        let mut taken = HashMap::new();
        let w = watch(&[3, 1, 2], Some(1000));
        assert_eq!(due(&w, &taken, now), Some(3));
        taken.insert(3, now);
        assert_eq!(due(&w, &taken, now), Some(1));
        assert_eq!(next_due(&w, &taken, now), Some(Duration::ZERO));
    }

    #[test]
    fn the_oldest_picture_is_refreshed_once_due() {
        let start = Instant::now();
        let taken: HashMap<u64, Instant> =
            [(1, start), (2, start + Duration::from_millis(300))].into_iter().collect();
        let w = watch(&[1, 2], Some(1000));
        assert_eq!(due(&w, &taken, start + Duration::from_millis(500)), None);
        assert_eq!(next_due(&w, &taken, start + Duration::from_millis(500)), Some(Duration::from_millis(500)));
        assert_eq!(due(&w, &taken, start + Duration::from_millis(1100)), Some(1));
    }

    #[test]
    fn without_a_refresh_each_window_is_taken_once() {
        let now = Instant::now();
        let taken: HashMap<u64, Instant> = [(1, now)].into_iter().collect();
        let w = watch(&[1], None);
        assert_eq!(due(&w, &taken, now + Duration::from_secs(60)), None);
        assert_eq!(next_due(&w, &taken, now), None);
    }

    #[test]
    fn shrinking_keeps_the_shape_and_swaps_to_rgba() {
        // 4×2 BGRA, every pixel blue (B = 255) and opaque.
        let bytes: Vec<u8> = (0..8).flat_map(|_| [255u8, 0, 0, 255]).collect();
        let (w, h, px) = shrink(&bytes, wl_shm::Format::Argb8888, 4, 2, 16, 2, false);
        assert_eq!((w, h), (2, 1));
        assert_eq!(&px[..4], &[0, 0, 255, 255], "blue ends up in the third channel");
    }

    #[test]
    fn padded_rows_and_x_formats_are_read() {
        // 2×2 XRGB with a stride of 12 (4 bytes of padding a row) and a
        // garbage x byte, which must come out opaque.
        let row = [0u8, 0, 200, 7, 0, 0, 200, 7, 9, 9, 9, 9];
        let bytes: Vec<u8> = row.iter().chain(row.iter()).copied().collect();
        let (w, h, px) = shrink(&bytes, wl_shm::Format::Xrgb8888, 2, 2, 12, 64, false);
        assert_eq!((w, h), (2, 2));
        assert!(px.chunks_exact(4).all(|p| p == [200, 0, 0, 255]));
    }

    #[test]
    fn ten_bit_pixels_unpack_to_eight() {
        let high = (0x3ffu32 << 20) | (0x3 << 30);
        assert_eq!(unpack_2101010(high, false), [255, 0, 0, 255]);
        assert_eq!(unpack_2101010(high, true), [0, 0, 255, 255]);
    }
}
