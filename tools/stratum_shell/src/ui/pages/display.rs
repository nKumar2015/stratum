//! Display page: arrange the monitors and set their modes, applied to
//! Hyprland.
//!
//! The page edits a draft of every monitor's settings. A map of the enabled
//! monitors, drawn at their logical sizes, lets one be dragged; it is always
//! put against another's edge, never over it (`displays::place`). The
//! selected monitor's resolution, refresh rate, scale, rotation and VRR are
//! chips and switches below. Apply sends what changed to the display
//! manager (`display_manager.rs`), which puts it on trial: a bar asks to
//! keep it and counts down to putting the old layout back. A kept layout is
//! saved for this set of monitors and survives reloads and restarts until
//! "Use Hyprland config" forgets it.
//!
//! While the draft matches what it was made from, it follows the monitors as
//! they change; once edited it holds until applied or discarded.

use crate::actions::SystemAction;
use crate::display_manager::Command;
use crate::displays::{self, Area, Settings};
use crate::render::display_list::{DisplayList, Rect};
use crate::ui::controls;
use crate::ui::pages::{devices, layout};
use crate::ui::{self, InputState, PaintCtx};

/// Rotations, as Hyprland's transform without the flip.
const ROTATIONS: [&str; 4] = ["Normal", "90\u{b0}", "180\u{b0}", "270\u{b0}"];

/// How layout coordinates map onto the map: `screen = origin + (layout - base) * k`.
#[derive(Clone, Copy, Debug)]
struct Frame {
    base: (f32, f32),
    origin: (f32, f32),
    k: f32,
}

impl Frame {
    fn to_screen(self, a: Area) -> Rect {
        layout::rect(
            self.origin.0 + (a.x as f32 - self.base.0) * self.k,
            self.origin.1 + (a.y as f32 - self.base.1) * self.k,
            a.w as f32 * self.k,
            a.h as f32 * self.k,
        )
    }

    fn to_layout(self, (x, y): (f32, f32)) -> (f32, f32) {
        (self.base.0 + (x - self.origin.0) / self.k, self.base.1 + (y - self.origin.1) / self.k)
    }
}

/// A monitor being dragged on the map. The frame is fixed for the drag, or
/// the map would rescale under the pointer as the layout's bounds change.
struct Drag {
    name: String,
    /// From the monitor's corner to the pointer, in layout pixels.
    grab: (f32, f32),
    frame: Frame,
}

#[derive(Default)]
pub struct DisplayPage {
    /// The settings being edited, by connector name.
    draft: Vec<(String, Settings)>,
    /// What the draft was made from.
    base: Vec<(String, Settings)>,
    selected: Option<String>,
    drag: Option<Drag>,
    /// Whether this showing of the page has asked for fresh monitors.
    refreshed: bool,
    /// A layout is on trial, so the countdown needs frames.
    trial: bool,
    /// The resolution dropdown is open.
    sizes_open: bool,
    /// What the open dropdown lists, set by the paint that drew its face:
    /// it is painted last, over the page.
    sizes_menu: Option<SizesMenu>,
    /// The open list's card as last painted. The pointer over it belongs to
    /// the list, not to the controls under it.
    list_rect: Option<Rect>,
    /// The scale slider is being dragged.
    scale_drag: bool,
    /// How far the open list is scrolled; `None` until it is first painted,
    /// which scrolls to the chosen option.
    list_scroll: Option<f32>,
    night_light: crate::ui::pages::night_light::NightLightSection,
}

/// The resolution dropdown's contents for this frame.
struct SizesMenu {
    monitor: String,
    anchor: Rect,
    sizes: Vec<displays::Mode>,
    labels: Vec<String>,
    selected: Option<usize>,
}

impl DisplayPage {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn animating(&self) -> bool {
        self.trial
    }

    /// Leaving the page drops an unapplied draft.
    pub fn on_hidden(&mut self) {
        self.draft.clear();
        self.base.clear();
        self.drag = None;
        self.refreshed = false;
        self.sizes_open = false;
        self.list_rect = None;
    }

    /// The wheel over the open list scrolls the list, not the page.
    /// Returns whether it was taken.
    pub fn on_scroll(&mut self, point: (f32, f32), delta: f32) -> bool {
        let over = self.list_rect.is_some_and(|r| {
            r.contains(crate::render::display_list::Point::new(point.0, point.1))
        });
        if over {
            layout::scroll_list(&mut self.list_scroll, delta);
        }
        over
    }

    /// Returns the bottom of the page.
    pub fn paint(
        &mut self,
        ctx: &mut PaintCtx<'_>,
        list: &mut DisplayList,
        area: Rect,
        input_all: &InputState,
        clicked_all: bool,
        pending: &mut Vec<SystemAction>,
    ) -> f32 {
        let m = ctx.metrics.page.clone();
        let typo = ctx.metrics.typography.clone();
        let theme = ctx.theme.clone();
        let snap = ctx.state.displays.get();
        // An open list lies over the page: the pointer over it is not over
        // what it covers, for hover or for clicks.
        let over_list = self.list_rect.is_some_and(|r| layout::hovered(input_all, r));
        let masked = InputState { pointer: if over_list { None } else { input_all.pointer }, pressed: input_all.pressed };
        let input = &masked;
        let clicked = clicked_all && !over_list;
        self.sizes_menu = None;
        if !self.refreshed {
            pending.push(SystemAction::Display(Command::Refresh));
            self.refreshed = true;
        }
        self.trial = snap.trial_until.is_some();

        let area = layout::column(area, m.max_width);
        let enabled_count = snap.monitors.iter().filter(|m| m.current.enabled).count();
        let summary = if !snap.available {
            String::new()
        } else {
            let source = if snap.saved { "saved layout" } else { "Hyprland config" };
            let plural = if snap.monitors.len() == 1 { "" } else { "s" };
            format!("{} monitor{plural}, {enabled_count} on \u{b7} {source}", snap.monitors.len())
        };
        let (head, body) = layout::header(ctx, list, area, "Display", &summary);
        if !snap.available || snap.monitors.is_empty() {
            // Night light needs only the gamma protocol, not Hyprland.
            let y = devices::empty_card(ctx, list, body, body.y, "No monitors to configure", "Display settings need Hyprland");
            return self.night_light.paint(ctx, list, body, y + m.section_spacing, input, clicked, pending);
        }

        // Follow the monitors while the draft is untouched, or when the set
        // of monitors changed under it.
        let current: Vec<(String, Settings)> = snap.monitors.iter().map(|m| (m.name.clone(), m.current)).collect();
        let names = |v: &[(String, Settings)]| v.iter().map(|(n, _)| n.clone()).collect::<Vec<_>>();
        // A drag that has not moved yet leaves the draft untouched, so only a
        // change of monitors ends it.
        if names(&self.draft) != names(&current) {
            self.drag = None;
            self.draft.clear();
            self.base.clear();
        }
        if self.draft == self.base {
            self.draft = current.clone();
            self.base = current.clone();
        }
        let selected_ok = self.selected.as_ref().is_some_and(|s| self.draft.iter().any(|(n, _)| n == s));
        if !selected_ok {
            self.selected = self
                .draft
                .iter()
                .find(|(_, s)| s.enabled)
                .or(self.draft.first())
                .map(|(n, _)| n.clone());
        }

        // Going back to the config's own rules.
        if snap.saved && !self.trial {
            let y = head.y + (head.height - m.badge) * 0.5;
            let (_, hit) = layout::text_buttons(ctx, list, head.x + head.width, y, &["Use Hyprland config"], true, input, clicked);
            if hit.is_some() {
                pending.push(SystemAction::Display(Command::Forget));
            }
        }

        let mut y = body.y;
        if let Some(error) = &snap.error {
            let line = layout::rect(body.x, y, body.width, m.section_label_height);
            let error = ui::elide(ctx, error, typo.caption, typo.weight_regular, body.width);
            ui::paint_text_vcentered(ctx, list, &error, body.x, line, typo.caption, typo.weight_regular, theme.error);
            y += line.height + m.label_gap;
        }

        y = layout::section(ctx, list, body, y, "Arrangement");
        y = self.paint_map(ctx, list, body, y, input, clicked);
        let hint = "Drag a monitor to move it; it snaps to the others' edges";
        let line = layout::rect(body.x, y + m.label_gap * 0.5, body.width, m.section_label_height);
        let hint = ui::elide(ctx, hint, typo.caption, typo.weight_regular, body.width);
        ui::paint_text_vcentered(ctx, list, &hint, body.x, line, typo.caption, typo.weight_regular, theme.text_muted);
        y = line.y + line.height + m.section_spacing;

        // Which monitor the settings below are for.
        let labels: Vec<String> = self.draft.iter().map(|(n, _)| n.clone()).collect();
        let selected = labels.iter().position(|n| Some(n) == self.selected.as_ref());
        y = layout::section(ctx, list, body, y, "Monitor");
        let (bottom, hit) = layout::chip_flow(ctx, list, body.x, y, body.width, &labels, selected, input, clicked);
        if let Some(i) = hit {
            if self.selected.as_ref() != Some(&labels[i]) {
                self.sizes_open = false;
            }
            self.selected = Some(labels[i].clone());
        }
        y = bottom + m.label_gap;
        if let Some(name) = self.selected.clone() {
            y = self.paint_settings(ctx, list, body, y, &name, &snap.monitors, input, clicked);
        }

        // Discard and Apply.
        y += m.section_spacing;
        let changes: Vec<(String, Settings)> = self
            .draft
            .iter()
            .filter(|(name, s)| current.iter().find(|(n, _)| n == name).is_none_or(|(_, c)| s.differs(c)))
            .cloned()
            .collect();
        let edited = self.draft != self.base;
        let apply_w = layout::text_button_width(ctx, "Apply");
        let apply_x = body.x + body.width - apply_w;
        let can_apply = !changes.is_empty() && !self.trial;
        if can_apply && layout::chip(ctx, list, apply_x, y, "Apply", true, input, clicked) {
            // Every monitor, so the whole layout is pinned: one the config
            // places `auto` would otherwise be moved by Hyprland.
            pending.push(SystemAction::Display(Command::Apply(self.draft.clone())));
            self.base = self.draft.clone();
        } else if !can_apply {
            layout::text_button(ctx, list, apply_x, y, "Apply", false, input, clicked);
        }
        if edited {
            let discard_w = layout::text_button_width(ctx, "Discard");
            let discard_x = apply_x - m.grid_gap * 0.5 - discard_w;
            if layout::text_button(ctx, list, discard_x, y, "Discard", true, input, clicked) {
                self.draft = current.clone();
                self.base = current;
            }
        }
        let y = y + m.badge + m.section_spacing;
        let bottom = self.night_light.paint(ctx, list, body, y, input, clicked, pending);

        // The resolution list, over everything else. A click anywhere but
        // the list or its face closes it.
        self.list_rect = None;
        match (self.sizes_open, self.sizes_menu.take()) {
            (true, Some(menu)) => {
                // Over the page's text as well as its shapes.
                list.push_layer();
                let (card, hit) =
                    layout::dropdown_list(
                        ctx,
                        list,
                        menu.anchor,
                        &menu.labels,
                        menu.selected,
                        &mut self.list_scroll,
                        input_all,
                        clicked_all,
                    );
                // The list ends at the window's bottom and scrolls, so it
                // never makes the page longer.
                self.list_rect = Some(card);
                if let Some(i) = hit {
                    if let Some(monitor) = snap.monitors.iter().find(|mon| mon.name == menu.monitor) {
                        self.set_size(monitor, menu.sizes[i], &ctx.displays.scales);
                    }
                    self.sizes_open = false;
                } else if clicked_all && !layout::hovered(input_all, card) && !layout::hovered(input_all, menu.anchor) {
                    self.sizes_open = false;
                }
            }
            // Its monitor was turned off or deselected.
            (true, None) => self.sizes_open = false,
            _ => {}
        }
        bottom
    }

    /// Give `monitor` the mode of `size`: at the same rate if it has it,
    /// else its fastest; at a scale that still gives a whole logical size;
    /// and against the others' edges again, since its size changed.
    fn set_size(&mut self, monitor: &displays::Monitor, size: displays::Mode, scales: &[f32]) {
        let Some(index) = self.draft.iter().position(|(n, _)| *n == monitor.name) else { return };
        let mut s = self.draft[index].1;
        let rates = monitor.rates(&size);
        let same_rate = rates.iter().find(|r| (r.refresh - s.mode.refresh).abs() < 0.5);
        s.mode = same_rate.or(rates.first()).copied().unwrap_or(size);
        let valid = displays::scales_for(&s.mode, scales, 1.0);
        if !valid.iter().any(|v| (v - s.scale).abs() < 0.005) {
            s.scale = 1.0;
        }
        let others: Vec<Area> =
            self.draft.iter().filter(|(n, o)| *n != monitor.name && o.enabled).map(|(_, o)| o.area()).collect();
        let (x, y) = displays::place(s.logical_size(), (s.x, s.y), &others, 0);
        s.x = x;
        s.y = y;
        self.draft[index].1 = s;
    }

    /// The map of enabled monitors, with dragging. Returns its bottom.
    fn paint_map(
        &mut self,
        ctx: &mut PaintCtx<'_>,
        list: &mut DisplayList,
        body: Rect,
        y: f32,
        input: &InputState,
        clicked: bool,
    ) -> f32 {
        let m = ctx.metrics.page.clone();
        let theme = ctx.theme.clone();
        let card = layout::rect(body.x, y, body.width, m.display_map_height);
        layout::card(ctx, list, card, layout::Card::Plain);
        let pad = m.card_padding;
        let inner = layout::rect(card.x + pad, card.y + pad, card.width - pad * 2.0, card.height - pad * 2.0);

        let areas: Vec<(String, Area)> =
            self.draft.iter().filter(|(_, s)| s.enabled).map(|(n, s)| (n.clone(), s.area())).collect();
        let Some(bounds) = Area::bounds(areas.iter().map(|(_, a)| *a)) else {
            return card.y + card.height;
        };
        let frame = self.drag.as_ref().map(|d| d.frame).unwrap_or_else(|| {
            let k = (inner.width / bounds.w.max(1) as f32).min(inner.height / bounds.h.max(1) as f32);
            Frame {
                base: (bounds.x as f32, bounds.y as f32),
                origin: (
                    inner.x + (inner.width - bounds.w as f32 * k) * 0.5,
                    inner.y + (inner.height - bounds.h as f32 * k) * 0.5,
                ),
                k,
            }
        });

        // Start, follow and end a drag.
        if !input.pressed {
            self.drag = None;
        }
        if clicked {
            let hit = areas.iter().rev().find(|(_, a)| layout::hovered(input, frame.to_screen(*a)));
            if let (Some((name, area)), Some(pointer)) = (hit, input.pointer) {
                self.selected = Some(name.clone());
                let (lx, ly) = frame.to_layout(pointer);
                self.drag = Some(Drag { name: name.clone(), grab: (lx - area.x as f32, ly - area.y as f32), frame });
            }
        }
        if let (Some(drag), Some(pointer)) = (&self.drag, input.pointer) {
            let (lx, ly) = drag.frame.to_layout(pointer);
            let wanted = ((lx - drag.grab.0).round() as i32, (ly - drag.grab.1).round() as i32);
            let snap = (m.display_snap / drag.frame.k).round() as i32;
            let others: Vec<Area> = areas.iter().filter(|(n, _)| *n != drag.name).map(|(_, a)| *a).collect();
            if let Some((_, s)) = self.draft.iter_mut().find(|(n, _)| *n == drag.name) {
                let (x, y) = displays::place(s.logical_size(), wanted, &others, snap);
                s.x = x;
                s.y = y;
            }
        }

        // The monitors, the selected one last so its outline is on top.
        let mut order: Vec<(String, Settings)> = self.draft.iter().filter(|(_, s)| s.enabled).cloned().collect();
        order.sort_by_key(|(n, _)| Some(n) == self.selected.as_ref());
        list.push_clip(card);
        for (name, s) in &order {
            let rect = Rect { radius: m.button_radius, ..frame.to_screen(s.area()) };
            let selected = Some(name) == self.selected.as_ref();
            let hot = layout::hovered(input, rect);
            let fill = if selected {
                layout::mix(theme.surface_highest, theme.primary, 0.22)
            } else if hot {
                layout::mix(theme.surface_highest, theme.text_main, 0.06)
            } else {
                theme.surface_highest
            };
            let (border, width) = if selected { (theme.primary, m.outline_active) } else { (theme.outline, m.outline) };
            ui::paint_frame(list, rect, width, border, fill);
            let label = layout::rect(rect.x + pad * 0.5, rect.y, (rect.width - pad).max(0.0), rect.height);
            layout::two_lines(ctx, list, label, name, &s.mode.size_label(), theme.text_muted);
        }
        list.pop_clip();
        card.y + card.height
    }

    /// The selected monitor's settings. Returns their bottom.
    #[allow(clippy::too_many_arguments)]
    fn paint_settings(
        &mut self,
        ctx: &mut PaintCtx<'_>,
        list: &mut DisplayList,
        body: Rect,
        y: f32,
        name: &str,
        monitors: &[displays::Monitor],
        input: &InputState,
        clicked: bool,
    ) -> f32 {
        let m = ctx.metrics.page.clone();
        let typo = ctx.metrics.typography.clone();
        let theme = ctx.theme.clone();
        let Some(monitor) = monitors.iter().find(|mon| mon.name == name) else { return y };
        let Some(index) = self.draft.iter().position(|(n, _)| n == name) else { return y };
        let mut s = self.draft[index].1;
        let others_on = self.draft.iter().filter(|(n, o)| n != name && o.enabled).count();

        let pad = m.card_padding;
        let gap = m.label_gap;
        let label_w = m.editor_label_width.min(body.width * 0.35);
        let value_x = body.x + pad + label_w;
        let value_w = body.x + body.width - pad - value_x;
        let row_h = m.badge;

        // The card is drawn first, at the height its rows will take.
        let sizes = monitor.sizes();
        let rates = monitor.rates(&s.mode);
        let scales = displays::scales_for(&s.mode, &ctx.displays.scales, s.scale);
        let size_labels: Vec<String> = sizes.iter().map(displays::Mode::size_label).collect();
        let rate_labels: Vec<String> = rates.iter().map(displays::Mode::refresh_label).collect();
        let rotation_labels: Vec<String> = ROTATIONS.iter().map(|r| r.to_string()).collect();
        let chip_gap = m.grid_gap * 0.5;
        let flow_h = |ctx: &mut PaintCtx<'_>, labels: &[String]| {
            let (mut x, mut rows) = (0.0, 1.0);
            for label in labels {
                let w = layout::text_button_width(ctx, label);
                if x > 0.0 && x + w > value_w {
                    x = 0.0;
                    rows += 1.0;
                }
                x += w + chip_gap;
            }
            rows * row_h + (rows - 1.0) * chip_gap
        };
        let title_h = typo.line(typo.body) + typo.line(typo.caption);
        let mut height = pad * 2.0 + title_h + gap + row_h;
        if s.enabled {
            for labels in [&rate_labels, &rotation_labels] {
                height += gap + flow_h(ctx, labels);
            }
            // The resolution dropdown and the scale slider.
            height += (gap + row_h) * 2.0;
            let switches = if monitor.vrr_capable { 3.0 } else { 2.0 };
            height += (gap + row_h) * switches;
        }
        let card = layout::rect(body.x, y, body.width, height);
        layout::card(ctx, list, card, layout::Card::Plain);

        let mut row_y = card.y + pad;
        let title = layout::rect(body.x + pad, row_y, body.width - pad * 2.0, title_h);
        let description = if monitor.description.is_empty() { "Unnamed monitor" } else { monitor.description.as_str() };
        layout::two_lines(ctx, list, title, name, description, theme.text_muted);
        row_y += title.height + gap;

        let label = |ctx: &mut PaintCtx<'_>, list: &mut DisplayList, text: &str, y: f32| {
            let row = layout::rect(body.x + pad, y, label_w, row_h);
            ui::paint_text_vcentered(ctx, list, text, row.x, row, typo.body, typo.weight_regular, theme.text_main);
        };
        let switch_y = |y: f32| y + (row_h - m.toggle_height) * 0.5;

        // On or off. The last monitor on cannot be turned off.
        label(ctx, list, "Enabled", row_y);
        if let Some(on) = controls::switch(ctx, list, value_x, switch_y(row_y), s.enabled, input, clicked) {
            if on || others_on > 0 {
                s.enabled = on;
                if on {
                    // Back beside the others, at the right.
                    let others = self.draft.iter().filter(|(n, o)| n != name && o.enabled).map(|(_, o)| o.area());
                    if let Some(b) = Area::bounds(others) {
                        s.x = b.x + b.w;
                        s.y = b.y;
                    }
                }
            }
        }
        if !s.enabled || others_on == 0 {
            let note = if s.enabled { "The only monitor on" } else { "Off" };
            let x = value_x + m.toggle_width + gap;
            let row = layout::rect(x, row_y, (value_w - m.toggle_width - gap).max(0.0), row_h);
            ui::paint_text_vcentered(ctx, list, note, x, row, typo.caption, typo.weight_regular, theme.text_muted);
        }
        row_y += row_h;

        if s.enabled {
            let mut resized = false;

            row_y += gap;
            // Resolution: a dropdown, whose list is painted last, over the page.
            label(ctx, list, "Resolution", row_y);
            let selected = sizes.iter().position(|size| size.same_size(&s.mode));
            let face_w = layout::dropdown_width(ctx, &size_labels, row_h).min(value_w);
            let face = layout::rect(value_x, row_y, face_w, row_h);
            let face_label = selected.map_or_else(|| s.mode.size_label(), |i| size_labels[i].clone());
            if layout::dropdown(ctx, list, face, &face_label, self.sizes_open, input, clicked) {
                self.sizes_open = !self.sizes_open;
                self.list_scroll = None;
            }
            self.sizes_menu = Some(SizesMenu {
                monitor: name.to_string(),
                anchor: face,
                sizes: sizes.clone(),
                labels: size_labels.clone(),
                selected,
            });
            row_y += row_h + gap;

            label(ctx, list, "Refresh rate", row_y);
            let selected = rates.iter().position(|r| r.same(&s.mode));
            let (bottom, hit) = layout::chip_flow(ctx, list, value_x, row_y, value_w, &rate_labels, selected, input, clicked);
            if let Some(i) = hit {
                s.mode = rates[i];
            }
            row_y = bottom + gap;

            // Scale: a slider that stops only at the scales this mode allows.
            label(ctx, list, "Scale", row_y);
            let index = scales.iter().position(|v| (v - s.scale).abs() < 0.005).unwrap_or(0);
            let readout_w = ctx.measure("1.667\u{d7}", typo.body) + gap * 2.0;
            let knob = m.slider_knob * 0.5;
            let slider = layout::rect(value_x + knob, row_y, (value_w - readout_w - knob * 2.0).max(0.0), row_h);
            // The knob reaches past the track's ends; so does the grab.
            let grab = Rect { x: slider.x - knob, width: slider.width + knob * 2.0, ..slider };
            if input.pressed && clicked && layout::hovered(input, grab) {
                self.scale_drag = true;
            } else if !input.pressed {
                self.scale_drag = false;
            }
            if let Some(i) = controls::stepped_slider(ctx, list, slider, index, scales.len(), self.scale_drag, input) {
                if let Some(scale) = scales.get(i).filter(|v| (**v - s.scale).abs() >= 0.005) {
                    s.scale = *scale;
                    resized = true;
                }
            }
            let readout = format!("{}\u{d7}", displays::format_scale(s.scale));
            let readout_x = value_x + value_w - ctx.measure(&readout, typo.body);
            let row = layout::rect(readout_x, row_y, value_x + value_w - readout_x, row_h);
            ui::paint_text_vcentered(ctx, list, &readout, readout_x, row, typo.body, typo.weight_regular, theme.text_main);
            row_y += row_h + gap;

            label(ctx, list, "Rotation", row_y);
            let flipped = s.transform >= 4;
            let rotation = (s.transform % 4) as usize;
            let (bottom, hit) =
                layout::chip_flow(ctx, list, value_x, row_y, value_w, &rotation_labels, Some(rotation), input, clicked);
            if let Some(i) = hit {
                s.transform = i as u8 + if flipped { 4 } else { 0 };
                resized = true;
            }
            row_y = bottom + gap;

            label(ctx, list, "Flipped", row_y);
            if let Some(on) = controls::switch(ctx, list, value_x, switch_y(row_y), flipped, input, clicked) {
                s.transform = s.transform % 4 + if on { 4 } else { 0 };
            }
            row_y += row_h;

            if monitor.vrr_capable {
                row_y += gap;
                label(ctx, list, "Variable refresh", row_y);
                if let Some(on) = controls::switch(ctx, list, value_x, switch_y(row_y), s.vrr, input, clicked) {
                    s.vrr = on;
                }
                row_y += row_h;
            }

            // A new size can leave it overlapping, or apart from, the others.
            if resized {
                let others: Vec<Area> =
                    self.draft.iter().filter(|(n, o)| n != name && o.enabled).map(|(_, o)| o.area()).collect();
                let (x, y) = displays::place(s.logical_size(), (s.x, s.y), &others, 0);
                s.x = x;
                s.y = y;
            }

            row_y += gap;
            label(ctx, list, "Position", row_y);
            let (w, h) = s.logical_size();
            let text = format!("x {}, y {} \u{b7} {w}\u{d7}{h} logical", s.x, s.y);
            let row = layout::rect(value_x, row_y, value_w, row_h);
            ui::paint_text_vcentered(ctx, list, &text, value_x, row, typo.body, typo.weight_regular, theme.text_muted);
        }

        self.draft[index].1 = s;
        card.y + card.height
    }
}

/// The prompt to keep a layout on trial, as a modal over the whole settings
/// window: the screen just changed, and the old layout comes back unless
/// this is answered. The window paints it over any page, in a layer of its
/// own, and keeps the pointer from everything under it.
#[allow(clippy::too_many_arguments)]
pub fn paint_trial_modal(
    ctx: &mut PaintCtx<'_>,
    list: &mut DisplayList,
    window: Rect,
    until: std::time::Instant,
    input: &InputState,
    clicked: bool,
    pending: &mut Vec<SystemAction>,
) {
    let m = ctx.metrics.page.clone();
    let typo = ctx.metrics.typography.clone();
    let theme = ctx.theme.clone();
    ui::paint_scrim(list, window, m.modal_scrim);

    let pad = m.card_padding * 1.5;
    let gap = m.label_gap;
    let title_h = typo.line(m.summary.max(typo.body)) + typo.line(typo.caption);
    let width = m.modal_width.min(window.width - pad * 2.0).max(0.0);
    let height = pad * 2.0 + m.badge + gap + title_h + gap + m.meter + gap * 2.0 + m.badge;
    let card = Rect {
        radius: m.card_radius,
        ..layout::rect(
            window.x + (window.width - width) * 0.5,
            window.y + (window.height - height) * 0.5,
            width,
            height,
        )
    };
    ui::paint_frame(list, card, m.outline_active, theme.primary, theme.surface_high);

    let x = card.x + pad;
    let inner_w = card.width - pad * 2.0;
    let mut y = card.y + pad;
    let tint = layout::mix(theme.surface_high, theme.primary, 0.22);
    layout::badge(ctx, list, x, y, "\u{f108}", theme.primary, tint);
    y += m.badge + gap;

    let left = until.saturating_duration_since(std::time::Instant::now());
    let seconds = left.as_secs_f32().ceil() as u32;
    let unit = if seconds == 1 { "second" } else { "seconds" };
    let caption = format!("The previous layout comes back in {seconds} {unit}");
    layout::two_lines(ctx, list, layout::rect(x, y, inner_w, title_h), "Keep these display settings?", &caption, theme.text_muted);
    y += title_h + gap;

    // How much of the wait is left.
    let total = ctx.displays.confirm_s.max(1) as f32;
    let fraction = (left.as_secs_f32() / total).clamp(0.0, 1.0);
    layout::meter(ctx, list, layout::rect(x, y, inner_w, m.meter), fraction, theme.primary);
    y += m.meter + gap * 2.0;

    let keep_w = layout::text_button_width(ctx, "Keep changes");
    let revert_w = layout::text_button_width(ctx, "Revert");
    let keep_x = card.x + card.width - pad - keep_w;
    let revert_x = keep_x - m.grid_gap * 0.5 - revert_w;
    if layout::text_button(ctx, list, revert_x, y, "Revert", true, input, clicked) {
        pending.push(SystemAction::Display(Command::Revert));
    }
    if layout::chip(ctx, list, keep_x, y, "Keep changes", true, input, clicked) {
        pending.push(SystemAction::Display(Command::Keep));
    }
}
