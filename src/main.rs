//! 0xMark: paste an image from the clipboard, annotate it, copy it back.

use std::cell::{Cell, RefCell};
use std::f64::consts::{PI, TAU};
use std::io::Write;
use std::process::{Command, Stdio};
use std::rc::Rc;

use adw::prelude::*;
use gtk::{cairo, gdk, gio, glib};

const APP_ID: &str = "dev.abidibo.OxMark";

// ---------------------------------------------------------------------------
// Model
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Tool {
    Rect,
    Ellipse,
    Arrow,
    Pen,
    Text,
}

#[derive(Clone, Copy, Debug)]
struct Rgb {
    r: f64,
    g: f64,
    b: f64,
}

impl Rgb {
    const fn new(r: u8, g: u8, b: u8) -> Self {
        Self {
            r: r as f64 / 255.0,
            g: g as f64 / 255.0,
            b: b as f64 / 255.0,
        }
    }

    fn luminance(&self) -> f64 {
        0.2126 * self.r + 0.7152 * self.g + 0.0722 * self.b
    }

    fn set(&self, cr: &cairo::Context) {
        cr.set_source_rgb(self.r, self.g, self.b);
    }
}

/// (name, css hex, colour)
const PALETTE: &[(&str, &str, Rgb)] = &[
    ("Red", "#e01b24", Rgb::new(0xe0, 0x1b, 0x24)),
    ("Orange", "#ff7800", Rgb::new(0xff, 0x78, 0x00)),
    ("Yellow", "#f6d32d", Rgb::new(0xf6, 0xd3, 0x2d)),
    ("Green", "#33d17a", Rgb::new(0x33, 0xd1, 0x7a)),
    ("Blue", "#3584e4", Rgb::new(0x35, 0x84, 0xe4)),
    ("Black", "#000000", Rgb::new(0x00, 0x00, 0x00)),
    ("White", "#ffffff", Rgb::new(0xff, 0xff, 0xff)),
];

/// (name, multiplier applied to the base stroke width and font size)
const SIZES: &[(&str, f64)] = &[("Thin", 0.6), ("Medium", 1.0), ("Thick", 1.8)];

type Point = (f64, f64);

#[derive(Clone, Debug)]
enum Shape {
    Rect { a: Point, b: Point, color: Rgb, width: f64 },
    /// Fills the box between the two corners.
    Ellipse { a: Point, b: Point, color: Rgb, width: f64 },
    Arrow { a: Point, b: Point, color: Rgb, width: f64 },
    Pen { points: Vec<Point>, color: Rgb, width: f64 },
    Text { at: Point, text: String, color: Rgb, size: f64 },
}

struct State {
    image: Option<cairo::ImageSurface>,
    shapes: Vec<Shape>,
    redo: Vec<Shape>,
    current: Option<Shape>,
    tool: Tool,
    color: Rgb,
    size: f64,
    /// View transform, refreshed on every draw.
    scale: f64,
    offset: Point,
    pending_text_at: Point,
}

impl State {
    fn new() -> Self {
        Self {
            image: None,
            shapes: Vec::new(),
            redo: Vec::new(),
            current: None,
            tool: Tool::Rect,
            color: PALETTE[0].2,
            size: SIZES[1].1,
            scale: 1.0,
            offset: (0.0, 0.0),
            pending_text_at: (0.0, 0.0),
        }
    }

    fn image_size(&self) -> (f64, f64) {
        match &self.image {
            Some(img) => (img.width() as f64, img.height() as f64),
            None => (0.0, 0.0),
        }
    }

    /// Stroke width in image pixels, scaled to the image so it reads well on any resolution.
    fn stroke_width(&self) -> f64 {
        let (w, h) = self.image_size();
        (w.max(h) / 250.0).clamp(2.0, 12.0) * self.size
    }

    fn font_size(&self) -> f64 {
        let (w, h) = self.image_size();
        (w.max(h) / 35.0).clamp(14.0, 72.0) * self.size.sqrt()
    }

    fn to_image(&self, x: f64, y: f64) -> Point {
        ((x - self.offset.0) / self.scale, (y - self.offset.1) / self.scale)
    }

    fn push(&mut self, shape: Shape) {
        self.shapes.push(shape);
        self.redo.clear();
    }
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

fn rounded_rect(cr: &cairo::Context, x: f64, y: f64, w: f64, h: f64, r: f64) {
    let r = r.min(w / 2.0).min(h / 2.0);
    cr.new_sub_path();
    cr.arc(x + w - r, y + r, r, -PI / 2.0, 0.0);
    cr.arc(x + w - r, y + h - r, r, 0.0, PI / 2.0);
    cr.arc(x + r, y + h - r, r, PI / 2.0, PI);
    cr.arc(x + r, y + r, r, PI, 1.5 * PI);
    cr.close_path();
}

/// Image plus annotations, in image coordinates. Used for both preview and export.
fn render(cr: &cairo::Context, st: &State) {
    if let Some(img) = &st.image {
        let _ = cr.set_source_surface(img, 0.0, 0.0);
        let _ = cr.paint();
    }
    for shape in st.shapes.iter().chain(st.current.iter()) {
        draw_shape(cr, shape);
    }
}

fn draw_shape(cr: &cairo::Context, shape: &Shape) {
    cr.set_line_cap(cairo::LineCap::Round);
    cr.set_line_join(cairo::LineJoin::Round);

    match shape {
        Shape::Rect { a, b, color, width } => {
            color.set(cr);
            cr.set_line_width(*width);
            cr.rectangle(a.0.min(b.0), a.1.min(b.1), (b.0 - a.0).abs(), (b.1 - a.1).abs());
            let _ = cr.stroke();
        }
        Shape::Ellipse { a, b, color, width } => {
            let (rx, ry) = ((b.0 - a.0).abs() / 2.0, (b.1 - a.1).abs() / 2.0);
            // A zero radius would make the scale matrix singular.
            if rx < 0.5 || ry < 0.5 {
                return;
            }
            color.set(cr);
            cr.set_line_width(*width);
            let _ = cr.save();
            cr.translate((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0);
            cr.scale(rx, ry);
            cr.arc(0.0, 0.0, 1.0, 0.0, TAU);
            // Stroke after restoring, so the line keeps an even width around the ellipse.
            let _ = cr.restore();
            let _ = cr.stroke();
        }
        Shape::Arrow { a, b, color, width } => {
            color.set(cr);
            cr.set_line_width(*width);
            let angle = (b.1 - a.1).atan2(b.0 - a.0);
            let head = width * 4.0;
            let spread: f64 = 0.5;
            // Stop the shaft inside the head so the tip stays sharp.
            let shaft = head * spread.cos() * 0.8;
            let len = ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt();
            if len > shaft {
                cr.move_to(a.0, a.1);
                cr.line_to(b.0 - shaft * angle.cos(), b.1 - shaft * angle.sin());
                let _ = cr.stroke();
            }
            cr.move_to(b.0, b.1);
            cr.line_to(b.0 - head * (angle - spread).cos(), b.1 - head * (angle - spread).sin());
            cr.line_to(b.0 - head * (angle + spread).cos(), b.1 - head * (angle + spread).sin());
            cr.close_path();
            let _ = cr.fill_preserve();
            cr.set_line_width(width * 0.5);
            let _ = cr.stroke();
        }
        Shape::Pen { points, color, width } => {
            color.set(cr);
            cr.set_line_width(*width);
            match points.as_slice() {
                [] => {}
                [p] => {
                    cr.arc(p.0, p.1, width / 2.0, 0.0, TAU);
                    let _ = cr.fill();
                }
                [first, rest @ ..] => {
                    cr.move_to(first.0, first.1);
                    for p in rest {
                        cr.line_to(p.0, p.1);
                    }
                    let _ = cr.stroke();
                }
            }
        }
        Shape::Text { at, text, color, size } => {
            cr.select_font_face("Sans", cairo::FontSlant::Normal, cairo::FontWeight::Bold);
            cr.set_font_size(*size);
            cr.move_to(at.0, at.1);
            cr.text_path(text);
            // Contrasting outline so the label stays readable on any background.
            if color.luminance() > 0.5 {
                cr.set_source_rgb(0.0, 0.0, 0.0);
            } else {
                cr.set_source_rgb(1.0, 1.0, 1.0);
            }
            cr.set_line_width(size * 0.12);
            let _ = cr.stroke_preserve();
            color.set(cr);
            let _ = cr.fill();
        }
    }
}

fn render_png(st: &State) -> Result<Vec<u8>, String> {
    let img = st.image.as_ref().ok_or("no image loaded")?;
    let (w, h) = (img.width(), img.height());
    let surface = cairo::ImageSurface::create(cairo::Format::ARgb32, w, h)
        .map_err(|e| format!("cannot create surface: {e}"))?;
    {
        let cr = cairo::Context::new(&surface).map_err(|e| format!("cairo: {e}"))?;
        render(&cr, st);
    }
    surface.flush();
    let mut png = Vec::new();
    surface
        .write_to_png(&mut png)
        .map_err(|e| format!("png encoding failed: {e}"))?;
    Ok(png)
}

/// Download a GDK texture into a cairo image surface (premultiplied ARGB32).
fn texture_to_surface(tex: &gdk::Texture) -> Option<cairo::ImageSurface> {
    let (w, h) = (tex.width(), tex.height());
    let stride = cairo::Format::ARgb32.stride_for_width(w as u32).ok()?;
    let mut data = vec![0u8; stride as usize * h as usize];
    tex.download(&mut data, stride as usize);
    cairo::ImageSurface::create_for_data(data, cairo::Format::ARgb32, w, h, stride).ok()
}

fn copy_to_clipboard(png: &[u8]) -> Result<(), String> {
    let mut child = Command::new("wl-copy")
        .args(["--type", "image/png"])
        .stdin(Stdio::piped())
        .spawn()
        .map_err(|e| format!("cannot run wl-copy: {e}"))?;
    child
        .stdin
        .take()
        .ok_or("wl-copy stdin unavailable")?
        .write_all(png)
        .map_err(|e| format!("cannot write to wl-copy: {e}"))?;
    let status = child.wait().map_err(|e| format!("wl-copy: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("wl-copy exited with {status}"))
    }
}

// ---------------------------------------------------------------------------
// Toolbar icons, drawn with cairo in the widget's current foreground colour
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
enum Glyph {
    Tool(Tool),
    Dot(f64),
}

fn glyph(kind: Glyph) -> gtk::DrawingArea {
    let area = gtk::DrawingArea::builder()
        .content_width(18)
        .content_height(18)
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .can_target(false)
        .build();
    area.set_draw_func(move |w, cr, _, _| {
        let c = w.color();
        cr.set_source_rgba(c.red() as f64, c.green() as f64, c.blue() as f64, c.alpha() as f64);
        cr.set_line_width(1.75);
        cr.set_line_cap(cairo::LineCap::Round);
        cr.set_line_join(cairo::LineJoin::Round);
        match kind {
            Glyph::Tool(Tool::Rect) => {
                rounded_rect(cr, 2.5, 4.0, 13.0, 10.0, 2.0);
                let _ = cr.stroke();
            }
            Glyph::Tool(Tool::Ellipse) => {
                cr.arc(9.0, 9.0, 6.5, 0.0, TAU);
                let _ = cr.stroke();
            }
            Glyph::Tool(Tool::Arrow) => {
                cr.move_to(3.0, 15.0);
                cr.line_to(14.0, 4.0);
                cr.move_to(7.5, 4.0);
                cr.line_to(14.0, 4.0);
                cr.line_to(14.0, 10.5);
                let _ = cr.stroke();
            }
            Glyph::Tool(Tool::Pen) => {
                cr.move_to(2.5, 12.0);
                cr.curve_to(5.0, 4.0, 8.0, 4.0, 9.0, 9.0);
                cr.curve_to(10.0, 14.0, 13.0, 14.0, 15.5, 6.0);
                let _ = cr.stroke();
            }
            Glyph::Tool(Tool::Text) => {
                cr.set_line_width(2.25);
                cr.move_to(3.5, 4.0);
                cr.line_to(14.5, 4.0);
                cr.move_to(9.0, 4.0);
                cr.line_to(9.0, 15.0);
                let _ = cr.stroke();
            }
            Glyph::Dot(r) => {
                cr.arc(9.0, 9.0, r, 0.0, TAU);
                let _ = cr.fill();
            }
        }
    });
    area
}

fn css() -> String {
    let mut s = String::from(
        r#"
        .canvas { background-color: @view_bg_color; }

        .pill {
            background-color: @popover_bg_color;
            color: @popover_fg_color;
            border-radius: 9999px;
            padding: 6px 10px;
            margin: 0 12px 18px 12px;
            box-shadow: 0 0 0 1px alpha(black, 0.15),
                        0 4px 12px alpha(black, 0.25),
                        0 1px 3px alpha(black, 0.2);
        }
        .pill separator { margin: 8px 8px; }

        button.tool {
            min-width: 34px;
            min-height: 34px;
            padding: 0;
            border-radius: 9999px;
        }
        button.tool:checked {
            background-color: @accent_bg_color;
            color: @accent_fg_color;
        }

        button.swatch {
            min-width: 20px;
            min-height: 20px;
            padding: 0;
            margin: 0 3px;
            border-radius: 9999px;
            background-image: none;
            box-shadow: inset 0 0 0 1px alpha(black, 0.25);
            transition: box-shadow 120ms ease-out;
        }
        button.swatch:hover { box-shadow: inset 0 0 0 1px alpha(black, 0.25),
                                          0 0 0 2px alpha(@popover_fg_color, 0.25); }
        button.swatch:checked { box-shadow: inset 0 0 0 1px alpha(black, 0.25),
                                            0 0 0 2px @popover_bg_color,
                                            0 0 0 4px @popover_fg_color; }
        "#,
    );
    for (i, (_, hex, _)) in PALETTE.iter().enumerate() {
        s += &format!("button.swatch.c{i} {{ background-color: {hex}; }}\n");
    }
    s
}

// ---------------------------------------------------------------------------
// UI
// ---------------------------------------------------------------------------

/// A group of mutually exclusive, flat, icon-only toggle buttons.
fn toggle_group<T: Copy + 'static>(
    items: &[(T, Glyph, String)],
    on_select: impl Fn(T) + 'static,
) -> (gtk::Box, Vec<gtk::ToggleButton>) {
    let on_select = Rc::new(on_select);
    let bx = gtk::Box::new(gtk::Orientation::Horizontal, 2);
    let mut buttons: Vec<gtk::ToggleButton> = Vec::new();
    for (value, g, tip) in items {
        let icon = glyph(*g);
        let b = gtk::ToggleButton::builder()
            .child(&icon)
            .tooltip_text(tip)
            .focus_on_click(false)
            .build();
        b.add_css_class("flat");
        b.add_css_class("tool");
        if let Some(first) = buttons.first() {
            b.set_group(Some(first));
        }
        let on_select = on_select.clone();
        let value = *value;
        b.connect_toggled(move |b| {
            icon.queue_draw();
            if b.is_active() {
                on_select(value);
            }
        });
        bx.append(&b);
        buttons.push(b);
    }
    (bx, buttons)
}

fn vsep() -> gtk::Separator {
    gtk::Separator::new(gtk::Orientation::Vertical)
}

fn build_ui(app: &adw::Application) {
    let state = Rc::new(RefCell::new(State::new()));

    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title("0xMark")
        .default_width(1100)
        .default_height(760)
        .build();

    // --- canvas ------------------------------------------------------------
    let area = gtk::DrawingArea::builder().hexpand(true).vexpand(true).build();
    area.add_css_class("canvas");
    area.set_focusable(true);
    area.set_cursor_from_name(Some("crosshair"));
    {
        let state = state.clone();
        area.set_draw_func(move |_, cr, w, h| {
            let mut st = state.borrow_mut();
            let (iw, ih) = st.image_size();
            if iw <= 0.0 || ih <= 0.0 {
                return;
            }
            // Leave room at the bottom for the floating toolbar.
            let (pad_x, pad_top, pad_bottom) = (32.0, 24.0, 88.0);
            let avail_w = w as f64 - 2.0 * pad_x;
            let avail_h = h as f64 - pad_top - pad_bottom;
            let scale = (avail_w / iw).min(avail_h / ih).clamp(0.05, 1.0);
            let (dw, dh) = (iw * scale, ih * scale);
            let ox = ((w as f64 - dw) / 2.0).round();
            let oy = (pad_top + (avail_h - dh) / 2.0).round();
            st.scale = scale;
            st.offset = (ox, oy);

            // Soft drop shadow.
            for i in 1..=10 {
                let s = i as f64 * 1.6;
                cr.set_source_rgba(0.0, 0.0, 0.0, 0.028);
                rounded_rect(cr, ox - s / 2.0, oy - s / 2.0 + 3.0, dw + s, dh + s, 6.0 + s / 2.0);
                let _ = cr.fill();
            }

            let _ = cr.save();
            rounded_rect(cr, ox, oy, dw, dh, 6.0);
            cr.clip();
            cr.translate(ox, oy);
            cr.scale(scale, scale);
            render(cr, &st);
            let _ = cr.restore();
        });
    }

    let empty = adw::StatusPage::builder()
        .icon_name("edit-paste-symbolic")
        .title("No Image in Clipboard")
        .description("Copy an image, then press Ctrl+V")
        .build();

    // --- floating toolbar --------------------------------------------------
    let (tools_box, tool_buttons) = {
        let state = state.clone();
        let area = area.clone();
        toggle_group(
            &[
                (Tool::Rect, Glyph::Tool(Tool::Rect), "Rectangle  (R)".to_string()),
                (Tool::Ellipse, Glyph::Tool(Tool::Ellipse), "Circle  (C, hold Shift for a perfect circle)".to_string()),
                (Tool::Arrow, Glyph::Tool(Tool::Arrow), "Arrow  (A)".to_string()),
                (Tool::Pen, Glyph::Tool(Tool::Pen), "Pen  (P)".to_string()),
                (Tool::Text, Glyph::Tool(Tool::Text), "Text  (T)".to_string()),
            ],
            move |tool| {
                state.borrow_mut().tool = tool;
                area.set_cursor_from_name(Some(if tool == Tool::Text { "text" } else { "crosshair" }));
            },
        )
    };
    tool_buttons[0].set_active(true);

    let colors_box = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    colors_box.set_valign(gtk::Align::Center);
    let mut swatches: Vec<gtk::ToggleButton> = Vec::new();
    for (i, (name, _, rgb)) in PALETTE.iter().enumerate() {
        let b = gtk::ToggleButton::builder()
            .tooltip_text(*name)
            .focus_on_click(false)
            .valign(gtk::Align::Center)
            .build();
        b.add_css_class("swatch");
        b.add_css_class(&format!("c{i}"));
        if let Some(first) = swatches.first() {
            b.set_group(Some(first));
        }
        let state = state.clone();
        let rgb = *rgb;
        b.connect_toggled(move |b| {
            if b.is_active() {
                state.borrow_mut().color = rgb;
            }
        });
        colors_box.append(&b);
        swatches.push(b);
    }
    swatches[0].set_active(true);

    let (sizes_box, size_buttons) = {
        let state = state.clone();
        let items: Vec<_> = SIZES
            .iter()
            .enumerate()
            .map(|(i, (name, k))| (*k, Glyph::Dot(1.75 + 1.9 * i as f64), format!("{name}  ({})", i + 1)))
            .collect();
        toggle_group(&items, move |k| state.borrow_mut().size = k)
    };
    size_buttons[1].set_active(true);

    let pill = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .halign(gtk::Align::Center)
        .valign(gtk::Align::End)
        .visible(false)
        .build();
    pill.add_css_class("pill");
    pill.append(&tools_box);
    pill.append(&vsep());
    pill.append(&colors_box);
    pill.append(&vsep());
    pill.append(&sizes_box);

    let canvas_overlay = gtk::Overlay::new();
    canvas_overlay.set_child(Some(&area));
    canvas_overlay.add_overlay(&pill);

    let stack = gtk::Stack::builder()
        .transition_type(gtk::StackTransitionType::Crossfade)
        .build();
    stack.add_named(&empty, Some("empty"));
    stack.add_named(&canvas_overlay, Some("canvas"));

    let toasts = adw::ToastOverlay::new();
    toasts.set_child(Some(&stack));

    // --- header bar --------------------------------------------------------
    let title = adw::WindowTitle::new("0xMark", "");
    let header = adw::HeaderBar::new();
    header.set_title_widget(Some(&title));

    let undo_button = gtk::Button::builder()
        .icon_name("edit-undo-symbolic")
        .tooltip_text("Undo  (Ctrl+Z)")
        .sensitive(false)
        .build();
    let redo_button = gtk::Button::builder()
        .icon_name("edit-redo-symbolic")
        .tooltip_text("Redo  (Ctrl+Shift+Z)")
        .sensitive(false)
        .build();
    header.pack_start(&undo_button);
    header.pack_start(&redo_button);

    let copy_button = gtk::Button::builder()
        .child(&adw::ButtonContent::builder().icon_name("edit-copy-symbolic").label("Copy").build())
        .tooltip_text("Copy to clipboard and close  (Enter)")
        .sensitive(false)
        .build();
    copy_button.add_css_class("suggested-action");
    header.pack_end(&copy_button);

    // Redraw and refresh button sensitivity after any change to the shapes.
    let changed: Rc<dyn Fn()> = {
        let state = state.clone();
        let area = area.clone();
        let undo_button = undo_button.clone();
        let redo_button = redo_button.clone();
        let copy_button = copy_button.clone();
        Rc::new(move || {
            let st = state.borrow();
            undo_button.set_sensitive(!st.shapes.is_empty());
            redo_button.set_sensitive(!st.redo.is_empty());
            copy_button.set_sensitive(st.image.is_some());
            area.queue_draw();
        })
    };

    // --- text entry popover ------------------------------------------------
    let entry = gtk::Entry::builder()
        .placeholder_text("Type a label, press Enter")
        .width_chars(26)
        .build();
    let popover = gtk::Popover::builder().child(&entry).autohide(true).build();
    popover.set_parent(&area);
    {
        let state = state.clone();
        let popover = popover.clone();
        let changed = changed.clone();
        entry.connect_activate(move |e| {
            let text = e.text().trim().to_string();
            if !text.is_empty() {
                let mut st = state.borrow_mut();
                let shape = Shape::Text {
                    at: st.pending_text_at,
                    text,
                    color: st.color,
                    size: st.font_size(),
                };
                st.push(shape);
            }
            popover.popdown();
            changed();
        });
    }

    // --- drawing gesture ---------------------------------------------------
    let drag = gtk::GestureDrag::new();
    {
        let state = state.clone();
        let area = area.clone();
        let popover = popover.clone();
        let entry = entry.clone();
        drag.connect_drag_begin(move |_, x, y| {
            let mut st = state.borrow_mut();
            if st.image.is_none() {
                return;
            }
            let p = st.to_image(x, y);
            let (color, width) = (st.color, st.stroke_width());
            match st.tool {
                Tool::Rect => st.current = Some(Shape::Rect { a: p, b: p, color, width }),
                Tool::Ellipse => st.current = Some(Shape::Ellipse { a: p, b: p, color, width }),
                Tool::Arrow => st.current = Some(Shape::Arrow { a: p, b: p, color, width }),
                Tool::Pen => st.current = Some(Shape::Pen { points: vec![p], color, width }),
                Tool::Text => {
                    st.pending_text_at = p;
                    drop(st);
                    popover.set_pointing_to(Some(&gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
                    entry.set_text("");
                    popover.popup();
                    entry.grab_focus();
                    return;
                }
            }
            area.queue_draw();
        });
    }
    {
        let state = state.clone();
        let area = area.clone();
        drag.connect_drag_update(move |g, dx, dy| {
            let Some((sx, sy)) = g.start_point() else { return };
            let mut st = state.borrow_mut();
            let p = st.to_image(sx + dx, sy + dy);
            let shift = g.current_event_state().contains(gdk::ModifierType::SHIFT_MASK);
            match &mut st.current {
                Some(Shape::Rect { b, .. }) | Some(Shape::Arrow { b, .. }) => *b = p,
                // Shift turns the bounding box into a square, so the ellipse is a circle.
                Some(Shape::Ellipse { a, b, .. }) if shift => {
                    let side = (p.0 - a.0).abs().max((p.1 - a.1).abs());
                    *b = (a.0 + side.copysign(p.0 - a.0), a.1 + side.copysign(p.1 - a.1));
                }
                Some(Shape::Ellipse { b, .. }) => *b = p,
                Some(Shape::Pen { points, .. }) => points.push(p),
                _ => {}
            }
            area.queue_draw();
        });
    }
    {
        let state = state.clone();
        let changed = changed.clone();
        drag.connect_drag_end(move |_, _, _| {
            let mut st = state.borrow_mut();
            if let Some(s) = st.current.take() {
                st.push(s);
            }
            drop(st);
            changed();
        });
    }
    area.add_controller(drag);

    // --- actions -----------------------------------------------------------
    let load_clipboard: Rc<dyn Fn()> = {
        let state = state.clone();
        let stack = stack.clone();
        let pill = pill.clone();
        let area = area.clone();
        let title = title.clone();
        let toasts = toasts.clone();
        let window = window.clone();
        let changed = changed.clone();
        Rc::new(move || {
            let clipboard = window.clipboard();
            let state = state.clone();
            let stack = stack.clone();
            let pill = pill.clone();
            let area = area.clone();
            let title = title.clone();
            let toasts = toasts.clone();
            let changed = changed.clone();
            glib::spawn_future_local(async move {
                let texture = clipboard.read_texture_future().await.ok().flatten();
                match texture.and_then(|t| texture_to_surface(&t)) {
                    Some(img) => {
                        title.set_subtitle(&format!("{} × {}", img.width(), img.height()));
                        let mut st = state.borrow_mut();
                        st.image = Some(img);
                        st.shapes.clear();
                        st.redo.clear();
                        st.current = None;
                        drop(st);
                        stack.set_visible_child_name("canvas");
                        pill.set_visible(true);
                        area.grab_focus();
                    }
                    None => {
                        if state.borrow().image.is_none() {
                            stack.set_visible_child_name("empty");
                        }
                        toasts.add_toast(adw::Toast::new("Clipboard has no image"));
                    }
                }
                changed();
            });
        })
    };

    let copy_and_quit: Rc<dyn Fn()> = {
        let state = state.clone();
        let toasts = toasts.clone();
        let app = app.clone();
        Rc::new(move || {
            let result = render_png(&state.borrow()).and_then(|png| copy_to_clipboard(&png));
            match result {
                Ok(()) => app.quit(),
                Err(e) => toasts.add_toast(adw::Toast::new(&e)),
            }
        })
    };

    let undo: Rc<dyn Fn()> = {
        let state = state.clone();
        let changed = changed.clone();
        Rc::new(move || {
            let mut st = state.borrow_mut();
            if let Some(s) = st.shapes.pop() {
                st.redo.push(s);
            }
            drop(st);
            changed();
        })
    };

    let redo: Rc<dyn Fn()> = {
        let state = state.clone();
        let changed = changed.clone();
        Rc::new(move || {
            let mut st = state.borrow_mut();
            if let Some(s) = st.redo.pop() {
                st.shapes.push(s);
            }
            drop(st);
            changed();
        })
    };

    {
        let copy_and_quit = copy_and_quit.clone();
        copy_button.connect_clicked(move |_| copy_and_quit());
    }
    {
        let undo = undo.clone();
        undo_button.connect_clicked(move |_| undo());
    }
    {
        let redo = redo.clone();
        redo_button.connect_clicked(move |_| redo());
    }

    // --- keyboard ----------------------------------------------------------
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    {
        let window = window.clone();
        let popover = popover.clone();
        let tool_buttons = tool_buttons.clone();
        let size_buttons = size_buttons.clone();
        let load_clipboard = load_clipboard.clone();
        keys.connect_key_pressed(move |_, key, _, modifier| {
            if popover.is_visible() {
                return glib::Propagation::Proceed;
            }
            let ctrl = modifier.contains(gdk::ModifierType::CONTROL_MASK);
            let shift = modifier.contains(gdk::ModifierType::SHIFT_MASK);
            match (key.to_lower(), ctrl, shift) {
                (gdk::Key::Escape, _, _) => window.close(),
                (gdk::Key::Return | gdk::Key::KP_Enter, _, _) | (gdk::Key::c, true, _) => {
                    copy_and_quit()
                }
                (gdk::Key::z, true, true) | (gdk::Key::y, true, _) => redo(),
                (gdk::Key::z, true, false) => undo(),
                (gdk::Key::v, true, _) => load_clipboard(),
                (gdk::Key::r, false, _) => tool_buttons[0].set_active(true),
                (gdk::Key::c, false, _) => tool_buttons[1].set_active(true),
                (gdk::Key::a, false, _) => tool_buttons[2].set_active(true),
                (gdk::Key::p, false, _) => tool_buttons[3].set_active(true),
                (gdk::Key::t, false, _) => tool_buttons[4].set_active(true),
                (gdk::Key::_1, false, _) => size_buttons[0].set_active(true),
                (gdk::Key::_2, false, _) => size_buttons[1].set_active(true),
                (gdk::Key::_3, false, _) => size_buttons[2].set_active(true),
                _ => return glib::Propagation::Proceed,
            }
            glib::Propagation::Stop
        });
    }
    window.add_controller(keys);

    // --- assemble ----------------------------------------------------------
    let view = adw::ToolbarView::new();
    view.add_top_bar(&header);
    view.set_content(Some(&toasts));
    window.set_content(Some(&view));

    // On Wayland the clipboard offer arrives once the window has keyboard focus,
    // so load on first activation rather than right away.
    let loaded = Rc::new(Cell::new(false));
    {
        let load_clipboard = load_clipboard.clone();
        window.connect_is_active_notify(move |w| {
            if w.is_active() && !loaded.replace(true) {
                load_clipboard();
            }
        });
    }

    window.present();
}

fn main() -> glib::ExitCode {
    let app = adw::Application::builder()
        .application_id(APP_ID)
        .flags(gio::ApplicationFlags::NON_UNIQUE)
        .build();

    app.connect_startup(|_| {
        gtk::Window::set_default_icon_name(APP_ID);
        let provider = gtk::CssProvider::new();
        provider.load_from_string(&css());
        if let Some(display) = gdk::Display::default() {
            gtk::style_context_add_provider_for_display(
                &display,
                &provider,
                gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
            );
        }
    });
    app.connect_activate(build_ui);
    app.run()
}
