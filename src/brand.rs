//! The name, look, icon and default archive rules of this build
//! (see brand_public.rs).

#[path = "brand_public.rs"]
mod edition;

pub use edition::*;

use eframe::egui::Color32;

/// The accent colours of one look (light or dark).
#[derive(Clone, Copy)]
pub struct Accents {
    pub accent: Color32,
    pub accent_dark: Color32,
    pub accent_soft: Color32,
    pub accent_off: Color32,
    pub select: Color32,
}

pub const fn hex(v: u32) -> Color32 {
    Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

/// A rule the archive starts with: (how to match, text, folder key in old settings).
pub type DefaultRule = (&'static str, &'static str, &'static str);
