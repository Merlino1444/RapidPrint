//! PDFium (the PDF engine also used by Chrome), loaded from pdfium.dll next to
//! the program. It draws PDF pages straight onto the printer, so only the
//! pages of the PDF are printed.

#![allow(non_snake_case)]

use libloading::Library;
use std::ffi::c_void;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard, OnceLock};

type Doc = *mut c_void;
type Page = *mut c_void;

pub struct Pdfium {
    _lib: Library,
    FPDF_InitLibrary: unsafe extern "C" fn(),
    FPDF_LoadMemDocument64: unsafe extern "C" fn(*const u8, usize, *const i8) -> Doc,
    FPDF_CloseDocument: unsafe extern "C" fn(Doc),
    FPDF_GetPageCount: unsafe extern "C" fn(Doc) -> i32,
    FPDF_LoadPage: unsafe extern "C" fn(Doc, i32) -> Page,
    FPDF_ClosePage: unsafe extern "C" fn(Page),
    FPDF_GetPageWidthF: unsafe extern "C" fn(Page) -> f32,
    FPDF_GetPageHeightF: unsafe extern "C" fn(Page) -> f32,
    FPDF_GetLastError: unsafe extern "C" fn() -> u32,
    FPDF_SetPrintMode: unsafe extern "C" fn(i32) -> i32,
    FPDF_RenderPage: unsafe extern "C" fn(*mut c_void, Page, i32, i32, i32, i32, i32, i32) -> i32,
}

unsafe impl Send for Pdfium {}
unsafe impl Sync for Pdfium {}

/// Where pdfium.dll may be: next to the program, or (while developing) in
/// the project's vendor folder a few levels up.
fn candidates() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        let mut dir = exe.parent().map(|p| p.to_path_buf());
        if let Some(d) = &dir {
            out.push(d.join("pdfium.dll"));
        }
        for _ in 0..5 {
            dir = dir.and_then(|d| d.parent().map(|p| p.to_path_buf()));
            if let Some(d) = &dir {
                out.push(d.join("vendor").join("pdfium.dll"));
            }
        }
    }
    out.push(PathBuf::from("vendor").join("pdfium.dll"));
    out
}

impl Pdfium {
    fn load() -> Option<Pdfium> {
        for path in candidates() {
            if !path.is_file() {
                continue;
            }
            unsafe {
                let Ok(lib) = Library::new(&path) else { continue };
                macro_rules! get {
                    ($n:ident) => {
                        match lib.get(concat!(stringify!($n), "\0").as_bytes()) {
                            Ok(f) => *f,
                            Err(_) => continue,
                        }
                    };
                }
                let p = Pdfium {
                    FPDF_InitLibrary: get!(FPDF_InitLibrary),
                    FPDF_LoadMemDocument64: get!(FPDF_LoadMemDocument64),
                    FPDF_CloseDocument: get!(FPDF_CloseDocument),
                    FPDF_GetPageCount: get!(FPDF_GetPageCount),
                    FPDF_LoadPage: get!(FPDF_LoadPage),
                    FPDF_ClosePage: get!(FPDF_ClosePage),
                    FPDF_GetPageWidthF: get!(FPDF_GetPageWidthF),
                    FPDF_GetPageHeightF: get!(FPDF_GetPageHeightF),
                    FPDF_GetLastError: get!(FPDF_GetLastError),
                    FPDF_SetPrintMode: get!(FPDF_SetPrintMode),
                    FPDF_RenderPage: get!(FPDF_RenderPage),
                    _lib: lib,
                };
                (p.FPDF_InitLibrary)();
                return Some(p);
            }
        }
        None
    }
}

/// PDFium may only be used by one thread at a time.
pub fn lock() -> Option<MutexGuard<'static, Option<Pdfium>>> {
    static CELL: OnceLock<Mutex<Option<Pdfium>>> = OnceLock::new();
    let m = CELL.get_or_init(|| Mutex::new(Pdfium::load()));
    let g = m.lock().unwrap_or_else(|e| e.into_inner());
    if g.is_some() {
        Some(g)
    } else {
        None
    }
}

pub fn available() -> bool {
    lock().is_some()
}

/// An open PDF. Keep the PDFium lock while it is used.
pub struct Document<'a> {
    pdf: &'a Pdfium,
    doc: Doc,
    _data: Vec<u8>,
}

impl<'a> Document<'a> {
    pub fn open(pdf: &'a Pdfium, path: &std::path::Path) -> Result<Document<'a>, String> {
        let data = std::fs::read(path).map_err(|_| "file can't be read".to_string())?;
        let doc = unsafe { (pdf.FPDF_LoadMemDocument64)(data.as_ptr(), data.len(), std::ptr::null()) };
        if doc.is_null() {
            let err = unsafe { (pdf.FPDF_GetLastError)() };
            return Err(if err == 4 { "password protected".into() } else { "not a readable PDF".into() });
        }
        Ok(Document { pdf, doc, _data: data })
    }

    pub fn page_count(&self) -> usize {
        unsafe { (self.pdf.FPDF_GetPageCount)(self.doc).max(0) as usize }
    }

    /// Page size in points (1/72 inch).
    pub fn page_size(&self, index: usize) -> Result<(f32, f32), String> {
        unsafe {
            let page = (self.pdf.FPDF_LoadPage)(self.doc, index as i32);
            if page.is_null() {
                return Err(format!("page {} can't be read", index + 1));
            }
            let size = ((self.pdf.FPDF_GetPageWidthF)(page), (self.pdf.FPDF_GetPageHeightF)(page));
            (self.pdf.FPDF_ClosePage)(page);
            Ok(size)
        }
    }

    /// Draw one page onto a printer page (device context).
    #[allow(clippy::too_many_arguments)]
    pub fn render(&self, index: usize, dc: *mut c_void, x: i32, y: i32, w: i32, h: i32, rotate: i32, flags: i32)
                  -> Result<(), String> {
        unsafe {
            let page = (self.pdf.FPDF_LoadPage)(self.doc, index as i32);
            if page.is_null() {
                return Err(format!("page {} can't be read", index + 1));
            }
            (self.pdf.FPDF_SetPrintMode)(0); // EMF
            (self.pdf.FPDF_RenderPage)(dc, page, x, y, w, h, rotate, flags);
            (self.pdf.FPDF_ClosePage)(page);
            Ok(())
        }
    }
}

impl Drop for Document<'_> {
    fn drop(&mut self) {
        unsafe { (self.pdf.FPDF_CloseDocument)(self.doc) }
    }
}

/// Number of pages, or None when the file can't be read.
pub fn count_pages(path: &std::path::Path) -> Option<usize> {
    let guard = lock()?;
    let pdf = guard.as_ref()?;
    Document::open(pdf, path).ok().map(|d| d.page_count())
}

/// The folder pdfium.dll is loaded from (its licence text sits next to it).
pub fn dll_dir() -> Option<PathBuf> {
    candidates().into_iter().find(|p| p.is_file()).and_then(|p| p.parent().map(|d| d.to_path_buf()))
}
