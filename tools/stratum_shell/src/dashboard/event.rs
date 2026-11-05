//! An event's detail view, opened by clicking it in the agenda or day panel:
//! its title and time, then what its source says about it (location,
//! calendar, organizer, attendees, its own page, and the description) in a
//! card over the tab's panels that scrolls with the wheel or ↑ ↓.
//!
//! Descriptions arrive as plain text or HTML (Google writes links as
//! `<a>`, Outlook's `X-ALT-DESC` is a whole document); both become
//! [`Span`]s of text and links. A link is opened with `xdg-open`, and only
//! `http`, `https` and `mailto` ones are links at all: a feed is someone
//! else's text, and `file:` or a desktop entry must not be one click away.

use chrono::{NaiveDate, NaiveTime};
use stratum_sys::managers::calendar::Occurrence;

use super::agenda::time_label;
use super::{calendar_color, Button, Dashboard};
use crate::render::display_list::{Color, DisplayList, Rect};
use crate::ui::{self, PaintCtx};

/// The shown event, what its view lays out, and where it has scrolled to.
pub struct EventDetail {
    occurrence: Occurrence,
    /// A glyph, its calendar's colour or the accent, and the text beside it.
    rows: Vec<(&'static str, bool, Vec<Span>)>,
    description: Vec<Span>,
    pub(super) scroll: f32,
    /// From the last paint: how far it can scroll, how tall the scrolled
    /// area is, the whole view, and the links in it.
    pub(super) max_scroll: f32,
    viewport: f32,
    pub rect: Option<Rect>,
    pub(super) links: Vec<String>,
}

impl EventDetail {
    pub fn new(occurrence: Occurrence) -> Self {
        let details = std::sync::Arc::clone(&occurrence.details);
        let mut rows = Vec::new();
        if !occurrence.location.trim().is_empty() {
            rows.push(("\u{f034e}", false, linkify(occurrence.location.trim())));
        }
        let calendar = match details.calendar_name.as_str() {
            "" => format!("Calendar {}", occurrence.calendar + 1),
            name => name.to_string(),
        };
        rows.push(("\u{f00ed}", true, vec![Span::plain(calendar)]));
        if !details.organizer.is_empty() {
            rows.push(("\u{f0004}", false, vec![Span::plain(format!("Organised by {}", details.organizer))]));
        }
        if !details.attendees.is_empty() {
            rows.push(("\u{f000e}", false, vec![Span::plain(details.attendees.join(", "))]));
        }
        if !details.url.is_empty() {
            let url = details.url.clone();
            let link = openable(&url).then(|| url.clone());
            rows.push(("\u{f0337}", false, vec![Span { text: url, link }]));
        }
        Self {
            description: description(&details.description),
            occurrence,
            rows,
            scroll: 0.0,
            max_scroll: 0.0,
            viewport: 0.0,
            rect: None,
            links: Vec::new(),
        }
    }

    /// Scroll by `delta`, within what the last paint found; whether it
    /// moved.
    pub fn scroll_by(&mut self, delta: f32) -> bool {
        let next = (self.scroll + delta).clamp(0.0, self.max_scroll);
        let moved = next != self.scroll;
        self.scroll = next;
        moved
    }

    /// Most of the scrolled area: what Page Down moves.
    pub fn page(&self) -> f32 {
        self.viewport * 0.9
    }

    pub fn link(&self, index: usize) -> Option<String> {
        self.links.get(index).cloned()
    }
}

/// A block of the view: its glyph and that glyph's colour (none for the
/// description), its laid-out lines, and their colour.
type Block<'a> = (Option<(&'a str, Color)>, Vec<Vec<Run>>, Color);

impl Dashboard {
    /// The shown event's details, filling `area` (the tab's panels).
    pub(super) fn paint_detail(&mut self, ctx: &mut PaintCtx<'_>, list: &mut DisplayList, area: Rect) {
        let Some(mut detail) = self.detail.take() else { return };
        let m = self.metrics.clone();
        let typography = &ctx.metrics.typography;
        let (regular, bold) = (typography.weight_regular, typography.weight_bold);
        ui::paint_frame(list, area, m.border, ctx.theme.border_active, ctx.theme.bg_widget);
        detail.rect = Some(area);
        let x = area.x + m.inner;
        let w = area.width - m.inner * 2.0;
        let accent = calendar_color(ctx, detail.occurrence.calendar);

        let close = Rect { x: x + w - m.button, y: area.y + m.inner, width: m.button, height: m.button, radius: m.radius * 0.6, color: Color::TRANSPARENT };
        ui::paint_frame(list, close, m.border, ctx.theme.border_inactive, ctx.theme.bg_main);
        ui::paint_text_centered(ctx, list, "\u{f0156}", close, m.text, ctx.theme.text_main);
        self.buttons.push((close, Button::CloseDetail));

        // The title, beside a stripe of its calendar's colour, then when.
        let tx = x + m.stripe + m.inner * 0.6;
        let title_w = close.x - m.inner - tx;
        let title_h = m.line * 1.2;
        let top = area.y + m.inner;
        let mut y = top;
        for line in ui::wrap_text(ctx, &detail.occurrence.summary, m.title, bold, title_w, 3) {
            let row = Rect { x: tx, y, width: title_w, height: title_h, radius: 0.0, color: Color::TRANSPARENT };
            ui::paint_text_vcentered(ctx, list, &line, tx, row, m.title, bold, ctx.theme.text_main);
            y += title_h;
        }
        let row = Rect { x: tx, y, width: title_w, height: m.line, radius: 0.0, color: Color::TRANSPARENT };
        ui::paint_text_vcentered(ctx, list, &when_label(&detail.occurrence), tx, row, m.caption, regular, ctx.theme.text_muted);
        y += m.line;
        list.push_rect(Rect { x, y: top + m.line * 0.15, width: m.stripe, height: y - top - m.line * 0.3, radius: m.stripe * 0.5, color: accent });
        y += m.inner * 0.5;
        list.push_rect(Rect { x, y, width: w, height: m.border, radius: 0.0, color: ctx.theme.border_inactive });
        y += m.border + m.inner * 0.5;

        // The rest scrolls, beside a bar while it overflows.
        let viewport = Rect { x, y, width: w, height: (area.y + area.height - m.inner - y).max(0.0), radius: 0.0, color: Color::TRANSPARENT };
        let text_w = w - m.scrollbar - m.inner * 0.5;
        let glyph_w = m.text * 1.6;
        let style = crate::text::TextStyle {
            family: std::sync::Arc::from(ctx.theme.font.as_str()),
            size: m.text,
            weight: regular,
            color: ctx.theme.text_main,
        };
        detail.links.clear();
        let mut blocks: Vec<Block> = Vec::new();
        for (glyph, calendar, spans) in &detail.rows {
            let lines = layout(spans, text_w - glyph_w, &mut detail.links, |s| ctx.text.measure(s, &style).0);
            let color = if *calendar { accent } else { ctx.theme.primary };
            blocks.push((Some((glyph, color)), lines, ctx.theme.text_main));
        }
        if detail.description.is_empty() {
            blocks.push((None, vec![vec![Run { text: "No description".into(), x: 0.0, width: 0.0, link: None }]], ctx.theme.text_muted));
        } else {
            let lines = layout(&detail.description, text_w, &mut detail.links, |s| ctx.text.measure(s, &style).0);
            blocks.push((None, lines, ctx.theme.text_main));
        }
        let gap = m.inner * 0.5;
        // Every block is lines, and the description (last) has a gap above.
        let content = blocks.iter().map(|(_, lines, _)| lines.len().max(1) as f32 * m.line).sum::<f32>() + gap;
        detail.viewport = viewport.height;
        detail.max_scroll = (content - viewport.height).max(0.0);
        detail.scroll = detail.scroll.clamp(0.0, detail.max_scroll);

        let (ascent, line_height) = ctx.text.metrics(&style);
        let bottom = viewport.y + viewport.height;
        list.push_clip(viewport);
        let mut y = viewport.y - detail.scroll;
        for (i, (glyph, lines, color)) in blocks.iter().enumerate() {
            // The description stands apart from the rows above it.
            if glyph.is_none() && i > 0 {
                y += gap;
            }
            let indent = if glyph.is_some() { glyph_w } else { 0.0 };
            if let Some((glyph, glyph_color)) = glyph {
                let row = Rect { x, y, width: glyph_w, height: m.line, radius: 0.0, color: Color::TRANSPARENT };
                ui::paint_text_vcentered(ctx, list, glyph, x, row, m.text, regular, *glyph_color);
            }
            for line in lines {
                if y + m.line > viewport.y && y < bottom {
                    let row = Rect { x, y, width: text_w, height: m.line, radius: 0.0, color: Color::TRANSPARENT };
                    for run in line {
                        let rx = x + indent + run.x;
                        let run_color = if run.link.is_some() { ctx.theme.primary } else { *color };
                        ui::paint_text_vcentered(ctx, list, &run.text, rx, row, m.text, regular, run_color);
                        let Some(link) = run.link else { continue };
                        let underline = y + (m.line - line_height) * 0.5 + ascent + m.border;
                        list.push_rect(Rect { x: rx, y: underline, width: run.width, height: m.border, radius: 0.0, color: ctx.theme.primary });
                        // Clickable where it shows.
                        let (top, end) = (y.max(viewport.y), (y + m.line).min(bottom));
                        self.buttons.push((Rect { x: rx, y: top, width: run.width, height: end - top, radius: 0.0, color: Color::TRANSPARENT }, Button::Link(link)));
                    }
                }
                y += m.line;
            }
            if lines.is_empty() {
                y += m.line;
            }
        }
        list.pop_clip();
        if detail.max_scroll > 0.0 && viewport.height > 0.0 {
            let thumb = viewport.height * viewport.height / content;
            let ty = viewport.y + (viewport.height - thumb) * detail.scroll / detail.max_scroll;
            list.push_rect(Rect { x: x + w - m.scrollbar, y: ty, width: m.scrollbar, height: thumb, radius: m.scrollbar * 0.5, color: ctx.theme.border_inactive });
        }
        self.detail = Some(detail);
    }
}

/// Text, or a link's text and where it goes.
#[derive(Clone, Debug, PartialEq)]
pub struct Span {
    pub text: String,
    pub link: Option<String>,
}

impl Span {
    pub fn plain(text: impl Into<String>) -> Self {
        Self { text: text.into(), link: None }
    }
}

/// Whether a click may hand `url` to `xdg-open`.
pub fn openable(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    let scheme = ["https://", "http://", "mailto:"].iter().find(|s| lower.starts_with(**s));
    scheme.is_some_and(|s| url.len() > s.len()) && !url.chars().any(|c| c.is_whitespace() || c.is_control())
}

/// "Thursday 5 November · 3:00 PM – 4:00 PM", and the like for all-day and
/// overnight events.
pub fn when_label(o: &Occurrence) -> String {
    let long = |d: NaiveDate| d.format("%A %-d %B").to_string();
    let short = |d: NaiveDate| d.format("%a %-d %b").to_string();
    let day = o.start.date();
    if o.all_day {
        // `end` is the midnight after the last day.
        let last = (o.end - chrono::Duration::days(1)).date().max(day);
        return if last == day {
            format!("{} \u{b7} All day", long(day))
        } else {
            format!("{} \u{2013} {} \u{b7} All day", short(day), short(last))
        };
    }
    if o.end <= o.start {
        return format!("{} \u{b7} {}", long(day), time_label(o.start));
    }
    let ends_today = o.end.date() == day || (o.end.time() == NaiveTime::MIN && o.end.date() == day.succ_opt().unwrap_or(day));
    if ends_today {
        format!("{} \u{b7} {} \u{2013} {}", long(day), time_label(o.start), time_label(o.end))
    } else {
        format!("{} {} \u{2013} {} {}", short(day), time_label(o.start), short(o.end.date()), time_label(o.end))
    }
}

/// A description as spans: HTML read as HTML, anything else as plain text
/// with its line breaks; addresses in either become links.
pub fn description(text: &str) -> Vec<Span> {
    if looks_like_html(text) {
        html(text)
    } else {
        linkify(&tidy_lines(text))
    }
}

/// Plain text with trailing spaces, runs of blank lines and the blank
/// lines around it removed.
fn tidy_lines(text: &str) -> String {
    let mut out = String::new();
    let mut blank = false;
    for line in text.lines().map(str::trim_end) {
        if line.trim().is_empty() {
            blank = !out.is_empty();
            continue;
        }
        if !out.is_empty() {
            out.push_str(if blank { "\n\n" } else { "\n" });
        }
        out.push_str(line);
        blank = false;
    }
    out
}

fn looks_like_html(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    const TAGS: [&str; 18] = [
        "<br", "<p>", "<p ", "</p>", "<a ", "</a>", "<div", "<span", "<b>", "<i>", "<u>", "<ul", "<ol", "<li", "<html",
        "<table", "<strong", "<em>",
    ];
    TAGS.iter().any(|t| lower.contains(t))
}

/// `text` with every `http(s)://` address made a link. An address ends at
/// whitespace, `<`, `>` or `"`, less trailing punctuation and an unmatched
/// closing bracket.
pub fn linkify(text: &str) -> Vec<Span> {
    let lower = text.to_ascii_lowercase();
    let mut spans = Vec::new();
    let (mut emitted, mut search) = (0, 0);
    while let Some(start) = ["https://", "http://"].iter().filter_map(|p| lower[search..].find(p)).min().map(|i| search + i) {
        let len = text[start..].find(|c: char| c.is_whitespace() || matches!(c, '<' | '>' | '"')).unwrap_or(text.len() - start);
        let url = trim_url(&text[start..start + len]);
        if !openable(url) {
            search = start + len.max(1);
            continue;
        }
        push(&mut spans, &text[emitted..start], None);
        push(&mut spans, url, Some(url));
        emitted = start + url.len();
        search = emitted;
    }
    push(&mut spans, &text[emitted..], None);
    spans
}

fn trim_url(mut url: &str) -> &str {
    loop {
        let Some(last) = url.chars().last() else { return url };
        let unmatched = |open: char, close: char| last == close && url.matches(open).count() < url.matches(close).count();
        if matches!(last, '.' | ',' | ';' | ':' | '!' | '?' | '\'' | '*') || unmatched('(', ')') || unmatched('[', ']') {
            url = &url[..url.len() - last.len_utf8()];
        } else {
            return url;
        }
    }
}

/// Append `text` to `spans`, joining the last span when it goes to the same
/// place.
fn push(spans: &mut Vec<Span>, text: &str, link: Option<&str>) {
    if text.is_empty() {
        return;
    }
    match spans.last_mut() {
        Some(last) if last.link.as_deref() == link => last.text.push_str(text),
        _ => spans.push(Span { text: text.to_string(), link: link.map(str::to_string) }),
    }
}

/// HTML as spans: line breaks for `<br>` and block elements, `<a href>`
/// as links, entities decoded, whitespace collapsed, and `<head>`,
/// `<style>` and `<script>` left out.
fn html(text: &str) -> Vec<Span> {
    let mut out = HtmlText::default();
    let lower = text.to_ascii_lowercase();
    let mut i = 0;
    while i < text.len() {
        let rest = &text[i..];
        if rest.starts_with("<!--") {
            i += rest.find("-->").map_or(rest.len(), |e| e + 3);
            continue;
        }
        if let Some((tag, closing, len)) = tag_at(rest) {
            let name = tag.to_ascii_lowercase();
            i += len;
            match (name.as_str(), closing) {
                ("head" | "style" | "script" | "title", false) => {
                    let end = format!("</{name}");
                    i = lower[i..].find(&end).map_or(text.len(), |e| i + e);
                    i += text[i..].find('>').map_or(text.len() - i, |e| e + 1);
                }
                ("br", _) => out.line_break(1),
                ("li", false) => {
                    out.line_break(1);
                    out.text("\u{2022} ");
                }
                ("p" | "div" | "tr" | "ul" | "ol" | "li" | "table" | "blockquote" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6", _) => {
                    out.line_break(if matches!(name.as_str(), "p" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6") { 2 } else { 1 })
                }
                ("a", false) => out.open_link(attribute(&rest[..len], "href").map(|h| decode_entities(&h))),
                ("a", true) => out.close_link(),
                _ => {}
            }
            continue;
        }
        // Up to the next `<`, past this one if it began no tag.
        let first = rest.chars().next().map_or(1, char::len_utf8);
        let next = rest[first..].find('<').map_or(rest.len(), |n| n + first);
        out.text(&decode_entities(&rest[..next]));
        i += next;
    }
    out.finish()
}

/// A tag at the start of `text`: its name, whether it closes, and its
/// length. `<` followed by anything but a name (`<https://…>`, `a < b`)
/// is text.
fn tag_at(text: &str) -> Option<(&str, bool, usize)> {
    let inner = text.strip_prefix('<')?;
    let (closing, inner) = match inner.strip_prefix('/') {
        Some(rest) => (true, rest),
        None => (false, inner.strip_prefix('!').unwrap_or(inner)),
    };
    let name_len = inner.find(|c: char| !c.is_ascii_alphanumeric()).unwrap_or(inner.len());
    let after = inner[name_len..].chars().next()?;
    if name_len == 0 || !inner.starts_with(|c: char| c.is_ascii_alphabetic()) || !(after.is_whitespace() || after == '>' || after == '/') {
        return None;
    }
    let end = text.find('>')?;
    Some((&inner[..name_len], closing, end + 1))
}

/// An attribute's value in a tag, quoted or not.
fn attribute(tag: &str, name: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let mut from = 0;
    while let Some(at) = lower[from..].find(name).map(|i| from + i) {
        from = at + name.len();
        let before = lower[..at].chars().last();
        let rest = lower[from..].trim_start();
        if !before.is_some_and(char::is_whitespace) || !rest.starts_with('=') {
            continue;
        }
        let value_at = from + (lower[from..].len() - rest.len()) + 1;
        let value = tag[value_at..].trim_start();
        return Some(match value.chars().next()? {
            quote @ ('"' | '\'') => value[1..].split(quote).next()?.to_string(),
            _ => value.split(|c: char| c.is_whitespace() || c == '>').next()?.to_string(),
        });
    }
    None
}

fn decode_entities(text: &str) -> String {
    if !text.contains('&') {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        rest = &rest[at..];
        let entity = rest[1..].find(';').filter(|&e| e <= 10).map(|e| &rest[1..=e]);
        let decoded = entity.and_then(|e| match e {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "nbsp" => Some(' '),
            _ => {
                let number = e.strip_prefix('#')?;
                let code = match number.strip_prefix(['x', 'X']) {
                    Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                    None => number.parse().ok()?,
                };
                char::from_u32(code)
            }
        });
        match (entity, decoded) {
            (Some(e), Some(c)) => {
                out.push(c);
                rest = &rest[e.len() + 2..];
            }
            _ => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// HTML's text as it is read: whitespace collapsed, breaks kept, at most
/// one blank line in a row.
#[derive(Default)]
struct HtmlText {
    spans: Vec<Span>,
    link: Option<String>,
    /// The link's text so far: a link without any shows its address.
    link_text: bool,
    space: bool,
    breaks: usize,
}

impl HtmlText {
    fn text(&mut self, text: &str) {
        for c in text.chars() {
            if c.is_whitespace() {
                self.space = true;
                continue;
            }
            if !self.spans.is_empty() {
                if self.breaks > 0 {
                    push(&mut self.spans, &"\n".repeat(self.breaks.min(2)), None);
                } else if self.space {
                    push(&mut self.spans, " ", None);
                }
            }
            (self.breaks, self.space) = (0, false);
            let mut buf = [0; 4];
            push(&mut self.spans, c.encode_utf8(&mut buf), self.link.as_deref());
            self.link_text |= self.link.is_some();
        }
    }

    fn line_break(&mut self, lines: usize) {
        self.breaks = self.breaks.max(lines);
        self.space = false;
    }

    fn open_link(&mut self, href: Option<String>) {
        self.close_link();
        self.link = href.filter(|h| openable(h));
    }

    fn close_link(&mut self) {
        if let Some(href) = self.link.clone() {
            if !self.link_text {
                self.text(&href);
            }
        }
        (self.link, self.link_text) = (None, false);
    }

    /// The spans, with addresses in the plain text made links too.
    fn finish(mut self) -> Vec<Span> {
        self.close_link();
        let mut out = Vec::new();
        for span in self.spans {
            match span.link {
                Some(link) => push(&mut out, &span.text, Some(&link)),
                None => {
                    for part in linkify(&span.text) {
                        push(&mut out, &part.text, part.link.as_deref());
                    }
                }
            }
        }
        out
    }
}

/// Words and line breaks in spans, as laid out: a word knows whether a
/// space comes before it, so a link's closing bracket stays on its line.
#[derive(Clone, Debug, PartialEq)]
struct Fragment {
    text: String,
    link: Option<usize>,
    space_before: bool,
    /// Line breaks before it.
    breaks: usize,
}

fn fragments(spans: &[Span], links: &mut Vec<String>) -> Vec<Fragment> {
    let mut out = Vec::new();
    let (mut breaks, mut space) = (0, false);
    let mut flush = |word: &mut String, link: Option<usize>, breaks: &mut usize, space: &mut bool| {
        if !word.is_empty() {
            out.push(Fragment { text: std::mem::take(word), link, space_before: *space && *breaks == 0, breaks: *breaks });
            (*breaks, *space) = (0, false);
        }
    };
    for span in spans {
        let link = span.link.as_ref().filter(|l| openable(l)).map(|l| match links.iter().position(|k| k == l) {
            Some(i) => i,
            None => {
                links.push(l.clone());
                links.len() - 1
            }
        });
        let mut word = String::new();
        for c in span.text.chars() {
            if c == '\n' {
                flush(&mut word, link, &mut breaks, &mut space);
                breaks += 1;
            } else if c.is_whitespace() {
                flush(&mut word, link, &mut breaks, &mut space);
                space = true;
            } else {
                word.push(c);
            }
        }
        flush(&mut word, link, &mut breaks, &mut space);
    }
    out
}

/// A piece of a laid-out line: its text, where it starts and how wide it
/// is, and which of the view's links it is part of.
#[derive(Clone, Debug, PartialEq)]
pub struct Run {
    pub text: String,
    pub x: f32,
    pub width: f32,
    pub link: Option<usize>,
}

/// `spans` wrapped to `width`: lines of runs. Each link's address is added
/// to `links` once, and runs refer to it by index. A word wider than a
/// line is broken between characters (long addresses are).
pub fn layout(spans: &[Span], width: f32, links: &mut Vec<String>, mut measure: impl FnMut(&str) -> f32) -> Vec<Vec<Run>> {
    let space = measure(" ");
    let mut lines: Vec<Vec<Run>> = Vec::new();
    let mut line: Vec<Run> = Vec::new();
    let mut x = 0.0f32;
    for fragment in fragments(spans, links) {
        for _ in 0..fragment.breaks {
            lines.push(std::mem::take(&mut line));
            x = 0.0;
        }
        let mut gap = if fragment.space_before && !line.is_empty() { space } else { 0.0 };
        let mut pieces = vec![(fragment.text.clone(), measure(&fragment.text))];
        if pieces[0].1 > width {
            pieces = break_word(&fragment.text, width, &mut measure);
        }
        for (i, (text, w)) in pieces.into_iter().enumerate() {
            if !line.is_empty() && (i > 0 || x + gap + w > width) {
                lines.push(std::mem::take(&mut line));
                (x, gap) = (0.0, 0.0);
            }
            match line.last_mut() {
                // The same link, or plain text, after a space: one run.
                Some(last) if last.link == fragment.link && (gap > 0.0 || last.x + last.width == x) => {
                    if gap > 0.0 {
                        last.text.push(' ');
                    }
                    last.text.push_str(&text);
                    last.width = x + gap + w - last.x;
                }
                _ => line.push(Run { text, x: x + gap, width: w, link: fragment.link }),
            }
            x += gap + w;
            gap = 0.0;
        }
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

/// A word too wide for a line, in pieces that fit (at least a character
/// each), with their widths.
fn break_word(word: &str, width: f32, measure: &mut impl FnMut(&str) -> f32) -> Vec<(String, f32)> {
    let mut pieces = Vec::new();
    let mut piece = String::new();
    let mut piece_w = 0.0;
    for c in word.chars() {
        let mut buf = [0; 4];
        let w = measure(c.encode_utf8(&mut buf));
        if !piece.is_empty() && piece_w + w > width {
            pieces.push((std::mem::take(&mut piece), piece_w));
            piece_w = 0.0;
        }
        piece.push(c);
        piece_w += w;
    }
    if !piece.is_empty() {
        pieces.push((piece, piece_w));
    }
    pieces
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDateTime;

    fn link(text: &str, to: &str) -> Span {
        Span { text: text.into(), link: Some(to.into()) }
    }

    #[test]
    fn plain_addresses_become_links_without_their_punctuation() {
        let spans = linkify("Join (https://meet.example/abc), or see https://w.example/Foo_(bar).");
        assert_eq!(
            spans,
            [
                Span::plain("Join ("),
                link("https://meet.example/abc", "https://meet.example/abc"),
                Span::plain("), or see "),
                link("https://w.example/Foo_(bar)", "https://w.example/Foo_(bar)"),
                Span::plain("."),
            ]
        );
        assert_eq!(linkify("<https://teams.example/x>")[1], link("https://teams.example/x", "https://teams.example/x"));
        assert_eq!(linkify("just http:// alone"), [Span::plain("just http:// alone")]);
        assert_eq!(description("caf\u{e9} <b>d\u{e9}j\u{e0}</b> \u{2192} vu"), [Span::plain("caf\u{e9} d\u{e9}j\u{e0} \u{2192} vu")]);
    }

    #[test]
    fn only_web_and_mail_addresses_open() {
        assert!(openable("https://example.com/x") && openable("HTTP://example.com") && openable("mailto:a@b.c"));
        assert!(!openable("file:///etc/passwd") && !openable("javascript:alert(1)") && !openable("https://"));
        assert!(!openable("https://example.com/a b"));
    }

    #[test]
    fn plain_descriptions_keep_lines_and_at_most_one_blank() {
        let spans = description("\n\nAgenda:  \n- one\n\n\n\n- two\n\n");
        assert_eq!(spans, [Span::plain("Agenda:\n- one\n\n- two")]);
    }

    #[test]
    fn html_descriptions_read_as_text_and_links() {
        let text = "<p>Join with Google Meet: <a href=\"https://meet.google.com/abc-defg\">meet.google.com/abc-defg</a></p>\
                    <p>Notes &amp; agenda:<br>\n  - first&nbsp;item<br/>see https://docs.example/d</p>";
        assert_eq!(
            description(text),
            [
                Span::plain("Join with Google Meet: "),
                link("meet.google.com/abc-defg", "https://meet.google.com/abc-defg"),
                Span::plain("\n\nNotes & agenda:\n- first item\nsee "),
                link("https://docs.example/d", "https://docs.example/d"),
            ]
        );
    }

    #[test]
    fn outlook_documents_lose_their_head_and_unsafe_links() {
        let text = "<html><head><meta name=x><style>p {color:red}</style></head><body><!-- c --><div>Hi <a href='file:///x'>there</a></div>\
                    <ul><li>One</li><li>Two</li></ul><a href=\"mailto:a@b.c\"></a></body></html>";
        assert_eq!(description(text), [Span::plain("Hi there\n\u{2022} One\n\u{2022} Two\n"), link("mailto:a@b.c", "mailto:a@b.c")]);
    }

    #[test]
    fn entities_decode_and_stray_ampersands_stay() {
        assert_eq!(decode_entities("a &lt;b&gt; &#39;c&#x27; & d &bogus; e"), "a <b> 'c' & d &bogus; e");
        assert_eq!(attribute("<a class=x href = 'https://e.x/?a=1&b=2'>", "href").as_deref(), Some("https://e.x/?a=1&b=2"));
        assert_eq!(attribute("<a data-href=\"x\" href=https://e.x>", "href").as_deref(), Some("https://e.x"));
    }

    /// Ten pixels per character.
    fn mono(s: &str) -> f32 {
        s.chars().count() as f32 * 10.0
    }

    fn texts(lines: &[Vec<Run>]) -> Vec<Vec<&str>> {
        lines.iter().map(|l| l.iter().map(|r| r.text.as_str()).collect()).collect()
    }

    #[test]
    fn layout_wraps_words_and_keeps_links_apart() {
        let spans = [Span::plain("Join the call ("), link("https://m.ex/abc", "https://m.ex/abc"), Span::plain(") now\n\nBye")];
        let mut links = Vec::new();
        let lines = layout(&spans, 150.0, &mut links, mono);
        assert_eq!(texts(&lines), [vec!["Join the call ("], vec!["https://m.ex/ab"], vec!["c", ") now"], vec![], vec!["Bye"]]);
        assert_eq!(links, ["https://m.ex/abc"]);
        // Both pieces of the broken address are the same link.
        assert_eq!((lines[1][0].link, lines[2][0].link), (Some(0), Some(0)));
        assert_eq!((lines[2][1].x, lines[2][1].link), (10.0, None), "the bracket is glued to the link");
    }

    #[test]
    fn a_link_seen_twice_is_listed_once() {
        let spans = [link("a", "https://x.ex"), Span::plain(" "), link("b", "https://x.ex"), Span::plain(" "), link("c", "file:///x")];
        let mut links = Vec::new();
        let lines = layout(&spans, 500.0, &mut links, mono);
        assert_eq!(links, ["https://x.ex"]);
        assert_eq!(lines[0].iter().map(|r| r.link).collect::<Vec<_>>(), [Some(0), None]);
    }

    fn at(d: u32, h: u32, min: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, 11, d).unwrap().and_hms_opt(h, min, 0).unwrap()
    }

    fn occurrence(start: NaiveDateTime, end: NaiveDateTime, all_day: bool) -> Occurrence {
        Occurrence { summary: String::new(), location: String::new(), start, end, all_day, calendar: 0, details: Default::default() }
    }

    #[test]
    fn times_read_for_one_day_several_and_overnight() {
        assert_eq!(when_label(&occurrence(at(5, 15, 0), at(5, 16, 0), false)), "Thursday 5 November \u{b7} 3:00 PM \u{2013} 4:00 PM");
        assert_eq!(when_label(&occurrence(at(5, 23, 0), at(6, 0, 0), false)), "Thursday 5 November \u{b7} 11:00 PM \u{2013} 12:00 AM");
        assert_eq!(when_label(&occurrence(at(5, 22, 0), at(6, 2, 0), false)), "Thu 5 Nov 10:00 PM \u{2013} Fri 6 Nov 2:00 AM");
        assert_eq!(when_label(&occurrence(at(5, 9, 0), at(5, 9, 0), false)), "Thursday 5 November \u{b7} 9:00 AM");
        assert_eq!(when_label(&occurrence(at(5, 0, 0), at(6, 0, 0), true)), "Thursday 5 November \u{b7} All day");
        assert_eq!(when_label(&occurrence(at(5, 0, 0), at(8, 0, 0), true)), "Thu 5 Nov \u{2013} Sat 7 Nov \u{b7} All day");
    }
}
