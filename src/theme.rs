//! Light and dark looks with rounded corners; the accent colours come from the edition (brand.rs).

use eframe::egui::{self, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Stroke, TextStyle};
use std::sync::Arc;

#[derive(Clone, Copy)]
#[allow(dead_code)] // the full set, kept in step with the earlier version
pub struct Palette {
    pub dark: bool,
    pub bg: Color32,
    pub card: Color32,
    pub border: Color32,
    pub text: Color32,
    pub muted: Color32,
    pub accent: Color32,
    pub accent_dark: Color32,
    pub accent_soft: Color32,
    pub accent_off: Color32,
    pub ok: Color32,
    pub err: Color32,
    pub err_soft: Color32,
    pub warn: Color32,
    pub field: Color32,
    pub field_line: Color32,
    pub field_hover: Color32,
    pub disabled: Color32,
    pub button: Color32,
    pub button_hover: Color32,
    pub button_down: Color32,
    pub select: Color32,
    pub stripe: Color32,
    pub head: Color32,
    pub trough: Color32,
    pub switch_off: Color32,
}

const fn hex(v: u32) -> Color32 {
    Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

pub const LIGHT: Palette = Palette {
    dark: false,
    bg: hex(0xeef1f6),
    card: hex(0xffffff),
    border: hex(0xe1e6ee),
    text: hex(0x1c2433),
    muted: hex(0x687386),
    accent: crate::brand::LIGHT_ACCENTS.accent,
    accent_dark: crate::brand::LIGHT_ACCENTS.accent_dark,
    accent_soft: crate::brand::LIGHT_ACCENTS.accent_soft,
    accent_off: crate::brand::LIGHT_ACCENTS.accent_off,
    ok: hex(0x16803c),
    err: hex(0xc42b2b),
    err_soft: hex(0xfdecec),
    warn: hex(0xa15c00),
    field: hex(0xf5f7fb),
    field_line: hex(0xd5dce7),
    field_hover: hex(0xeef2f9),
    disabled: hex(0xa3abb8),
    button: hex(0xf2f4f8),
    button_hover: hex(0xe6ebf3),
    button_down: hex(0xdbe2ee),
    select: crate::brand::LIGHT_ACCENTS.select,
    stripe: hex(0xf8f9fc),
    head: hex(0xf5f7fb),
    trough: hex(0xe5e9f1),
    switch_off: hex(0xc9d0dc),
};

pub const DARK: Palette = Palette {
    dark: true,
    bg: hex(0x121419),
    card: hex(0x1b1e25),
    border: hex(0x2a2f39),
    text: hex(0xe7eaf0),
    muted: hex(0x959eae),
    accent: crate::brand::DARK_ACCENTS.accent,
    accent_dark: crate::brand::DARK_ACCENTS.accent_dark,
    accent_soft: crate::brand::DARK_ACCENTS.accent_soft,
    accent_off: crate::brand::DARK_ACCENTS.accent_off,
    ok: hex(0x4ade80),
    err: hex(0xf87171),
    err_soft: hex(0x3a2023),
    warn: hex(0xfbbf24),
    field: hex(0x242832),
    field_line: hex(0x353b47),
    field_hover: hex(0x2a2f3a),
    disabled: hex(0x5f6776),
    button: hex(0x262b35),
    button_hover: hex(0x2f3542),
    button_down: hex(0x363d4b),
    select: crate::brand::DARK_ACCENTS.select,
    stripe: hex(0x1f232b),
    head: hex(0x21252e),
    trough: hex(0x2a2f39),
    switch_off: hex(0x4a5160),
};

pub fn windows_prefers_dark() -> bool {
    winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER)
        .open_subkey(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize")
        .and_then(|k| k.get_value::<u32, _>("AppsUseLightTheme"))
        .map(|v| v == 0)
        .unwrap_or(false)
}

pub fn semibold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("semibold".into()))
}

/// Segoe UI from Windows when present (looks native); egui's own fonts otherwise.
pub fn install_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    let dir = std::env::var_os("WINDIR").map(std::path::PathBuf::from).unwrap_or_else(|| "C:\\Windows".into()).join("Fonts");
    let mut load = |key: &str, file: &str| -> bool {
        match std::fs::read(dir.join(file)) {
            Ok(bytes) => {
                fonts.font_data.insert(key.to_string(), Arc::new(FontData::from_owned(bytes)));
                true
            }
            Err(_) => false,
        }
    };
    let regular = load("segoe", "segoeui.ttf");
    let semi = load("segoe-semibold", "seguisb.ttf");
    let symbol = load("segoe-symbol", "seguisym.ttf");

    let defaults = fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    let mut prop = Vec::new();
    if regular {
        prop.push("segoe".to_string());
    }
    if symbol {
        prop.push("segoe-symbol".to_string());
    }
    prop.extend(defaults.iter().cloned());
    fonts.families.insert(FontFamily::Proportional, prop.clone());

    let mut semis = Vec::new();
    if semi {
        semis.push("segoe-semibold".to_string());
    }
    semis.extend(prop);
    fonts.families.insert(FontFamily::Name("semibold".into()), semis);
    ctx.set_fonts(fonts);
}

pub fn apply(ctx: &egui::Context, p: &Palette) {
    let mut v = if p.dark { egui::Visuals::dark() } else { egui::Visuals::light() };
    let r8 = CornerRadius::same(8);
    v.override_text_color = Some(p.text);
    v.window_fill = p.card;
    v.panel_fill = p.bg;
    v.extreme_bg_color = p.field;
    v.faint_bg_color = p.stripe;
    v.code_bg_color = p.field;
    v.hyperlink_color = p.accent;
    v.selection.bg_fill = p.select;
    v.selection.stroke = Stroke::new(1.0, p.text);
    v.window_stroke = Stroke::new(1.0, p.border);
    v.window_corner_radius = CornerRadius::same(10);
    v.menu_corner_radius = CornerRadius::same(10);
    v.popup_shadow = egui::epaint::Shadow { offset: [0, 4], blur: 16, spread: 0, color: Color32::from_black_alpha(if p.dark { 120 } else { 40 }) };
    v.text_cursor.stroke = Stroke::new(1.5, p.accent);
    v.striped = true;

    let w = &mut v.widgets;
    w.noninteractive.bg_fill = p.card;
    w.noninteractive.weak_bg_fill = p.card;
    w.noninteractive.bg_stroke = Stroke::new(1.0, p.border);
    w.noninteractive.fg_stroke = Stroke::new(1.0, p.text);
    w.noninteractive.corner_radius = r8;

    w.inactive.bg_fill = p.field;
    w.inactive.weak_bg_fill = p.button;
    w.inactive.bg_stroke = Stroke::new(1.0, p.field_line);
    w.inactive.fg_stroke = Stroke::new(1.0, p.text);
    w.inactive.corner_radius = r8;
    w.inactive.expansion = 0.0;

    w.hovered.bg_fill = p.field_hover;
    w.hovered.weak_bg_fill = p.button_hover;
    w.hovered.bg_stroke = Stroke::new(1.0, p.accent);
    w.hovered.fg_stroke = Stroke::new(1.0, p.text);
    w.hovered.corner_radius = r8;
    w.hovered.expansion = 0.0;

    w.active.bg_fill = p.button_down;
    w.active.weak_bg_fill = p.button_down;
    w.active.bg_stroke = Stroke::new(1.0, p.accent);
    w.active.fg_stroke = Stroke::new(1.0, p.text);
    w.active.corner_radius = r8;
    w.active.expansion = 0.0;

    w.open.bg_fill = p.field;
    w.open.weak_bg_fill = p.button_hover;
    w.open.bg_stroke = Stroke::new(1.0, p.accent);
    w.open.fg_stroke = Stroke::new(1.0, p.text);
    w.open.corner_radius = r8;
    ctx.set_visuals(v);

    ctx.style_mut(|s| {
        s.text_styles.insert(TextStyle::Body, FontId::proportional(13.5));
        s.text_styles.insert(TextStyle::Button, FontId::proportional(13.5));
        s.text_styles.insert(TextStyle::Small, FontId::proportional(11.5));
        s.text_styles.insert(TextStyle::Heading, semibold(16.5));
        s.spacing.button_padding = egui::vec2(12.0, 6.0);
        s.spacing.item_spacing = egui::vec2(8.0, 6.0);
        s.spacing.interact_size = egui::vec2(40.0, 28.0);
        s.spacing.combo_height = 300.0;
        s.spacing.scroll = egui::style::ScrollStyle::thin();
        s.interaction.selectable_labels = false;
    });
}

/// An on/off switch with a label (instead of a tick box).
pub fn switch(ui: &mut egui::Ui, p: &Palette, on: &mut bool, label: &str) -> egui::Response {
    let size = egui::vec2(34.0, 20.0);
    let text = egui::WidgetText::from(label).into_galley(ui, None, f32::INFINITY, TextStyle::Body);
    let total = egui::vec2(size.x + 8.0 + text.size().x, size.y.max(text.size().y));
    let (rect, mut resp) = ui.allocate_exact_size(total, egui::Sense::click());
    if resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    if ui.is_rect_visible(rect) {
        let how_on = ui.ctx().animate_bool_responsive(resp.id, *on);
        let track = egui::Rect::from_min_size(egui::pos2(rect.left(), rect.center().y - size.y / 2.0), size);
        let enabled = ui.is_enabled();
        let colour = if !enabled {
            p.border
        } else {
            lerp_colour(p.switch_off, p.accent, how_on)
        };
        let painter = ui.painter();
        painter.rect_filled(track, CornerRadius::same(10), colour);
        let x = egui::lerp((track.left() + 10.0)..=(track.right() - 10.0), how_on);
        painter.circle_filled(egui::pos2(x, track.center().y), 7.0, Color32::WHITE);
        let text_pos = egui::pos2(track.right() + 8.0, rect.center().y - text.size().y / 2.0);
        painter.galley(text_pos, text, if enabled { p.text } else { p.disabled });
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

pub fn lerp_colour(a: Color32, b: Color32, t: f32) -> Color32 {
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()))
}

/// A rounded panel.
pub fn card(p: &Palette) -> egui::Frame {
    egui::Frame::new()
        .fill(p.card)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(CornerRadius::same(12))
        .inner_margin(egui::Margin::same(14))
}

/// Decode one of the bundled PNG pictures.
pub fn decode_png(bytes: &[u8]) -> Option<egui::ColorImage> {
    let decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    let mut reader = decoder.read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).ok()?;
    let (w, h) = (info.width as usize, info.height as usize);
    let data = &buf[..info.buffer_size()];
    let rgba: Vec<u8> = match info.color_type {
        png::ColorType::Rgba => data.to_vec(),
        png::ColorType::Rgb => data.chunks(3).flat_map(|c| [c[0], c[1], c[2], 255]).collect(),
        png::ColorType::GrayscaleAlpha => data.chunks(2).flat_map(|c| [c[0], c[0], c[0], c[1]]).collect(),
        png::ColorType::Grayscale => data.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        _ => return None,
    };
    if info.bit_depth != png::BitDepth::Eight {
        return None;
    }
    Some(egui::ColorImage::from_rgba_unmultiplied([w, h], &rgba))
}
