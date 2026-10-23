//! The session lock: taking it, its surfaces, PAM results, and unlocking.

use super::*;

impl App {
    /// Paint the lock surfaces that need it. Errors are logged per surface
    /// rather than returned, so one output's failure leaves the others drawn.
    pub(super) fn render_lock(&mut self, dt: f32) {
        if !self.lock.active() {
            return;
        }
        let now = chrono::Local::now();
        if self.lock.clock_changed(now.timestamp() / 60) {
            for entry in &mut self.lock_surfaces {
                entry.dirty = true;
            }
        }
        self.lock.set_metrics(&self.config.metrics.lock);
        let time = now.format("%I:%M %p").to_string();
        let theme = self.config.active_theme();
        for entry in &mut self.lock_surfaces {
            if !entry.configured || !entry.dirty {
                continue;
            }
            let output_name = entry.output_name.clone();
            let mut ctx = paint_ctx!(self, &theme, &output_name, dt);
            let mut list = crate::render::display_list::DisplayList::new(theme.bg_dark.with_alpha(1.0));
            self.lock
                .paint(&mut ctx, &mut list, entry.logical_rect(), &self.lock_art, &time);
            if let Err(err) = entry.render_list(&self.gpu, self.display, &list, &mut self.text) {
                tracing::error!("lock surface on {output_name} failed to render: {err}");
            }
        }
    }

    /// Route lock-screen worker results (PAM answers, decoded artwork) into
    /// the event loop.
    pub(super) fn start_lock_events(&mut self, loop_handle: &calloop::LoopHandle<'static, App>) {
        let (tx, channel) = calloop::channel::channel();
        self.lock_events = Some(tx);
        if let Err(err) = loop_handle.insert_source(channel, |event, _, app: &mut App| {
            if let calloop::channel::Event::Msg(event) = event {
                app.handle_lock_event(event);
            }
        }) {
            warn!("failed to insert lock event source: {err}");
        }
    }

    fn handle_lock_event(&mut self, event: LockEvent) {
        match event {
            LockEvent::Auth(result) => {
                if self.lock.phase() != crate::lock::Phase::Locked {
                    return;
                }
                if let Err(err) = &result {
                    info!("unlock attempt failed: {}", err.message);
                }
                if self.lock.auth_result(result) {
                    info!("password accepted; unlocking");
                    self.end_lock(true);
                } else {
                    self.mark_lock_dirty();
                }
            }
            LockEvent::Artwork(spec, art) => {
                // A newer decode may have been requested since this one began.
                if self.lock_art_spec.as_ref() == Some(&spec) {
                    self.lock_art = art;
                    self.mark_lock_dirty();
                }
            }
        }
    }

    pub(super) fn mark_lock_dirty(&mut self) {
        for entry in &mut self.lock_surfaces {
            entry.dirty = true;
        }
    }

    /// Decode the lock artwork on a worker thread, unless the current decode
    /// already matches the configured files and their modification times.
    pub(super) fn load_lock_art(&mut self) {
        let scale = self
            .output_state
            .outputs()
            .filter_map(|output| self.output_state.info(&output))
            .map(|info| info.scale_factor)
            .max()
            .unwrap_or(1);
        let spec = crate::lock::ArtworkSpec::new(&self.config.lock, &self.config.metrics.lock, scale);
        if self.lock_art_spec.as_ref() == Some(&spec) {
            return;
        }
        let Some(sender) = self.lock_events.clone() else {
            return;
        };
        self.lock_art_spec = Some(spec.clone());
        std::thread::spawn(move || {
            let art = spec.load();
            let _ = sender.send(LockEvent::Artwork(spec, art));
        });
    }

    /// Ask the compositor to lock the session.
    ///
    /// Refuses whenever the lock could not be undone: no PAM service exists,
    /// or its modules cannot load in this process. A lock nobody can unlock is
    /// worse than none. Refusals answer `unsupported` (with the reason in the
    /// log), the same as a compositor without the session-lock protocol, and
    /// the CLI reports it.
    pub(super) fn begin_lock(&mut self) -> Result<(), IpcError> {
        if self.lock.active() {
            return Ok(());
        }
        let refuse = |reason: String| {
            tracing::error!("refusing to lock: {reason}");
            IpcError::Unsupported
        };
        let confdir = crate::pam::test_confdir();
        let service = crate::pam::resolve_service(&self.config.lock.pam_service, confdir.as_deref())
            .ok_or_else(|| refuse("no usable PAM service in /etc/pam.d".into()))?;
        crate::pam::check_service(&service, confdir.as_deref())
            .map_err(|reason| refuse(format!("PAM service '{service}' cannot run in this build: {reason}")))?;
        let user = crate::pam::current_user()
            .ok_or_else(|| refuse("cannot determine the user".into()))?;
        let session_lock = self
            .session_lock_state
            .lock(&self.queue_handle)
            .map_err(|_| refuse("the compositor does not offer ext-session-lock-v1".into()))?;
        info!("locking session (PAM service '{service}', user '{user}')");

        self.lock.request();
        self.lock_auth = Some(LockAuth { service, user });
        self.session_lock = Some(session_lock);
        // Nothing else should hold the keyboard or a popup over the lock.
        self.power_menu.close();
        self.hover_menus.intent.close();
        if self.tray_menu.target().is_some() {
            self.actions.push(crate::actions::SystemAction::TrayMenuClose);
            self.tray_menu.close();
            self.tray_menu_geometry = None;
        }
        let qh = self.queue_handle.clone();
        self.create_lock_surfaces(&qh);
        self.load_lock_art();
        Ok(())
    }

    /// Give every output a lock surface. Also called when an output appears
    /// while locked: an output without one would show nothing, but the
    /// compositor may still route input to it.
    pub(super) fn create_lock_surfaces(&mut self, qh: &QueueHandle<Self>) {
        let Some(session_lock) = self.session_lock.clone() else {
            return;
        };
        for output in self.output_state.outputs() {
            if self.lock_surfaces.iter().any(|entry| entry.output == output) {
                continue;
            }
            let Some(info) = self.output_state.info(&output) else {
                continue;
            };
            let name = info.name.clone().unwrap_or_else(|| info.model.clone());
            let wl = self.compositor_state.create_surface(qh);
            let surface = session_lock.create_lock_surface(wl, &output, qh);
            debug!("lock surface created on '{name}'");
            self.lock_surfaces
                .push(LockSurface::new(surface, output, name, info.scale_factor));
        }
    }

    /// Tear the lock down. `unlock` is true only after a successful PAM check;
    /// when the compositor ended the lock itself, the object is just destroyed.
    fn end_lock(&mut self, unlock: bool) {
        if let Some(session_lock) = self.session_lock.take() {
            if unlock {
                session_lock.unlock();
            }
        }
        // Surfaces after the unlock request, so the compositor never sees a
        // locked session without a lock surface.
        self.lock_surfaces.clear();
        self.lock_auth = None;
        self.lock.finish();
        self.mark_all_dirty();
    }
}

impl SessionLockHandler for App {
    fn locked(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _lock: SessionLock) {
        info!("session locked");
        self.lock.confirm();
        self.mark_lock_dirty();
    }

    fn finished(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _lock: SessionLock) {
        // Either the compositor refused the lock (another client holds one)
        // or it ended it. In both cases unlocking is not ours to request.
        if self.lock.phase() == crate::lock::Phase::Locking {
            warn!("compositor refused the session lock");
        } else {
            warn!("compositor ended the session lock");
        }
        self.end_lock(false);
    }

    fn configure(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        surface: SessionLockSurface,
        configure: SessionLockSurfaceConfigure,
        _serial: u32,
    ) {
        let (width, height) = configure.new_size;
        if let Some(entry) = self
            .lock_surfaces
            .iter_mut()
            .find(|entry| entry.surface.wl_surface() == surface.wl_surface())
        {
            entry.resize(&self.gpu, width, height);
        }
    }
}
