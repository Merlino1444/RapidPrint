//! RapidPrint — print many PDF files at once, only your own pages.
//!
//!   RapidPrint [files or folders...]
//!   RapidPrint --screenshot <file.png>   draw ~120 frames, save the window's picture, exit
//!   RapidPrint --install | --uninstall

#![windows_subsystem = "windows"]

mod app;
mod brand;
mod common;
mod installer;
mod pdfium;
mod printing;
mod theme;
mod win;

use eframe::egui;
use std::path::{Path, PathBuf};

/// Save an egui picture as a PNG file.
pub fn save_png(path: &Path, img: &egui::ColorImage) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        if !dir.as_os_str().is_empty() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
    }
    let file = std::fs::File::create(path).map_err(|e| e.to_string())?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), img.size[0] as u32, img.size[1] as u32);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut w = enc.write_header().map_err(|e| e.to_string())?;
    let data: Vec<u8> = img.pixels.iter().flat_map(|c| c.to_srgba_unmultiplied()).collect();
    w.write_image_data(&data).map_err(|e| e.to_string())
}

fn main() -> eframe::Result<()> {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--uninstall") {
        win::com_init();
        installer::uninstall_with_dialogs();
        return Ok(());
    }
    if args.iter().any(|a| a == "--install") {
        win::com_init();
        let _ = installer::install(!args.iter().any(|a| a == "--no-desktop"));
        return Ok(());
    }
    let mut shot: Option<PathBuf> = std::env::var_os("RAPIDPRINT_SCREENSHOT").map(PathBuf::from);
    if let Some(i) = args.iter().position(|a| a == "--screenshot") {
        shot = Some(PathBuf::from(args.get(i + 1).cloned().unwrap_or_else(|| "screenshot.png".into())));
        let end = (i + 2).min(args.len());
        args.drain(i..end);
    }
    let paths: Vec<PathBuf> = args.into_iter().filter(|a| !a.starts_with("--")).map(PathBuf::from).collect();

    if let Some(sh) = win::shell() {
        // Own taskbar button and icon (matches the shortcuts).
        unsafe {
            (sh.SetCurrentProcessExplicitAppUserModelID)(win::wide(installer::APP_ID).as_ptr());
        }
    }

    let mut viewport = egui::ViewportBuilder::default()
        .with_title(common::APP_NAME)
        .with_inner_size([1100.0, 880.0])
        .with_min_inner_size([920.0, 780.0])
        .with_drag_and_drop(true);
    if let Some(img) = theme::decode_png(brand::ICON_PNG) {
        let rgba: Vec<u8> = img.pixels.iter().flat_map(|c| c.to_srgba_unmultiplied()).collect();
        viewport = viewport.with_icon(egui::IconData { rgba, width: img.size[0] as u32, height: img.size[1] as u32 });
    }
    if shot.is_some() {
        // Open without taking focus and behind other windows, so a check never
        // gets in the way of the person; a covered window still draws on Windows.
        viewport = viewport.with_active(false).with_window_level(egui::WindowLevel::AlwaysOnBottom);
    }
    let options = eframe::NativeOptions { viewport, centered: true, ..Default::default() };
    let shot = shot.map(|path| app::Shot { path, frames: 0, asked: false });
    eframe::run_native(common::APP_NAME, options, Box::new(move |cc| Ok(Box::new(app::App::new(cc, paths, shot)))))
}
