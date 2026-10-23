//! The system managers, configuration reload, and the backlight reading.

use super::*;

impl App {
    /// Start the system managers and compositor watcher.
    ///
    /// The managers publish into `SystemState` from their own threads, so the
    /// UI thread only ever reads. Hyprland changes wake the render loop by
    /// marking every surface dirty.
    pub(super) fn start_monitors(&mut self) {
        use stratum_sys::managers;

        let publisher = SystemState::publisher(&self.state);
        self.actions.start_worker(Arc::clone(&self.state));
        self.state.dashboard_feed.spawn(Arc::clone(&self.state));
        self.dashboard.load_focus();

        // The managers' async entry points expect a Tokio runtime. The shell's
        // UI thread runs calloop, so give them a small dedicated runtime.
        managers::audio::initialize();
        let audio_publisher = publisher.clone();
        std::thread::Builder::new()
            .name("stratum-runtime".into())
            .spawn(move || {
                let runtime = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(runtime) => runtime,
                    Err(err) => {
                        tracing::error!("failed to start manager runtime: {err}");
                        return;
                    }
                };
                runtime.block_on(async move {
                    managers::audio::spawn_monitor(audio_publisher);
                    // Keep the runtime alive for the shell's lifetime.
                    std::future::pending::<()>().await;
                });
            })
            .ok();

        // Safety-net poller. User-initiated changes refresh their own domain
        // synchronously (see `actions`), so this only has to catch changes made
        // outside the shell. A slow cadence is fine and keeps idle CPU low.
        let state = Arc::clone(&self.state);
        let poll_ms = self.config.behavior.poll_ms.max(250);
        std::thread::Builder::new()
            .name("stratum-poll".into())
            .spawn(move || loop {
                state.net.set(managers::net::status());
                let wifi = managers::wifi::state();
                let wifi_enabled = wifi["enabled"].as_bool().unwrap_or(false);
                state.wifi.set(wifi);
                if wifi_enabled {
                    // Re-read the cache only: forcing a rescan here would put
                    // radio work on a timer for no reason.
                    state.wifi_networks.set(managers::wifi::list(false));
                }
                state.bluetooth.set(managers::bluetooth::status_with_all());
                state.battery.set(managers::battery::status());
                state.notify();
                std::thread::sleep(std::time::Duration::from_millis(poll_ms));
            })
            .ok();

        // Own the tray watcher when configured to.
        if self.config.behavior.tray {
            let sink_state = Arc::clone(&self.state);
            let sink: crate::tray::TraySink =
                Arc::new(move |items| {
                    if sink_state.tray.set(items) {
                        sink_state.notify();
                    }
                });
            let tray = crate::tray::Tray::new(sink);
            self.tray = Some(Arc::clone(&tray));
            // The action worker uses this for activation and menu requests.
            self.state.set_tray_handle(Arc::clone(&tray));
            std::thread::Builder::new()
                .name("stratum-tray".into())
                .spawn(move || {
                    let runtime = match tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                    {
                        Ok(runtime) => runtime,
                        Err(err) => {
                            tracing::error!("failed to start tray runtime: {err}");
                            return;
                        }
                    };
                    if let Err(err) = runtime.block_on(crate::tray::serve(tray)) {
                        tracing::warn!("tray watcher unavailable: {err}");
                    }
                })
                .ok();
        }

        if self.config.behavior.polkit {
            self.start_polkit_agent(publisher.clone());
        }

        // Own org.freedesktop.Notifications when configured to. A dev build
        // beside the installed shell turns it off and leaves the name to it.
        if self.config.behavior.notifications {
            let sink_state = Arc::clone(&self.state);
            let sink: crate::notifications::NotificationSink =
                Arc::new(move |list| {
                    if sink_state.notifications.set(list) {
                        sink_state.notify();
                    }
                });
            let server = crate::notifications::NotificationServer::new(sink);
            // Put back what the last run left, before any client can connect.
            // Restored notifications are in the centre but already seen, so
            // they do not pop up as toasts again.
            if let Some(snapshot) = crate::notification_store::load() {
                info!("restoring {} notifications", snapshot.notifications.len());
                for n in &snapshot.notifications {
                    self.toasts.dismiss(n.id);
                }
                self.state.dnd.set(snapshot.do_not_disturb);
                server.restore(snapshot.notifications, snapshot.next_id);
            }
            self.notification_saver =
                crate::notification_store::default_path().map(crate::notification_store::Saver::start);
            self.notifications = Some(Arc::clone(&server));
            std::thread::Builder::new()
                .name("stratum-notifications".into())
                .spawn(move || {
                    let runtime = match tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                    {
                        Ok(runtime) => runtime,
                        Err(err) => {
                            tracing::error!("failed to start notification runtime: {err}");
                            return;
                        }
                    };
                    if let Err(err) = runtime.block_on(server.serve()) {
                        tracing::error!("notification server failed: {err}");
                    }
                })
                .ok();
        }

        // Brightness has no push source, so it is polled. `brightnessctl` is
        // two short-lived processes per reading; at 350 ms that was the largest
        // remaining idle cost (and it spawned a process just to discover there
        // is no backlight). Read the sysfs node directly when possible, at
        // QML's 350 ms so brightness keys show the OSD promptly; the
        // `brightnessctl` fallback keeps a slower cadence.
        let brightness_state = Arc::clone(&self.state);
        std::thread::Builder::new()
            .name("stratum-brightness".into())
            .spawn(move || {
                let mut last: Option<i64> = None;
                loop {
                    let sysfs = read_brightness_sysfs();
                    let reading = sysfs.or_else(read_brightness_tool);
                    if let Some(percent) = reading {
                        if last != Some(percent) {
                            last = Some(percent);
                            brightness_state.brightness.set(Some(percent));
                            brightness_state.notify();
                        }
                    }
                    let interval = if sysfs.is_some() { 350 } else { 1000 };
                    std::thread::sleep(std::time::Duration::from_millis(interval));
                }
            })
            .ok();

        // Hyprland is edge-driven: re-read on each relevant event.
        let state = Arc::clone(&self.state);
        crate::hyprland::HyprState::spawn_watcher(move || {
            state.hypr.set(crate::hyprland::HyprState::read());
            state.notify();
        });
        // Seed once so the first frame is correct.
        self.state.hypr.set(crate::hyprland::HyprState::read());
    }

    /// Reload the configuration when the file changed on disk.
    ///
    /// Checked at most once a second so a burst of editor writes does not
    /// trigger repeated reloads. A parse error leaves the old config in place
    /// and logs the problem, so a typo cannot blank a running shell.
    pub(super) fn maybe_reload_config(&mut self) {
        if !self.config.behavior.reload_on_change {
            return;
        }
        if self.last_reload_check.elapsed() < std::time::Duration::from_secs(1) {
            return;
        }
        self.last_reload_check = std::time::Instant::now();

        // A file that appears, changes or goes away all change the stamp.
        let stamp = self.config_source.user_stamp();
        if stamp == self.config_stamp {
            return;
        }
        self.config_stamp = stamp;

        // The same loader as at startup, so the defaults and the checks are
        // identical.
        match self.config_source.load() {
            Ok(mut config) => {
                info!("reloaded configuration");
                // A live pick survives the reload unless the file changed its
                // own theme since.
                let had_saved_theme = self.theme_choice.is_some();
                self.theme_choice = crate::theme_choice::apply(&mut config, self.theme_choice.take());
                if had_saved_theme && self.theme_choice.is_none() {
                    crate::theme_choice::store(None);
                }
                let width_changed = config.sidebar.width != self.config.sidebar.width
                    || config.sidebar.position != self.config.sidebar.position
                    || config.sidebar.inverted_corners != self.config.sidebar.inverted_corners
                    || config.metrics.bar.corner_size != self.config.metrics.bar.corner_size;
                self.config = Arc::new(config);
                self.sidebar = Sidebar::new(Arc::clone(&self.config));
                // An open settings window would otherwise keep the old
                // config until restart.
                self.control_center.set_config(Arc::clone(&self.config));
                if width_changed {
                    info!("sidebar geometry changed; restart to apply the new anchorage");
                }
                self.mark_all_dirty();
            }
            Err(err) => {
                tracing::error!("config reload rejected: {err}\nkeeping the previous configuration");
            }
        }
    }
}

/// Read the backlight percentage with `brightnessctl`, for when the sysfs
/// nodes are unavailable.
///
/// Whether the tool exists is cached for the process lifetime, because a
/// machine without a backlight will never grow one and each probe spawns two
/// processes. Returns `None` when there is no backlight, which is normal on
/// desktops; the OSD then simply never shows brightness.
fn read_brightness_tool() -> Option<i64> {
    static TOOL_AVAILABLE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();

    if !*TOOL_AVAILABLE.get_or_init(brightnessctl_available) {
        return None;
    }
    read_brightnessctl()
}

/// Read `brightness`/`max_brightness` from the first `/sys/class/backlight`
/// device that exposes both.
fn read_brightness_sysfs() -> Option<i64> {
    let entries = std::fs::read_dir("/sys/class/backlight").ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        let current = std::fs::read_to_string(path.join("brightness")).ok()?;
        let max = std::fs::read_to_string(path.join("max_brightness")).ok()?;
        let current = current.trim().parse::<i64>().ok()?;
        let max = max.trim().parse::<i64>().ok()?;
        if max > 0 {
            return Some(((current as f64 * 100.0) / max as f64).round() as i64);
        }
    }
    None
}

fn brightnessctl_available() -> bool {
    std::process::Command::new("brightnessctl")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

fn read_brightnessctl() -> Option<i64> {
    let read = |arg: &str| {
        let out = std::process::Command::new("brightnessctl").arg(arg).output().ok()?;
        if !out.status.success() {
            return None;
        }
        String::from_utf8_lossy(&out.stdout).trim().parse::<i64>().ok()
    };
    let (current, max) = (read("get")?, read("max")?);
    if max <= 0 {
        return None;
    }
    Some(((current as f64 * 100.0) / max as f64).round() as i64)
}
