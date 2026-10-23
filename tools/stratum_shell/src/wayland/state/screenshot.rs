//! The screenshot overlay and the viewer window it opens.

use super::*;

impl App {
    /// Route screenshot worker results (frozen frames, captures) into the
    /// event loop.
    pub(super) fn start_screenshot_events(&mut self, loop_handle: &calloop::LoopHandle<'static, App>) {
        let (tx, channel) = calloop::channel::channel();
        self.screenshot_events = Some(tx);
        if let Err(err) = loop_handle.insert_source(channel, |event, _, app: &mut App| {
            if let calloop::channel::Event::Msg(event) = event {
                app.handle_screenshot_event(event);
            }
        }) {
            warn!("failed to insert screenshot event source: {err}");
        }
    }

    fn handle_screenshot_event(&mut self, event: ScreenshotEvent) {
        match event {
            ScreenshotEvent::Frozen { output, width, height, pixels } => {
                self.screenshot.frozen(&output, width, height, pixels);
            }
            ScreenshotEvent::FreezeFailed { error } => {
                // Picking over a live screen would capture something other
                // than what is shown, so give up, as QML did.
                if self.screenshot.is_open() {
                    self.screenshot.close();
                    self.actions.push(crate::screenshot::failure("Start failed", &error));
                }
            }
            ScreenshotEvent::Captured { path, mode, image } => {
                self.screenshot.close();
                self.viewer.show(path, mode, true, image);
                self.ensure_viewer_window();
            }
            ScreenshotEvent::CaptureFailed { error } => {
                self.screenshot.capture_failed();
                self.actions.push(crate::screenshot::failure("Capture failed", &error));
            }
            ScreenshotEvent::Posted(result) => {
                if let Err(error) = &result {
                    self.actions.push(crate::screenshot::failure("Viewer action failed", error));
                }
                self.viewer.finished(result);
            }
            ScreenshotEvent::SaveAsChosen(result) => match result {
                Ok(Some(target)) => {
                    self.viewer.set_working(false);
                    self.run_viewer_post(crate::screenshot_viewer::PostAction::SaveTo, Some(target));
                }
                Ok(None) => self.viewer.set_working(false),
                Err(error) => self.viewer.finished(Err(error)),
            },
        }
        self.mark_screenshot_dirty();
        self.mark_viewer_dirty();
    }

    pub(super) fn mark_viewer_dirty(&mut self) {
        if let Some(window) = &mut self.viewer_window {
            window.dirty = true;
        }
    }

    /// Create the viewer's window, or bring it forward when it exists.
    fn ensure_viewer_window(&mut self) {
        let qh = self.queue_handle.clone();
        let app_id = "org.stratum.ScreenshotViewer";
        if self.viewer_window.is_none() {
            let surface = self.compositor_state.create_surface(&qh);
            let window = self.xdg_shell.create_window(surface, WindowDecorations::RequestServer, &qh);
            window.set_title("Screenshot Viewer");
            window.set_app_id(app_id);
            let m = self.viewer.metrics();
            window.set_min_size(Some((m.min_width, m.min_height)));
            let size = (m.width, m.height);
            window.commit();
            // As sharp as the sharpest output, like the settings window.
            let scale = self.surfaces.iter().map(|s| s.scale).max().unwrap_or(1);
            self.viewer_window = Some(crate::wayland::viewer_window::ViewerWindow::new(window, scale, size));
        }
        if let (Some(activation), Some(window)) = (&self.activation, &self.viewer_window) {
            activation.request_token(
                &qh,
                RequestData {
                    seat_and_serial: None,
                    surface: Some(window.window.wl_surface().clone()),
                    app_id: Some(app_id.to_string()),
                    udata: (),
                },
            );
        }
        self.mark_viewer_dirty();
    }

    pub(super) fn close_viewer(&mut self) {
        self.viewer.close();
        // Dropping the window unmaps it.
        self.viewer_window = None;
    }

    /// Carry out what the viewer asked for.
    pub(super) fn apply_viewer_action(&mut self, action: crate::screenshot_viewer::ViewerAction) {
        use crate::screenshot_viewer::ViewerAction;
        match action {
            ViewerAction::None => return,
            ViewerAction::Redraw => {}
            ViewerAction::Close => self.close_viewer(),
            ViewerAction::Post { action, target } => self.run_viewer_post(action, target),
            ViewerAction::SaveAs => {
                let Some(sender) = self.screenshot_events.clone() else {
                    return;
                };
                self.viewer.set_working(true);
                let timeout = std::time::Duration::from_millis(self.viewer.metrics().save_as_timeout_ms);
                std::thread::spawn(move || {
                    let result = crate::screenshot_viewer::portal_save_as(timeout);
                    let _ = sender.send(ScreenshotEvent::SaveAsChosen(result));
                });
            }
        }
        self.mark_viewer_dirty();
    }

    /// Run a `screenshot-post` action on the shown image, composing its
    /// strokes into a temporary file first when there are any.
    fn run_viewer_post(&mut self, action: crate::screenshot_viewer::PostAction, target: Option<String>) {
        let (Some(sender), Some(image)) = (self.screenshot_events.clone(), self.viewer.image().cloned())
        else {
            return;
        };
        let strokes = self.viewer.strokes().to_vec();
        let source = self.viewer.path().to_string();
        self.viewer.set_working(true);
        std::thread::spawn(move || {
            let result = if strokes.is_empty() {
                crate::screenshot_viewer::post(action, &source, target.as_deref())
            } else {
                crate::screenshot_viewer::export(&image, &strokes).and_then(|temp| {
                    let result = crate::screenshot_viewer::post(action, &temp, target.as_deref());
                    let _ = std::fs::remove_file(&temp);
                    result
                })
            };
            let _ = sender.send(ScreenshotEvent::Posted(result));
        });
        self.mark_viewer_dirty();
    }

    /// One pointer event on the viewer window.
    pub(super) fn viewer_pointer(&mut self, kind: &PointerEventKind, x: f32, y: f32) {
        let point = crate::render::display_list::Point::new(x, y);
        let ctrl = self.modifiers.ctrl;
        let action = match kind {
            PointerEventKind::Enter { .. } | PointerEventKind::Motion { .. } => self.viewer.motion(point),
            PointerEventKind::Leave { .. } => {
                self.viewer.leave();
                crate::screenshot_viewer::ViewerAction::Redraw
            }
            PointerEventKind::Press { button, .. } if *button == BTN_LEFT || *button == BTN_MIDDLE => {
                self.viewer.press(point, *button == BTN_MIDDLE, ctrl)
            }
            PointerEventKind::Release { button, .. } if *button == BTN_LEFT || *button == BTN_MIDDLE => {
                self.viewer.release()
            }
            PointerEventKind::Axis { vertical, .. } => {
                let steps = if vertical.value120 != 0 {
                    vertical.value120 as f32 / 120.0
                } else if vertical.discrete != 0 {
                    vertical.discrete as f32
                } else {
                    vertical.absolute as f32
                };
                self.viewer.scroll(point, steps, ctrl)
            }
            _ => crate::screenshot_viewer::ViewerAction::None,
        };
        self.apply_viewer_action(action);
    }

    /// Open the overlay on every output and freeze each one.
    pub(super) fn start_screenshot(&mut self) {
        let output = self.focused_output();
        if !self.screenshot.open_on(&output) {
            return;
        }
        let Some(sender) = self.screenshot_events.clone() else {
            return;
        };
        for entry in &self.screenshot_surfaces {
            let output = entry.output_name.clone();
            let sender = sender.clone();
            std::thread::spawn(move || {
                let event = match crate::screenshot::freeze(&output) {
                    Ok((width, height, pixels)) => {
                        ScreenshotEvent::Frozen { output, width, height, pixels }
                    }
                    Err(error) => ScreenshotEvent::FreezeFailed { error },
                };
                let _ = sender.send(event);
            });
        }
        self.mark_screenshot_dirty();
    }

    /// Run a capture picked on `output`: cut from its frozen frame, or, with
    /// no frame, taken with `grim` once the overlay is off screen.
    fn run_screenshot_capture(&mut self, output: &str, capture: crate::screenshot::Capture) {
        let Some(sender) = self.screenshot_events.clone() else {
            return;
        };
        let frozen = self.screenshot.crop_frozen(
            output,
            self.output_origin(output),
            self.output_logical_size(output),
            &capture,
        );
        let delay = std::time::Duration::from_millis(self.screenshot.metrics().capture_delay_ms);
        std::thread::spawn(move || {
            let mode = capture.mode();
            let captured = match frozen {
                Some((width, height, pixels)) => crate::screenshot::save_png(width, height, &pixels)
                    .map(|path| {
                        let image = crate::screenshot_viewer::Image { width, height, pixels: Arc::from(pixels) };
                        (path, image)
                    }),
                None => {
                    std::thread::sleep(delay);
                    // Decoded here too, so the viewer opens with its image.
                    crate::screenshot::capture(&capture).and_then(|path| {
                        match crate::screenshot_viewer::load(&path) {
                            Ok(image) => Ok((path, image)),
                            Err(error) => {
                                let _ = std::fs::remove_file(&path);
                                Err(error)
                            }
                        }
                    })
                }
            };
            let event = match captured {
                Ok((path, image)) => ScreenshotEvent::Captured { path, mode, image },
                Err(error) => ScreenshotEvent::CaptureFailed { error },
            };
            let _ = sender.send(event);
        });
        self.mark_screenshot_dirty();
    }

    /// One pointer event on `output`'s screenshot overlay.
    pub(super) fn screenshot_pointer(&mut self, output: &str, kind: &PointerEventKind, x: f32, y: f32) {
        let point = crate::render::display_list::Point::new(x, y);
        let origin = self.output_origin(output);
        if let (PointerEventKind::Enter { serial }, Some(device)) = (kind, &self.cursor_shape_device) {
            // Set per surface: the compositor restores its own on leave.
            device.set_shape(
                *serial,
                smithay_client_toolkit::reexports::protocols::wp::cursor_shape::v1::client::wp_cursor_shape_device_v1::Shape::Crosshair,
            );
        }
        match kind {
            PointerEventKind::Enter { .. } | PointerEventKind::Motion { .. } => {
                // Windows on the output's active workspace, from the state the
                // shell already keeps.
                let hypr = self.state.hypr.get();
                let workspaces: Vec<i32> = hypr.active.get(output).copied().into_iter().collect();
                let window = hypr
                    .window_at(origin.0 + x as i32, origin.1 + y as i32, &workspaces)
                    .map(|c| crate::screenshot::GlobalRect {
                        x: c.x,
                        y: c.y,
                        width: c.width,
                        height: c.height,
                    });
                self.screenshot.motion(output, point, window);
            }
            PointerEventKind::Leave { .. } => self.screenshot.leave(output),
            PointerEventKind::Press { button, .. } if *button == BTN_LEFT => {
                self.screenshot.press(output, point);
            }
            PointerEventKind::Release { button, .. } if *button == BTN_LEFT => {
                if let Some(capture) = self.screenshot.release(output, origin) {
                    self.run_screenshot_capture(output, capture);
                }
            }
            _ => return,
        }
        self.mark_screenshot_dirty();
    }

    pub(super) fn mark_screenshot_dirty(&mut self) {
        for entry in &mut self.screenshot_surfaces {
            entry.dirty = true;
        }
    }
}
