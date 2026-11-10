//! The neutral, public edition of RapidPrint.

use super::{hex, Accents, DefaultRule};

pub const PUBLISHER: &str = "RapidPrint";

/// Saved settings live in %APPDATA%\<this>\settings.json.
pub const SETTINGS_FOLDER: &str = "RapidPrint";

pub const LIGHT_ACCENTS: Accents = Accents {
    accent: hex(0x2563eb),
    accent_dark: hex(0x1d4ed8),
    accent_soft: hex(0xe8efff),
    accent_off: hex(0xa9c1f7),
    select: hex(0xdbe7ff),
};

pub const DARK_ACCENTS: Accents = Accents {
    accent: hex(0x4f86f7),
    accent_dark: hex(0x3b73e6),
    accent_soft: hex(0x1d2a44),
    accent_off: hex(0x2c3d66),
    select: hex(0x263a5e),
};

/// Logo shown in the top bar (light look, dark look). None: a drawn printer badge.
pub const LOGO_LIGHT: Option<&[u8]> = None;
pub const LOGO_DARK: Option<&[u8]> = None;

pub const ICON_PNG: &[u8] = include_bytes!("../assets/icon.png");
pub const ICON_ICO: &[u8] = include_bytes!("../assets/icon.ico");

/// The archive starts empty: everyone adds their own rules.
pub const DEFAULT_RULES: &[DefaultRule] = &[];

/// An earlier install to remove when installing (name, Settings > Apps key). None here.
pub const LEGACY_INSTALL: Option<(&str, &str)> = None;
