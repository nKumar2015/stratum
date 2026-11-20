//! The desktop portal's FileChooser, so every app's Open and Save dialogs
//! are ours: `org.freedesktop.impl.portal.FileChooser` on the session bus,
//! as `org.freedesktop.impl.portal.desktop.stratum`. xdg-desktop-portal
//! calls it when `portals.conf` names `stratum` for the FileChooser.
//!
//! Each request starts `stratum-files --choose` (its own process and window)
//! and waits for its answer without blocking the bus. While it runs, a
//! `Request` object at the call's handle lets the portal close it.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use zbus::object_server::ObjectServer;
use zbus::zvariant::{ObjectPath, OwnedValue, Value};

use crate::chooser::{Answer, Request};

pub const BUS_NAME: &str = "org.freedesktop.impl.portal.desktop.stratum";
const PATH: &str = "/org/freedesktop/portal/desktop";

/// A filter as the portal sends it: a name and `(0 glob | 1 MIME, pattern)`s.
type Filter = (String, Vec<(u32, String)>);

/// The running choosers, by request handle, so Close can stop one.
#[derive(Clone, Default)]
struct Running(Arc<Mutex<HashMap<String, u32>>>);

struct FileChooser {
    running: Running,
}

/// The `org.freedesktop.impl.portal.Request` at a call's handle.
struct RequestObject {
    running: Running,
    handle: String,
}

#[zbus::interface(name = "org.freedesktop.impl.portal.Request")]
impl RequestObject {
    fn close(&self) {
        if let Some(pid) = self.running.0.lock().ok().and_then(|r| r.get(&self.handle).copied()) {
            // SAFETY: a signal to the chooser this service started.
            unsafe { libc::kill(pid as i32, libc::SIGTERM) };
        }
    }
}

fn text(options: &HashMap<String, OwnedValue>, key: &str) -> Option<String> {
    options.get(key).and_then(|v| String::try_from(v.try_clone().ok()?).ok())
}

fn flag(options: &HashMap<String, OwnedValue>, key: &str) -> bool {
    options.get(key).and_then(|v| bool::try_from(v).ok()).unwrap_or(false)
}

/// A path sent as a NUL-terminated byte string (`ay`).
fn bytes_path(options: &HashMap<String, OwnedValue>, key: &str) -> Option<PathBuf> {
    let value = options.get(key)?.try_clone().ok()?;
    let bytes: Vec<u8> = Vec::try_from(value).ok()?;
    let bytes: Vec<u8> = bytes.into_iter().take_while(|b| *b != 0).collect();
    (!bytes.is_empty()).then(|| PathBuf::from(<std::ffi::OsString as std::os::unix::ffi::OsStringExt>::from_vec(bytes)))
}

fn filters(options: &HashMap<String, OwnedValue>) -> Vec<Filter> {
    options.get("filters").and_then(|v| Vec::<Filter>::try_from(v.try_clone().ok()?).ok()).unwrap_or_default()
}

fn current_filter(options: &HashMap<String, OwnedValue>) -> Option<Filter> {
    options.get("current_filter").and_then(|v| Filter::try_from(v.try_clone().ok()?).ok())
}

/// The chooser's request for a portal call's options.
pub fn request(title: &str, save: bool, options: &HashMap<String, OwnedValue>) -> Request {
    let filters = filters(options);
    let current = current_filter(options);
    // The current filter is one of the list, or added to it.
    let (filters, current_filter) = match current {
        Some(f) => match filters.iter().position(|g| g.0 == f.0) {
            Some(i) => (filters, Some(i)),
            None => {
                let mut all = vec![f];
                all.extend(filters);
                (all, Some(0))
            }
        },
        None => (filters, None),
    };
    // Saving over a file names it and its folder.
    let current_file = if save { bytes_path(options, "current_file") } else { None };
    Request {
        save,
        title: title.to_string(),
        multiple: flag(options, "multiple"),
        directory: flag(options, "directory"),
        accept_label: text(options, "accept_label"),
        current_folder: bytes_path(options, "current_folder").or_else(|| current_file.as_ref().and_then(|f| f.parent().map(PathBuf::from))),
        current_name: text(options, "current_name").or_else(|| current_file.and_then(|f| f.file_name().map(|n| n.to_string_lossy().into_owned()))),
        filters,
        current_filter,
    }
}

/// The portal's results for a chooser's answer: response 0 with the URIs,
/// or 1 when cancelled.
pub fn results(answer: &Answer, filters: &[Filter]) -> (u32, HashMap<String, OwnedValue>) {
    if answer.cancelled {
        return (1, HashMap::new());
    }
    let mut out = HashMap::new();
    out.insert("uris".to_string(), OwnedValue::try_from(Value::from(answer.uris.clone())).expect("no fds"));
    if let Some(filter) = answer.current_filter.and_then(|i| filters.get(i)) {
        if let Ok(value) = OwnedValue::try_from(Value::from(filter.clone())) {
            out.insert("current_filter".to_string(), value);
        }
    }
    (0, out)
}

impl FileChooser {
    async fn run(&self, server: &ObjectServer, handle: ObjectPath<'_>, request: Request) -> (u32, HashMap<String, OwnedValue>) {
        let filters = request.filters.clone();
        let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("stratum-files"));
        let child = async_process::Command::new(exe)
            .arg("--choose")
            .stdin(async_process::Stdio::piped())
            .stdout(async_process::Stdio::piped())
            .spawn();
        let mut child = match child {
            Ok(child) => child,
            Err(err) => {
                tracing::error!("could not start the chooser: {err}");
                return (2, HashMap::new());
            }
        };
        let key = handle.to_string();
        if let Ok(mut running) = self.running.0.lock() {
            running.insert(key.clone(), child.id());
        }
        let object = RequestObject { running: self.running.clone(), handle: key.clone() };
        let served = server.at(&handle, object).await.unwrap_or(false);
        if let Some(mut stdin) = child.stdin.take() {
            use futures_lite::AsyncWriteExt;
            let _ = stdin.write_all(serde_json::to_string(&request).unwrap_or_default().as_bytes()).await;
        }
        let output = child.output().await;
        if served {
            let _ = server.remove::<RequestObject, _>(&handle).await;
        }
        if let Ok(mut running) = self.running.0.lock() {
            running.remove(&key);
        }
        let answer = output
            .ok()
            .and_then(|o| serde_json::from_slice::<Answer>(&o.stdout).ok())
            .unwrap_or(Answer { cancelled: true, ..Answer::default() });
        results(&answer, &filters)
    }
}

#[zbus::interface(name = "org.freedesktop.impl.portal.FileChooser")]
impl FileChooser {
    #[allow(clippy::too_many_arguments)]
    async fn open_file(
        &self,
        handle: ObjectPath<'_>,
        _app_id: &str,
        _parent_window: &str,
        title: &str,
        options: HashMap<String, OwnedValue>,
        #[zbus(object_server)] server: &ObjectServer,
    ) -> (u32, HashMap<String, OwnedValue>) {
        self.run(server, handle, request(title, false, &options)).await
    }

    #[allow(clippy::too_many_arguments)]
    async fn save_file(
        &self,
        handle: ObjectPath<'_>,
        _app_id: &str,
        _parent_window: &str,
        title: &str,
        options: HashMap<String, OwnedValue>,
        #[zbus(object_server)] server: &ObjectServer,
    ) -> (u32, HashMap<String, OwnedValue>) {
        self.run(server, handle, request(title, true, &options)).await
    }

    /// Several files at once: a folder is chosen, and each file is saved in
    /// it under its own name.
    #[allow(clippy::too_many_arguments)]
    async fn save_files(
        &self,
        handle: ObjectPath<'_>,
        _app_id: &str,
        _parent_window: &str,
        title: &str,
        options: HashMap<String, OwnedValue>,
        #[zbus(object_server)] server: &ObjectServer,
    ) -> (u32, HashMap<String, OwnedValue>) {
        let mut request = request(title, false, &options);
        request.directory = true;
        request.multiple = false;
        let names: Vec<Vec<u8>> = options.get("files").and_then(|v| Vec::<Vec<u8>>::try_from(v.try_clone().ok()?).ok()).unwrap_or_default();
        let (response, mut results) = self.run(server, handle, request).await;
        if response != 0 {
            return (response, results);
        }
        let folder = results
            .get("uris")
            .and_then(|v| Vec::<String>::try_from(v.try_clone().ok()?).ok())
            .and_then(|u| u.first().and_then(|u| stratum_ui::uri::to_path(u)));
        let Some(folder) = folder else { return (2, HashMap::new()) };
        let uris: Vec<String> = names
            .iter()
            .map(|n| n.iter().copied().take_while(|b| *b != 0).collect::<Vec<u8>>())
            .map(|n| stratum_ui::uri::from_path(&folder.join(<std::ffi::OsString as std::os::unix::ffi::OsStringExt>::from_vec(n))))
            .collect();
        results.insert("uris".into(), OwnedValue::try_from(Value::from(uris)).expect("no fds"));
        (0, results)
    }

    #[zbus(property, name = "version")]
    fn version(&self) -> u32 {
        4
    }
}

/// Serve the FileChooser until the session ends.
pub fn serve() -> Result<(), String> {
    zbus::block_on(async {
        let _conn = zbus::connection::Builder::session()?
            .name(BUS_NAME)?
            .serve_at(PATH, FileChooser { running: Running::default() })?
            .build()
            .await?;
        tracing::info!("serving the FileChooser portal as {BUS_NAME}");
        std::future::pending::<()>().await;
        Ok::<(), zbus::Error>(())
    })
    .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn value<'a>(v: impl Into<Value<'a>>) -> OwnedValue {
        OwnedValue::try_from(v.into()).unwrap()
    }

    #[test]
    fn options_become_a_request() {
        let mut options = HashMap::new();
        options.insert("accept_label".to_string(), value("_Export"));
        options.insert("multiple".to_string(), value(true));
        options.insert("current_folder".to_string(), value(b"/home/me\0".to_vec()));
        let png: Filter = ("PNG".into(), vec![(0, "*.png".into())]);
        let all: Filter = ("All".into(), vec![(0, "*".into())]);
        options.insert("filters".to_string(), value(vec![png.clone(), all.clone()]));
        options.insert("current_filter".to_string(), value(all.clone()));
        let request = request("Export", false, &options);
        assert!(request.multiple && !request.save);
        assert_eq!(request.current_folder, Some(PathBuf::from("/home/me")));
        assert_eq!(request.accept_label.as_deref(), Some("_Export"));
        assert_eq!(request.current_filter, Some(1));

        let mut save = HashMap::new();
        save.insert("current_file".to_string(), value(b"/tmp/report.pdf\0".to_vec()));
        let request = super::request("Save", true, &save);
        assert_eq!(request.current_folder, Some(PathBuf::from("/tmp")));
        assert_eq!(request.current_name.as_deref(), Some("report.pdf"));
    }

    #[test]
    fn answers_become_results() {
        let (response, results) = results(&Answer { cancelled: true, ..Answer::default() }, &[]);
        assert_eq!((response, results.len()), (1, 0));
        let filters = vec![("PNG".to_string(), vec![(0, "*.png".to_string())])];
        let answer = Answer { uris: vec!["file:///a.png".into()], current_filter: Some(0), cancelled: false };
        let (response, results) = super::results(&answer, &filters);
        assert_eq!(response, 0);
        assert_eq!(Vec::<String>::try_from(results["uris"].try_clone().unwrap()).unwrap(), ["file:///a.png"]);
        assert!(results.contains_key("current_filter"));
    }
}
