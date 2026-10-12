//! 0xMark: paste an image from the clipboard, annotate it, copy it back.

use std::cell::{Cell, RefCell};
use std::io::Write;
use std::process::{Command, Stdio};
use std::rc::Rc;

use adw::prelude::*;
use gtk::{cairo, gdk, gio, glib};

const APP_ID: &str = "dev.abidibo.OxMark";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Tool {
    Rect,
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

type Point = (f64, f64);

#[derive(Clone, Debug)]
enum Shape {
    Rect { a: Point, b: Point, color: Rgb },
    Arrow { a: Point, b: Point, color: Rgb },
    Pen { points: Vec<Point>, color: Rgb },
    Text { at: Point, text: String, color: Rgb },
}

struct State {
    image: Option<cairo::ImageSurface>,
    shapes: Vec<Shape>,
    current: Option<Shape>,
    tool: Tool,
    color: Rgb,
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
            current: None,
            tool: Tool::Rect,
            color: PALETTE[0].2,
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

    fn line_width(&self) -> f64 {
        let (w, h) = self.image_size();
        (w.max(h) / 250.0).clamp(2.0, 12.0)
    }

    fn font_size(&self) -> f64 {
        let (w, h) = self.image_size();
        (w.max(h) / 35.0).clamp(14.0, 72.0)
    }

    fn to_image(&self, x: f64, y: f64) -> Point {
        ((x - self.offset.0) / self.scale, (y - self.offset.1) / self.scale)
    }
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

fn render(cr: &cairo::Context, st: &State) {
    if let Some(img) = &st.image {
        let _ = cr.set_source_surface(img, 0.0, 0.0);
        let _ = cr.paint();
    }
    for shape in st.shapes.iter().chain(st.current.iter()) {
        draw_shape(cr, shape, st);
    }
}

fn draw_shape(cr: &cairo::Context, shape: &Shape, st: &State) {
    let lw = st.line_width();
    cr.set_line_width(lw);
    cr.set_line_cap(cairo::LineCap::Round);
    cr.set_line_join(cairo::LineJoin::Round);

    match shape {
        Shape::Rect { a, b, color } => {
            cr.set_source_rgb(color.r, color.g, color.b);
            cr.rectangle(a.0.min(b.0), a.1.min(b.1), (b.0 - a.0).abs(), (b.1 - a.1).abs());
            let _ = cr.stroke();
        }
        Shape::Arrow { a, b, color } => {
            cr.set_source_rgb(color.r, color.g, color.b);
            cr.move_to(a.0, a.1);
            cr.line_to(b.0, b.1);
            let _ = cr.stroke();

            let angle = (b.1 - a.1).atan2(b.0 - a.0);
            let head = lw * 4.0;
            let spread = 0.5;
            let p1 = (b.0 - head * (angle - spread).cos(), b.1 - head * (angle - spread).sin());
            let p2 = (b.0 - head * (angle + spread).cos(), b.1 - head * (angle + spread).sin());
            cr.move_to(b.0, b.1);
            cr.line_to(p1.0, p1.1);
            cr.line_to(p2.0, p2.1);
            cr.close_path();
            let _ = cr.fill();
        }
        Shape::Pen { points, color } => {
            cr.set_source_rgb(color.r, color.g, color.b);
            match points.as_slice() {
                [] => {}
                [p] => {
                    cr.arc(p.0, p.1, lw / 2.0, 0.0, std::f64::consts::TAU);
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
        Shape::Text { at, text, color } => {
            cr.select_font_face("Sans", cairo::FontSlant::Normal, cairo::FontWeight::Bold);
            cr.set_font_size(st.font_size());
            cr.move_to(at.0, at.1);
            cr.text_path(text);
            // Contrasting outline so the label stays readable on any background.
            if color.luminance() > 0.5 {
                cr.set_source_rgb(0.0, 0.0, 0.0);
            } else {
                cr.set_source_rgb(1.0, 1.0, 1.0);
            }
            cr.set_line_width(lw * 0.6);
            let _ = cr.stroke_preserve();
            cr.set_source_rgb(color.r, color.g, color.b);
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
// UI
// ---------------------------------------------------------------------------

fn css() -> String {
    let mut s = String::from(
        "button.swatch{min-width:16px;min-height:16px;padding:0;margin:4px 1px;\
         border-radius:9999px;border:2px solid transparent;background-image:none;}\n\
         button.swatch:checked{border-color:@window_fg_color;}\n",
    );
    for (i, (_, hex, _)) in PALETTE.iter().enumerate() {
        s += &format!(
            "button.swatch.c{i},button.swatch.c{i}:hover,button.swatch.c{i}:checked,\
             button.swatch.c{i}:active{{background-color:{hex};}}\n"
        );
    }
    s
}

fn build_ui(app: &adw::Application) {
    let state = Rc::new(RefCell::new(State::new()));

    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title("0xMark")
        .default_width(1100)
        .default_height(750)
        .build();

    // --- canvas ------------------------------------------------------------
    let area = gtk::DrawingArea::builder().hexpand(true).vexpand(true).build();
    {
        let state = state.clone();
        area.set_draw_func(move |_, cr, w, h| {
            let mut st = state.borrow_mut();
            let (iw, ih) = st.image_size();
            if iw <= 0.0 || ih <= 0.0 {
                return;
            }
            let pad = 16.0;
            let scale = ((w as f64 - 2.0 * pad) / iw)
                .min((h as f64 - 2.0 * pad) / ih)
                .min(1.0);
            let ox = ((w as f64) - iw * scale) / 2.0;
            let oy = ((h as f64) - ih * scale) / 2.0;
            st.scale = scale;
            st.offset = (ox, oy);

            let _ = cr.save();
            cr.translate(ox, oy);
            cr.scale(scale, scale);
            render(cr, &st);
            let _ = cr.restore();
        });
    }

    let empty = adw::StatusPage::builder()
        .icon_name("edit-paste-symbolic")
        .title("No image in clipboard")
        .description("Copy an image, then press Ctrl+V")
        .build();

    let stack = gtk::Stack::new();
    stack.add_named(&empty, Some("empty"));
    stack.add_named(&area, Some("canvas"));

    let toasts = adw::ToastOverlay::new();
    toasts.set_child(Some(&stack));

    // --- text entry popover ------------------------------------------------
    let entry = gtk::Entry::builder()
        .placeholder_text("Label, then Enter")
        .width_chars(24)
        .build();
    let popover = gtk::Popover::builder().child(&entry).autohide(true).build();
    popover.set_parent(&area);
    {
        let state = state.clone();
        let area = area.clone();
        let popover = popover.clone();
        entry.connect_activate(move |e| {
            let text = e.text().to_string();
            if !text.trim().is_empty() {
                let mut st = state.borrow_mut();
                let at = st.pending_text_at;
                let color = st.color;
                st.shapes.push(Shape::Text { at, text, color });
            }
            popover.popdown();
            area.queue_draw();
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
            let color = st.color;
            match st.tool {
                Tool::Rect => st.current = Some(Shape::Rect { a: p, b: p, color }),
                Tool::Arrow => st.current = Some(Shape::Arrow { a: p, b: p, color }),
                Tool::Pen => st.current = Some(Shape::Pen { points: vec![p], color }),
                Tool::Text => {
                    st.pending_text_at = p;
                    drop(st);
                    popover.set_pointing_to(Some(&gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
                    entry.set_text("");
                    popover.popup();
                    entry.grab_focus();
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
            match &mut st.current {
                Some(Shape::Rect { b, .. }) | Some(Shape::Arrow { b, .. }) => *b = p,
                Some(Shape::Pen { points, .. }) => points.push(p),
                _ => {}
            }
            area.queue_draw();
        });
    }
    {
        let state = state.clone();
        let area = area.clone();
        drag.connect_drag_end(move |_, _, _| {
            let mut st = state.borrow_mut();
            if let Some(s) = st.current.take() {
                st.shapes.push(s);
            }
            area.queue_draw();
        });
    }
    area.add_controller(drag);

    // --- actions -----------------------------------------------------------
    let load_clipboard: Rc<dyn Fn()> = {
        let state = state.clone();
        let area = area.clone();
        let stack = stack.clone();
        let toasts = toasts.clone();
        let window = window.clone();
        Rc::new(move || {
            let clipboard = window.clipboard();
            let state = state.clone();
            let area = area.clone();
            let stack = stack.clone();
            let toasts = toasts.clone();
            glib::spawn_future_local(async move {
                let texture = clipboard.read_texture_future().await.ok().flatten();
                let image = texture.and_then(|t| texture_to_surface(&t));
                match image {
                    Some(img) => {
                        let mut st = state.borrow_mut();
                        st.image = Some(img);
                        st.shapes.clear();
                        st.current = None;
                        stack.set_visible_child_name("canvas");
                        area.queue_draw();
                    }
                    None => {
                        stack.set_visible_child_name("empty");
                        toasts.add_toast(adw::Toast::new("Clipboard has no image"));
                    }
                }
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
        let area = area.clone();
        Rc::new(move || {
            state.borrow_mut().shapes.pop();
            area.queue_draw();
        })
    };

    // --- header bar --------------------------------------------------------
    let header = adw::HeaderBar::new();
    header.set_title_widget(Some(&adw::WindowTitle::new(
        "0xMark",
        "Enter: copy & quit   ·   Ctrl+Z: undo   ·   Esc: quit",
    )));

    let tools_box = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    tools_box.add_css_class("linked");
    let tool_defs = [
        (Tool::Rect, "Rect", "Rectangle (R)"),
        (Tool::Arrow, "Arrow", "Arrow (A)"),
        (Tool::Pen, "Pen", "Freehand pen (P)"),
        (Tool::Text, "Text", "Text label (T), click to place"),
    ];
    let mut tool_buttons: Vec<gtk::ToggleButton> = Vec::new();
    for (tool, label, tip) in tool_defs {
        let b = gtk::ToggleButton::builder().label(label).tooltip_text(tip).build();
        if let Some(first) = tool_buttons.first() {
            b.set_group(Some(first));
        }
        let state = state.clone();
        b.connect_toggled(move |b| {
            if b.is_active() {
                state.borrow_mut().tool = tool;
            }
        });
        tools_box.append(&b);
        tool_buttons.push(b);
    }
    tool_buttons[0].set_active(true);
    header.pack_start(&tools_box);

    let colors_box = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    let mut swatches: Vec<gtk::ToggleButton> = Vec::new();
    for (i, (name, _, rgb)) in PALETTE.iter().enumerate() {
        let b = gtk::ToggleButton::builder().tooltip_text(*name).build();
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
    header.pack_start(&colors_box);

    let copy_button = gtk::Button::builder()
        .label("Copy")
        .tooltip_text("Copy to clipboard and quit (Enter)")
        .build();
    copy_button.add_css_class("suggested-action");
    {
        let copy_and_quit = copy_and_quit.clone();
        copy_button.connect_clicked(move |_| copy_and_quit());
    }
    header.pack_end(&copy_button);

    let undo_button = gtk::Button::builder()
        .icon_name("edit-undo-symbolic")
        .tooltip_text("Undo (Ctrl+Z)")
        .build();
    {
        let undo = undo.clone();
        undo_button.connect_clicked(move |_| undo());
    }
    header.pack_end(&undo_button);

    // --- keyboard ----------------------------------------------------------
    let keys = gtk::EventControllerKey::new();
    {
        let window = window.clone();
        let popover = popover.clone();
        let load_clipboard = load_clipboard.clone();
        let copy_and_quit = copy_and_quit.clone();
        let undo = undo.clone();
        let tool_buttons = tool_buttons.clone();
        keys.connect_key_pressed(move |_, key, _, modifier| {
            if popover.is_visible() {
                return glib::Propagation::Proceed;
            }
            let ctrl = modifier.contains(gdk::ModifierType::CONTROL_MASK);
            match (key, ctrl) {
                (gdk::Key::Escape, _) => window.close(),
                (gdk::Key::Return, _) | (gdk::Key::KP_Enter, _) | (gdk::Key::c, true) => {
                    copy_and_quit()
                }
                (gdk::Key::z, true) => undo(),
                (gdk::Key::v, true) => load_clipboard(),
                (gdk::Key::r, false) => tool_buttons[0].set_active(true),
                (gdk::Key::a, false) => tool_buttons[1].set_active(true),
                (gdk::Key::p, false) => tool_buttons[2].set_active(true),
                (gdk::Key::t, false) => tool_buttons[3].set_active(true),
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
