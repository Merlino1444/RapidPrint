//! Install / remove RapidPrint for the current Windows user (no admin rights).
//!
//! Installing copies the program to %LOCALAPPDATA%\Programs\RapidPrint and adds a
//! Start menu shortcut, a desktop shortcut, a "Send to > RapidPrint" entry and an
//! entry under Settings > Apps. It also clears an older RapidPrint in the same folder
//! (the earlier build carried its own Python runtime) and, when the edition names
//! one (brand::LEGACY_INSTALL), a copy installed under an earlier name.
//!
//! For testing, set RAPIDPRINT_TEST_ROOT to a folder: every location is then made
//! inside that folder and the registry is left alone.

use crate::common::APP_NAME;
use crate::win::{self, wide, GUID};
use std::ffi::c_void;
use std::path::{Path, PathBuf};

pub const APP_ID: &str = "RapidPrint";
const EXE_NAME: &str = "RapidPrint.exe";
const VERSION: &str = env!("CARGO_PKG_VERSION");
use crate::brand::PUBLISHER;
const UNINSTALL_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall\RapidPrint";
/// Files that travel with the program.
const COMPANIONS: [&str; 2] = ["pdfium.dll", "PDFium-LICENSE.txt"];
use crate::brand::ICON_ICO as ICON;

fn test_root() -> Option<PathBuf> {
    std::env::var_os("RAPIDPRINT_TEST_ROOT").map(PathBuf::from)
}

pub fn exe() -> PathBuf {
    std::env::current_exe().unwrap_or_else(|_| PathBuf::from(EXE_NAME))
}

pub fn exe_dir() -> PathBuf {
    exe().parent().map(Path::to_path_buf).unwrap_or_default()
}

fn same(a: &Path, b: &Path) -> bool {
    let n = |p: &Path| p.to_string_lossy().trim_end_matches(['\\', '/']).to_lowercase().replace('/', "\\");
    n(a) == n(b)
}

/// A build made while developing (inside the project's target folder) never offers to install.
pub fn is_dev_build() -> bool {
    let d = exe_dir().to_string_lossy().to_lowercase();
    d.contains("\\target\\debug") || d.contains("\\target\\release")
}

fn known_folder(guid: &str) -> Option<PathBuf> {
    let sh = win::shell()?;
    let g = GUID::parse(guid);
    let mut p: *mut u16 = std::ptr::null_mut();
    unsafe {
        if (sh.SHGetKnownFolderPath)(&g, 0, std::ptr::null_mut(), &mut p) != 0 {
            return None;
        }
        let s = win::from_wide(p);
        if let Some(o) = win::ole() {
            (o.CoTaskMemFree)(p as *mut c_void);
        }
        Some(PathBuf::from(s))
    }
}

pub struct Locations {
    pub install: PathBuf,
    pub start: PathBuf,
    pub desktop: PathBuf,
    pub sendto: PathBuf,
}

pub fn locations() -> Locations {
    if let Some(r) = test_root() {
        return Locations {
            install: r.join("Programs").join(APP_NAME),
            start: r.join("StartMenu"),
            desktop: r.join("Desktop"),
            sendto: r.join("SendTo"),
        };
    }
    let home = std::env::var_os("USERPROFILE").map(PathBuf::from).unwrap_or_default();
    let local = std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(|| home.join(r"AppData\Local"));
    let roaming = std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(|| home.join(r"AppData\Roaming"));
    Locations {
        install: local.join("Programs").join(APP_NAME),
        start: known_folder("A77F5D77-2E2B-44C3-A6A2-ABA601054A51")
            .unwrap_or_else(|| roaming.join(r"Microsoft\Windows\Start Menu\Programs")),
        desktop: known_folder("B4BFCC3A-DB2C-424C-B029-7FE99A87C641").unwrap_or_else(|| home.join("Desktop")),
        sendto: known_folder("8983036C-27C0-404B-8F08-102D10DCFD74")
            .unwrap_or_else(|| roaming.join(r"Microsoft\Windows\SendTo")),
    }
}

fn shortcut_files(loc: &Locations, name: &str) -> [PathBuf; 3] {
    let f = format!("{name}.lnk");
    [loc.start.join(&f), loc.desktop.join(&f), loc.sendto.join(&f)]
}

pub fn installed_here() -> bool {
    same(&exe_dir(), &locations().install)
}

/// Installed already (this Rust version, or the older Python-based one).
pub fn is_installed() -> bool {
    locations().install.join(EXE_NAME).is_file()
}

fn legacy_folder(loc: &Locations, name: &str) -> PathBuf {
    loc.install.parent().map(|p| p.join(name)).unwrap_or_default()
}

/// The earlier name, when a copy under that name is installed.
pub fn legacy_installed() -> Option<&'static str> {
    let (name, _) = crate::brand::LEGACY_INSTALL?;
    legacy_folder(&locations(), name).join(format!("{name}.exe")).is_file().then_some(name)
}

// --------------------------------------------------------------------------
// Shortcuts (.lnk) through the Windows Shell
// --------------------------------------------------------------------------

#[repr(C)]
struct PropertyKey {
    fmtid: GUID,
    pid: u32,
}

#[repr(C)]
struct PropVariant {
    vt: u16,
    r1: u16,
    r2: u16,
    r3: u16,
    ptr: *const u16,
    pad: usize,
}

fn check(hr: i32, what: &str) -> Result<(), String> {
    if hr < 0 {
        Err(format!("{what} failed (0x{:08X})", hr as u32))
    } else {
        Ok(())
    }
}

pub fn make_shortcut(lnk: &Path, target: &Path, workdir: &Path, icon: &Path, description: &str) -> Result<(), String> {
    type Hr1 = unsafe extern "system" fn(*mut c_void, *const u16) -> i32;
    type HrIcon = unsafe extern "system" fn(*mut c_void, *const u16, i32) -> i32;
    type Qi = unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> i32;
    type SetValue = unsafe extern "system" fn(*mut c_void, *const PropertyKey, *const PropVariant) -> i32;
    type Commit = unsafe extern "system" fn(*mut c_void) -> i32;
    type Save = unsafe extern "system" fn(*mut c_void, *const u16, i32) -> i32;

    win::com_init();
    let ole = win::ole().ok_or("COM is not available")?;
    let mut link: *mut c_void = std::ptr::null_mut();
    unsafe {
        check(
            (ole.CoCreateInstance)(&GUID::parse("00021401-0000-0000-C000-000000000046"), std::ptr::null_mut(), 1,
                                   &GUID::parse("000214F9-0000-0000-C000-000000000046"), &mut link),
            "creating a shortcut",
        )?;
        let result = (|| -> Result<(), String> {
            let t = wide(&target.to_string_lossy());
            check(win::method::<Hr1>(link, 20)(link, t.as_ptr()), "setting the shortcut target")?; // SetPath
            let w = wide(&workdir.to_string_lossy());
            win::method::<Hr1>(link, 9)(link, w.as_ptr()); // SetWorkingDirectory
            let i = wide(&icon.to_string_lossy());
            win::method::<HrIcon>(link, 17)(link, i.as_ptr(), 0); // SetIconLocation
            let d = wide(description);
            win::method::<Hr1>(link, 7)(link, d.as_ptr()); // SetDescription

            // Same AppUserModelID as the running window, so the taskbar groups it with our icon.
            let mut store: *mut c_void = std::ptr::null_mut();
            if win::method::<Qi>(link, 0)(link, &GUID::parse("886D8EEB-8CF2-4446-8D02-CDBA1DBDCF99"), &mut store) >= 0
                && !store.is_null()
            {
                let key = PropertyKey { fmtid: GUID::parse("9F4C2855-9F79-4B39-A8D0-E1D42DE1D5F3"), pid: 5 };
                let id = wide(APP_ID);
                let pv = PropVariant { vt: 31, r1: 0, r2: 0, r3: 0, ptr: id.as_ptr(), pad: 0 }; // VT_LPWSTR
                if win::method::<SetValue>(store, 6)(store, &key, &pv) >= 0 {
                    win::method::<Commit>(store, 7)(store);
                }
                win::release(store);
            }

            let mut pf: *mut c_void = std::ptr::null_mut();
            check(win::method::<Qi>(link, 0)(link, &GUID::parse("0000010B-0000-0000-C000-000000000046"), &mut pf),
                  "saving the shortcut")?;
            if let Some(parent) = lnk.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let l = wide(&lnk.to_string_lossy());
            let hr = win::method::<Save>(pf, 6)(pf, l.as_ptr(), 1);
            win::release(pf);
            check(hr, "saving the shortcut")
        })();
        win::release(link);
        result
    }
}

fn notify_shell() {
    if let Some(sh) = win::shell() {
        unsafe { (sh.SHChangeNotify)(0x0800_0000, 0, std::ptr::null(), std::ptr::null()) }
    }
}

// --------------------------------------------------------------------------
// Install / uninstall
// --------------------------------------------------------------------------

fn folder_size_kb(folder: &Path) -> u32 {
    fn walk(p: &Path) -> u64 {
        std::fs::read_dir(p)
            .map(|rd| {
                rd.filter_map(|e| e.ok())
                    .map(|e| {
                        let p = e.path();
                        if p.is_dir() {
                            walk(&p)
                        } else {
                            e.metadata().map(|m| m.len()).unwrap_or(0)
                        }
                    })
                    .sum()
            })
            .unwrap_or(0)
    }
    (walk(folder) / 1024) as u32
}

fn register(folder: &Path) -> Result<(), String> {
    if test_root().is_some() {
        return Ok(());
    }
    use winreg::enums::HKEY_CURRENT_USER;
    let (k, _) = winreg::RegKey::predef(HKEY_CURRENT_USER).create_subkey(UNINSTALL_KEY).map_err(|e| e.to_string())?;
    let exe = folder.join(EXE_NAME);
    let s = |n: &str, v: &str| k.set_value(n, &v.to_string());
    let _ = s("DisplayName", APP_NAME);
    let _ = s("DisplayVersion", VERSION);
    let _ = s("Publisher", PUBLISHER);
    let _ = s("DisplayIcon", &folder.join("icon.ico").to_string_lossy());
    let _ = s("InstallLocation", &folder.to_string_lossy());
    let _ = s("UninstallString", &format!("\"{}\" --uninstall", exe.to_string_lossy()));
    let _ = k.set_value("EstimatedSize", &folder_size_kb(folder));
    let _ = k.set_value("NoModify", &1u32);
    let _ = k.set_value("NoRepair", &1u32);
    Ok(())
}

fn unregister(key: &str) {
    if test_root().is_some() {
        return;
    }
    let _ = winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER).delete_subkey_all(key);
}

/// Remove a copy installed under an earlier name.
fn remove_legacy(loc: &Locations) {
    let Some((name, key)) = crate::brand::LEGACY_INSTALL else { return };
    for lnk in shortcut_files(loc, name) {
        let _ = std::fs::remove_file(lnk);
    }
    unregister(key);
    let old = legacy_folder(loc, name);
    if old.is_dir() && !same(&exe_dir(), &old) {
        let _ = std::fs::remove_dir_all(old);
    }
}

/// Copy the program to the user's Programs folder and add the shortcuts.
pub fn install(desktop: bool) -> Result<PathBuf, String> {
    let loc = locations();
    let dst = loc.install.clone();
    let src = exe_dir();
    if !same(&src, &dst) {
        // An older RapidPrint that carried its own Python runtime: clear it first.
        if dst.join(format!("{APP_NAME}._pth")).is_file() {
            let _ = std::fs::remove_dir_all(&dst);
        }
        std::fs::create_dir_all(&dst).map_err(|e| e.to_string())?;
        std::fs::copy(exe(), dst.join(EXE_NAME)).map_err(|e| format!("copying the program: {e}"))?;
        for f in COMPANIONS {
            let from = crate::pdfium::dll_dir().map(|d| d.join(f)).unwrap_or_else(|| src.join(f));
            if from.is_file() {
                std::fs::copy(&from, dst.join(f)).map_err(|e| format!("copying {f}: {e}"))?;
            }
        }
    }
    let icon = dst.join("icon.ico");
    let _ = std::fs::write(&icon, ICON);
    let exe = dst.join(EXE_NAME);
    let [start, desk, sendto] = shortcut_files(&loc, APP_NAME);
    let desc = "Print many PDF files at once";
    make_shortcut(&start, &exe, &dst, &icon, desc)?;
    if desktop {
        make_shortcut(&desk, &exe, &dst, &icon, desc)?;
    }
    make_shortcut(&sendto, &exe, &dst, &icon, "Add to RapidPrint")?;
    register(&dst)?;
    remove_legacy(&loc);
    notify_shell();
    Ok(exe)
}

/// Remove shortcuts, the Settings > Apps entry and the installed folder.
pub fn uninstall() {
    let loc = locations();
    for lnk in shortcut_files(&loc, APP_NAME) {
        let _ = std::fs::remove_file(lnk);
    }
    unregister(UNINSTALL_KEY);
    notify_shell();
    let folder = loc.install;
    if !folder.is_dir() {
        return;
    }
    if same(&exe_dir(), &folder) {
        // Running from that folder: delete it once we have exited (a few tries).
        let comspec = std::env::var("COMSPEC").unwrap_or_else(|_| r"C:\Windows\System32\cmd.exe".into());
        let f = folder.to_string_lossy().replace('"', "");
        let cmd = format!(
            "for /l %i in (1,1,15) do @if exist \"{f}\" (ping -n 2 127.0.0.1 >nul & rmdir /s /q \"{f}\" 2>nul)"
        );
        use std::os::windows::process::CommandExt;
        let _ = std::process::Command::new(comspec)
            .raw_arg("/d /c")
            .raw_arg(cmd)
            .current_dir(std::env::temp_dir())
            .creation_flags(0x0800_0000 | 0x0000_0008) // no window, detached
            .spawn();
    } else {
        let _ = std::fs::remove_dir_all(folder);
    }
}

pub fn uninstall_with_dialogs() {
    use rfd::{MessageButtons, MessageDialog, MessageDialogResult, MessageLevel};
    let yes = MessageDialog::new()
        .set_title(APP_NAME)
        .set_description("Remove RapidPrint from this computer?\n\nYour PDF files are not touched.")
        .set_buttons(MessageButtons::YesNo)
        .set_level(MessageLevel::Info)
        .show();
    if yes == MessageDialogResult::Yes {
        uninstall();
        MessageDialog::new().set_title(APP_NAME).set_description("RapidPrint has been removed.").show();
    }
}
