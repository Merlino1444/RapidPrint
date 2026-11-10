//! Small doorway to the Windows functions RapidPrint needs (printers, printing,
//! Print Tickets, shortcuts). They are looked up in the system DLLs when first
//! used, so nothing extra has to be linked in.

#![allow(non_snake_case, clippy::upper_case_acronyms)]

use libloading::Library;
use std::ffi::c_void;
use std::sync::OnceLock;

pub type HANDLE = *mut c_void;
pub type PCWSTR = *const u16;
pub type PWSTR = *mut u16;

macro_rules! dll {
    ($name:ident, $getter:ident, $file:expr, { $( fn $f:ident ( $($t:ty),* ) -> $r:ty ; )* }) => {
        pub struct $name {
            _lib: Library,
            $( pub $f: unsafe extern "system" fn($($t),*) -> $r, )*
        }
        unsafe impl Sync for $name {}
        unsafe impl Send for $name {}
        impl $name {
            fn load() -> Option<Self> {
                unsafe {
                    let lib = Library::new($file).ok()?;
                    $( let $f = *lib.get::<unsafe extern "system" fn($($t),*) -> $r>(
                        concat!(stringify!($f), "\0").as_bytes()).ok()?; )*
                    Some(Self { _lib: lib, $($f,)* })
                }
            }
        }
        pub fn $getter() -> Option<&'static $name> {
            static CELL: OnceLock<Option<$name>> = OnceLock::new();
            CELL.get_or_init($name::load).as_ref()
        }
    };
}

dll!(Winspool, winspool, "winspool.drv", {
    fn OpenPrinterW(PCWSTR, *mut HANDLE, *mut c_void) -> i32;
    fn ClosePrinter(HANDLE) -> i32;
    fn DocumentPropertiesW(isize, HANDLE, PCWSTR, *mut u8, *const u8, u32) -> i32;
    fn DeviceCapabilitiesW(PCWSTR, PCWSTR, u16, *mut c_void, *const u8) -> i32;
    fn GetPrinterW(HANDLE, u32, *mut u8, u32, *mut u32) -> i32;
    fn EnumPrintersW(u32, PCWSTR, u32, *mut u8, u32, *mut u32, *mut u32) -> i32;
    fn GetDefaultPrinterW(PWSTR, *mut u32) -> i32;
});

#[repr(C)]
pub struct DOCINFOW {
    pub cbSize: i32,
    pub lpszDocName: PCWSTR,
    pub lpszOutput: PCWSTR,
    pub lpszDatatype: PCWSTR,
    pub fwType: u32,
}

dll!(Gdi, gdi, "gdi32.dll", {
    fn CreateDCW(PCWSTR, PCWSTR, PCWSTR, *const u8) -> HANDLE;
    fn DeleteDC(HANDLE) -> i32;
    fn GetDeviceCaps(HANDLE, i32) -> i32;
    fn StartDocW(HANDLE, *const DOCINFOW) -> i32;
    fn StartPage(HANDLE) -> i32;
    fn EndPage(HANDLE) -> i32;
    fn EndDoc(HANDLE) -> i32;
    fn AbortDoc(HANDLE) -> i32;
});

dll!(Ole, ole, "ole32.dll", {
    fn CoInitializeEx(*mut c_void, u32) -> i32;
    fn CreateStreamOnHGlobal(*mut c_void, i32, *mut *mut c_void) -> i32;
    fn CoCreateInstance(*const GUID, *mut c_void, u32, *const GUID, *mut *mut c_void) -> i32;
    fn CoTaskMemFree(*mut c_void) -> ();
});

dll!(OleAut, oleaut, "oleaut32.dll", {
    fn SysFreeString(*mut u16) -> ();
});

dll!(PrintTicket, prntvpt, "prntvpt.dll", {
    fn PTOpenProvider(PCWSTR, u32, *mut HANDLE) -> i32;
    fn PTCloseProvider(HANDLE) -> i32;
    fn PTGetPrintCapabilities(HANDLE, *mut c_void, *mut c_void, *mut *mut u16) -> i32;
    fn PTConvertDevModeToPrintTicket(HANDLE, u32, *const u8, i32, *mut c_void) -> i32;
    fn PTMergeAndValidatePrintTicket(HANDLE, *mut c_void, *mut c_void, i32, *mut c_void, *mut *mut u16) -> i32;
    fn PTConvertPrintTicketToDevMode(HANDLE, *mut c_void, i32, i32, *mut u32, *mut *mut u8, *mut *mut u16) -> i32;
    fn PTReleaseMemory(*mut c_void) -> i32;
});

dll!(Shell, shell, "shell32.dll", {
    fn SHGetKnownFolderPath(*const GUID, u32, HANDLE, *mut PWSTR) -> i32;
    fn SHChangeNotify(i32, u32, *const c_void, *const c_void) -> ();
    fn SetCurrentProcessExplicitAppUserModelID(PCWSTR) -> i32;
});

// --------------------------------------------------------------------------
// Helpers
// --------------------------------------------------------------------------

/// A Rust string as a zero-terminated wide string.
pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// A zero-terminated wide string back to Rust.
pub unsafe fn from_wide(p: *const u16) -> String {
    if p.is_null() {
        return String::new();
    }
    let mut n = 0;
    while *p.add(n) != 0 {
        n += 1;
    }
    String::from_utf16_lossy(std::slice::from_raw_parts(p, n))
}

/// Up to `max` wide characters, stopping at the first zero.
pub fn wide_field(buf: &[u16]) -> String {
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end])
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct GUID {
    pub d1: u32,
    pub d2: u16,
    pub d3: u16,
    pub d4: [u8; 8],
}

impl GUID {
    /// "00021401-0000-0000-C000-000000000046"
    pub fn parse(s: &str) -> GUID {
        let h: String = s.chars().filter(|c| c.is_ascii_hexdigit()).collect();
        let byte = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).unwrap_or(0);
        let mut d4 = [0u8; 8];
        for (k, b) in d4.iter_mut().enumerate() {
            *b = byte(16 + k * 2);
        }
        GUID {
            d1: u32::from_str_radix(&h[0..8], 16).unwrap_or(0),
            d2: u16::from_str_radix(&h[8..12], 16).unwrap_or(0),
            d3: u16::from_str_radix(&h[12..16], 16).unwrap_or(0),
            d4,
        }
    }
}

/// Prepare COM on this thread (needed by the Print Ticket and shortcut parts).
pub fn com_init() {
    if let Some(o) = ole() {
        unsafe {
            (o.CoInitializeEx)(std::ptr::null_mut(), 0x2);
        }
    }
}

/// The function at slot `index` of a COM object's table.
pub unsafe fn method<F: Copy>(obj: *mut c_void, index: usize) -> F {
    let vtbl = *(obj as *const *const *const c_void);
    let f = *vtbl.add(index);
    std::mem::transmute_copy::<*const c_void, F>(&f)
}

pub unsafe fn release(obj: *mut c_void) {
    if !obj.is_null() {
        let f: unsafe extern "system" fn(*mut c_void) -> u32 = method(obj, 2);
        f(obj);
    }
}

pub unsafe fn take_bstr(b: *mut u16) -> String {
    if b.is_null() {
        return String::new();
    }
    let s = from_wide(b);
    if let Some(o) = oleaut() {
        (o.SysFreeString)(b);
    }
    s
}

/// An in-memory IStream, used to hand XML to and from the Print Ticket API.
pub struct Stream(pub *mut c_void);

impl Stream {
    pub fn new(data: &[u8]) -> Result<Stream, String> {
        let o = ole().ok_or("COM is not available")?;
        let mut p = std::ptr::null_mut();
        let hr = unsafe { (o.CreateStreamOnHGlobal)(std::ptr::null_mut(), 1, &mut p) };
        if hr < 0 || p.is_null() {
            return Err("could not create a memory stream".into());
        }
        let s = Stream(p);
        if !data.is_empty() {
            unsafe {
                let write: unsafe extern "system" fn(*mut c_void, *const u8, u32, *mut u32) -> i32 = method(p, 4);
                write(p, data.as_ptr(), data.len() as u32, std::ptr::null_mut());
            }
            s.seek(0, 0);
        }
        Ok(s)
    }

    pub fn seek(&self, pos: i64, origin: u32) -> u64 {
        let mut out = 0u64;
        unsafe {
            let seek: unsafe extern "system" fn(*mut c_void, i64, u32, *mut u64) -> i32 = method(self.0, 5);
            seek(self.0, pos, origin, &mut out);
        }
        out
    }

    pub fn read_all(&self) -> Vec<u8> {
        let size = self.seek(0, 2) as usize;
        self.seek(0, 0);
        let mut buf = vec![0u8; size];
        let mut got = 0u32;
        unsafe {
            let read: unsafe extern "system" fn(*mut c_void, *mut u8, u32, *mut u32) -> i32 = method(self.0, 3);
            read(self.0, buf.as_mut_ptr(), size as u32, &mut got);
        }
        buf.truncate(got as usize);
        buf
    }
}

impl Drop for Stream {
    fn drop(&mut self) {
        unsafe { release(self.0) }
    }
}
