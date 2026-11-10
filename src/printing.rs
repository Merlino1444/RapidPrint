//! Sends PDF pages straight to a Windows printer.
//!
//! Printer settings are written into the printer's own settings block
//! (DEVMODE). Stapling and hole punching have no plain DEVMODE field, so they
//! are set through the Windows Print Ticket API, which modern drivers support.

use crate::pdfium;
use crate::win::{self, wide, GUID, HANDLE};
use std::path::Path;

pub const AS_PRINTER: &str = "As set on printer";
pub const TWO_SIDED: [(&str, Option<i16>); 4] = [
    (AS_PRINTER, None),
    ("Off (one side)", Some(1)),
    ("Long edge (book)", Some(2)),
    ("Short edge (flip up)", Some(3)),
];
pub const COLOUR: [(&str, Option<i16>); 3] = [(AS_PRINTER, None), ("Colour", Some(2)), ("Black & white", Some(1))];
pub const ORIENTATION: [(&str, i16); 3] = [("Automatic", 0), ("Portrait", 1), ("Landscape", 2)];
pub const FIT: [(&str, Fit); 3] = [
    ("Fit to page", Fit::Fit),
    ("Shrink only if too big", Fit::Shrink),
    ("Actual size", Fit::Actual),
];

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Fit {
    Fit,
    Shrink,
    Actual,
}

const NICE_FINISHING: [(&str, &str); 14] = [
    ("None", "Off"),
    ("StapleTopLeft", "1 staple, top left"),
    ("StapleTopRight", "1 staple, top right"),
    ("StapleBottomLeft", "1 staple, bottom left"),
    ("StapleBottomRight", "1 staple, bottom right"),
    ("StapleDualLeft", "2 staples, left side"),
    ("StapleDualRight", "2 staples, right side"),
    ("StapleDualTop", "2 staples, top"),
    ("StapleDualBottom", "2 staples, bottom"),
    ("SaddleStitch", "Booklet (saddle stitch)"),
    ("LeftEdge", "Left edge"),
    ("RightEdge", "Right edge"),
    ("TopEdge", "Top edge"),
    ("BottomEdge", "Bottom edge"),
];
const STAPLE_FEATURES: [&str; 2] = ["JobStapleAllDocuments", "DocumentStaple"];
const PUNCH_FEATURES: [&str; 2] = ["JobHolePunch", "DocumentHolePunch"];

const PSF: &str = "http://schemas.microsoft.com/windows/2003/08/printing/printschemaframework";
const PSK: &str = "http://schemas.microsoft.com/windows/2003/08/printing/printschemakeywords";

// DEVMODEW offsets and flags
pub const OFF_COPIES: usize = 86;
const OFF_SIZE: usize = 68;
const OFF_EXTRA: usize = 70;
const OFF_FIELDS: usize = 72;
const OFF_ORIENT: usize = 76;
const OFF_PAPER: usize = 78;
const OFF_SOURCE: usize = 88;
const OFF_COLOR: usize = 92;
const OFF_DUPLEX: usize = 94;
const OFF_COLLATE: usize = 100;
const DM_ORIENTATION: u32 = 0x1;
const DM_PAPERSIZE: u32 = 0x2;
const DM_COPIES: u32 = 0x100;
const DM_DEFAULTSOURCE: u32 = 0x200;
const DM_COLOR: u32 = 0x800;
const DM_DUPLEX: u32 = 0x1000;
const DM_COLLATE: u32 = 0x8000;

#[derive(Clone, Default, Debug)]
pub struct Finishing {
    pub feature: String,
    pub options: Vec<(String, String)>, // (option qname, label)
    pub ns: Vec<(String, String)>,      // prefix -> uri, from the capabilities
}

#[derive(Clone, Debug)]
pub struct Caps {
    pub papers: Vec<(String, u16)>,
    pub bins: Vec<(String, u16)>,
    pub duplex: bool,
    pub max_copies: i32,
    pub staple: Option<Finishing>,
    pub punch: Option<Finishing>,
    pub error: String,
}

impl Default for Caps {
    fn default() -> Self {
        Caps { papers: vec![], bins: vec![], duplex: true, max_copies: 999, staple: None, punch: None, error: String::new() }
    }
}

#[derive(Clone, Debug)]
pub struct JobOptions {
    pub copies: u32,
    pub driver_copies: bool,
    pub duplex: Option<i16>,
    pub colour: Option<i16>,
    pub orientation: i16,
    pub fit: Fit,
    pub pages: String,
    pub paper: Option<u16>,
    pub bin: Option<u16>,
    pub staple: Option<String>,
    pub punch: Option<String>,
}

// --------------------------------------------------------------------------
// Printers and their settings blocks (DEVMODE)
// --------------------------------------------------------------------------

/// (printer names, default printer)
pub fn list_printers() -> (Vec<String>, String) {
    let Some(ws) = win::winspool() else { return (vec![], String::new()) };
    let mut names = Vec::new();
    unsafe {
        let flags = 0x2 | 0x4; // local + connections
        let (mut needed, mut returned) = (0u32, 0u32);
        (ws.EnumPrintersW)(flags, std::ptr::null(), 4, std::ptr::null_mut(), 0, &mut needed, &mut returned);
        if needed > 0 {
            let mut buf = vec![0u64; (needed as usize).div_ceil(8)];
            if (ws.EnumPrintersW)(flags, std::ptr::null(), 4, buf.as_mut_ptr() as *mut u8, needed, &mut needed,
                                  &mut returned) != 0
            {
                // PRINTER_INFO_4W: name ptr, server ptr, attributes (padded to 24 bytes on 64-bit)
                let base = buf.as_ptr() as *const u8;
                let stride = std::mem::size_of::<usize>() * 2 + 8;
                for i in 0..returned as usize {
                    let p = *(base.add(i * stride) as *const *const u16);
                    let n = win::from_wide(p);
                    if !n.is_empty() {
                        names.push(n);
                    }
                }
            }
        }
        let mut size = 0u32;
        (ws.GetDefaultPrinterW)(std::ptr::null_mut(), &mut size);
        let mut default = String::new();
        if size > 0 {
            let mut d = vec![0u16; size as usize + 1];
            if (ws.GetDefaultPrinterW)(d.as_mut_ptr(), &mut size) != 0 {
                default = win::wide_field(&d);
            }
        }
        names.sort_by_key(|a| a.to_lowercase());
        (names, default)
    }
}

struct Printer {
    name: Vec<u16>,
    h: HANDLE,
}

impl Printer {
    fn open(name: &str) -> Result<Printer, String> {
        let ws = win::winspool().ok_or("the Windows print service isn't available")?;
        let wname = wide(name);
        let mut h: HANDLE = std::ptr::null_mut();
        if unsafe { (ws.OpenPrinterW)(wname.as_ptr(), &mut h, std::ptr::null_mut()) } == 0 {
            return Err(format!("cannot open printer “{name}”"));
        }
        Ok(Printer { name: wname, h })
    }

    fn port(&self) -> Vec<u16> {
        let Some(ws) = win::winspool() else { return wide("") };
        unsafe {
            let mut need = 0u32;
            (ws.GetPrinterW)(self.h, 5, std::ptr::null_mut(), 0, &mut need);
            if need == 0 {
                return wide("");
            }
            let mut buf = vec![0u64; (need as usize).div_ceil(8)];
            if (ws.GetPrinterW)(self.h, 5, buf.as_mut_ptr() as *mut u8, need, &mut need) == 0 {
                return wide("");
            }
            // PRINTER_INFO_5W: pPrinterName, pPortName, ...
            let p = *((buf.as_ptr() as *const u8).add(std::mem::size_of::<usize>()) as *const *const u16);
            wide(&win::from_wide(p))
        }
    }

    /// The printer's settings block. With `base`, the driver checks and merges it.
    /// With `prompt`, the driver's own options window is shown (None on Cancel).
    fn devmode(&self, base: Option<&[u8]>, hwnd: isize, prompt: bool) -> Result<Option<Vec<u8>>, String> {
        let ws = win::winspool().ok_or("the Windows print service isn't available")?;
        unsafe {
            let size = (ws.DocumentPropertiesW)(0, self.h, self.name.as_ptr(), std::ptr::null_mut(), std::ptr::null(), 0);
            if size <= 0 {
                return Err("the printer driver did not answer".into());
            }
            let mut out = vec![0u8; size as usize];
            let mut mode = 2u32; // DM_OUT_BUFFER
            let mut inp: Vec<u8> = Vec::new();
            if let Some(b) = base {
                inp = b.to_vec();
                if inp.len() < size as usize {
                    inp.resize(size as usize, 0);
                }
                mode |= 8; // DM_IN_BUFFER
            }
            if prompt {
                mode |= 4; // DM_IN_PROMPT
            }
            let res = (ws.DocumentPropertiesW)(hwnd, self.h, self.name.as_ptr(), out.as_mut_ptr(),
                                               if base.is_some() { inp.as_ptr() } else { std::ptr::null() }, mode);
            if prompt && res == 2 {
                return Ok(None); // Cancel
            }
            if res < 0 {
                if base.is_some() && !prompt {
                    return self.devmode(None, 0, false); // saved block no longer fits: start fresh
                }
                return Err("the printer driver refused the settings".into());
            }
            let total = dm_word(&out, OFF_SIZE) as usize + dm_word(&out, OFF_EXTRA) as usize;
            if total > 0 && total <= out.len() {
                out.truncate(total);
            }
            Ok(Some(out))
        }
    }
}

impl Drop for Printer {
    fn drop(&mut self) {
        if let Some(ws) = win::winspool() {
            unsafe {
                (ws.ClosePrinter)(self.h);
            }
        }
    }
}

fn dm_word(dm: &[u8], off: usize) -> u16 {
    if dm.len() < off + 2 {
        return 0;
    }
    u16::from_le_bytes([dm[off], dm[off + 1]])
}

pub fn dm_get(dm: &[u8], off: usize) -> i16 {
    dm_word(dm, off) as i16
}

fn dm_set(dm: &mut [u8], off: usize, flag: u32, value: i16) {
    if dm.len() < OFF_COLLATE + 2 {
        return;
    }
    dm[off..off + 2].copy_from_slice(&value.to_le_bytes());
    let mut f = [0u8; 4];
    f.copy_from_slice(&dm[OFF_FIELDS..OFF_FIELDS + 4]);
    let fields = u32::from_le_bytes(f) | flag;
    dm[OFF_FIELDS..OFF_FIELDS + 4].copy_from_slice(&fields.to_le_bytes());
}

pub fn dm_to_text(dm: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(dm)
}

pub fn dm_from_text(text: &str) -> Option<Vec<u8>> {
    use base64::Engine;
    if text.is_empty() {
        return None;
    }
    base64::engine::general_purpose::STANDARD.decode(text).ok().filter(|d| d.len() > OFF_COLLATE + 2)
}

/// Show the printer driver's own options window. Some(new settings) or None on Cancel.
pub fn open_driver_dialog(printer: &str, base: Option<&[u8]>, hwnd: isize) -> Result<Option<Vec<u8>>, String> {
    Printer::open(printer)?.devmode(base, hwnd, true)
}

// --------------------------------------------------------------------------
// Print Tickets (stapling, hole punching)
// --------------------------------------------------------------------------

struct Provider(HANDLE);

impl Provider {
    fn open(printer: &str) -> Result<Provider, String> {
        let pt = win::prntvpt().ok_or("Print Ticket support is missing")?;
        let mut h: HANDLE = std::ptr::null_mut();
        let hr = unsafe { (pt.PTOpenProvider)(wide(printer).as_ptr(), 1, &mut h) };
        if hr < 0 {
            return Err(format!("printer settings service not available (0x{:08x})", hr as u32));
        }
        Ok(Provider(h))
    }

    fn capabilities(&self) -> Result<Vec<u8>, String> {
        let pt = win::prntvpt().ok_or("Print Ticket support is missing")?;
        let out = win::Stream::new(&[])?;
        let mut err = std::ptr::null_mut();
        let hr = unsafe { (pt.PTGetPrintCapabilities)(self.0, std::ptr::null_mut(), out.0, &mut err) };
        let msg = unsafe { win::take_bstr(err) };
        if hr < 0 {
            return Err(if msg.is_empty() { "could not read printer capabilities".into() } else { msg });
        }
        Ok(out.read_all())
    }

    fn devmode_to_ticket(&self, dm: &[u8]) -> Result<Vec<u8>, String> {
        let pt = win::prntvpt().ok_or("Print Ticket support is missing")?;
        let out = win::Stream::new(&[])?;
        let hr = unsafe { (pt.PTConvertDevModeToPrintTicket)(self.0, dm.len() as u32, dm.as_ptr(), 2, out.0) };
        if hr < 0 {
            return Err("could not read the printer settings".into());
        }
        Ok(out.read_all())
    }

    fn merge(&self, base: &[u8], delta: &[u8]) -> Result<Vec<u8>, String> {
        let pt = win::prntvpt().ok_or("Print Ticket support is missing")?;
        let (a, b, out) = (win::Stream::new(base)?, win::Stream::new(delta)?, win::Stream::new(&[])?);
        let mut err = std::ptr::null_mut();
        let hr = unsafe { (pt.PTMergeAndValidatePrintTicket)(self.0, a.0, b.0, 2, out.0, &mut err) };
        let msg = unsafe { win::take_bstr(err) };
        if hr < 0 {
            return Err(if msg.is_empty() { "the printer refused the finishing settings".into() } else { msg });
        }
        Ok(out.read_all())
    }

    fn ticket_to_devmode(&self, xml: &[u8]) -> Result<Vec<u8>, String> {
        let pt = win::prntvpt().ok_or("Print Ticket support is missing")?;
        let s = win::Stream::new(xml)?;
        let mut size = 0u32;
        let mut ptr: *mut u8 = std::ptr::null_mut();
        let mut err = std::ptr::null_mut();
        let hr = unsafe { (pt.PTConvertPrintTicketToDevMode)(self.0, s.0, 0, 2, &mut size, &mut ptr, &mut err) };
        let msg = unsafe { win::take_bstr(err) };
        if hr < 0 || ptr.is_null() {
            return Err(if msg.is_empty() { "could not apply the finishing settings".into() } else { msg });
        }
        let data = unsafe { std::slice::from_raw_parts(ptr, size as usize).to_vec() };
        unsafe {
            (pt.PTReleaseMemory)(ptr as *mut _);
        }
        Ok(data)
    }
}

impl Drop for Provider {
    fn drop(&mut self) {
        if let Some(pt) = win::prntvpt() {
            unsafe {
                (pt.PTCloseProvider)(self.0);
            }
        }
    }
}

fn split_camel(s: &str) -> String {
    let mut out = String::new();
    let mut prev_lower = false;
    for c in s.chars() {
        if c.is_uppercase() && prev_lower {
            out.push(' ');
        }
        prev_lower = c.is_lowercase();
        out.push(c);
    }
    out
}

fn resolve<'a>(node: roxmltree::Node<'a, 'a>, qname: &str) -> (String, String) {
    match qname.split_once(':') {
        Some((prefix, local)) => (node.lookup_namespace_uri(Some(prefix)).unwrap_or("").to_string(), local.to_string()),
        None => (String::new(), qname.to_string()),
    }
}

fn finishing_options(xml: &str, wanted: &[&str]) -> Option<Finishing> {
    let doc = roxmltree::Document::parse(xml).ok()?;
    let root = doc.root_element();
    let ns: Vec<(String, String)> = root
        .namespaces()
        .filter_map(|n| n.name().map(|p| (p.to_string(), n.uri().to_string())))
        .collect();
    for want in wanted {
        for feat in doc.descendants().filter(|n| n.has_tag_name((PSF, "Feature"))) {
            let fname = feat.attribute("name").unwrap_or("");
            let (uri, local) = resolve(feat, fname);
            if uri != PSK || local != *want {
                continue;
            }
            let mut options: Vec<(String, String)> = Vec::new();
            for opt in feat.children().filter(|n| n.has_tag_name((PSF, "Option"))) {
                let Some(oname) = opt.attribute("name") else { continue };
                let (ouri, olocal) = resolve(opt, oname);
                let mut label = None;
                if ouri == PSK {
                    label = NICE_FINISHING.iter().find(|(k, _)| *k == olocal).map(|(_, v)| v.to_string());
                }
                if label.is_none() {
                    for prop in opt.children().filter(|n| n.has_tag_name((PSF, "Property"))) {
                        if resolve(prop, prop.attribute("name").unwrap_or("")).1 == "DisplayName" {
                            if let Some(v) = prop.children().find(|n| n.has_tag_name((PSF, "Value"))) {
                                if let Some(t) = v.text().map(str::trim).filter(|t| !t.is_empty()) {
                                    label = Some(t.to_string());
                                }
                            }
                        }
                    }
                }
                let mut label = label.unwrap_or_else(|| split_camel(&olocal));
                if options.iter().any(|(_, l)| *l == label) {
                    label = format!("{label} ({olocal})");
                }
                options.push((oname.to_string(), label));
            }
            if options.len() > 1 {
                return Some(Finishing { feature: fname.to_string(), options, ns });
            }
        }
    }
    None
}

fn delta_ticket(ns: &[(String, String)], choices: &[(String, String)]) -> Vec<u8> {
    let decl: Vec<String> = ns.iter().filter(|(p, _)| p != "psf").map(|(p, u)| format!("xmlns:{p}=\"{u}\"")).collect();
    let feats: String = choices
        .iter()
        .map(|(f, o)| format!("<psf:Feature name=\"{f}\"><psf:Option name=\"{o}\"/></psf:Feature>"))
        .collect();
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?><psf:PrintTicket xmlns:psf=\"{PSF}\" {} version=\"1\">{feats}</psf:PrintTicket>",
        decl.join(" ")
    )
    .into_bytes()
}

/// What a printer can do. Safe to call from a worker thread.
pub fn printer_capabilities(printer: &str) -> Caps {
    let mut caps = Caps::default();
    if printer.is_empty() {
        return caps;
    }
    win::com_init();
    let Some(ws) = win::winspool() else {
        caps.error = "the Windows print service isn't available".into();
        return caps;
    };
    let (port, dm) = match Printer::open(printer).and_then(|p| Ok((p.port(), p.devmode(None, 0, false)?))) {
        Ok((port, Some(dm))) => (port, dm),
        Ok((_, None)) => return caps,
        Err(e) => {
            caps.error = e;
            return caps;
        }
    };
    let wname = wide(printer);
    let dc = |cap: u16, out: *mut std::ffi::c_void| unsafe {
        (ws.DeviceCapabilitiesW)(wname.as_ptr(), port.as_ptr(), cap, out, dm.as_ptr())
    };
    let list = |count_cap: u16, ids_cap: u16, names_cap: u16, width: usize| -> Vec<(String, u16)> {
        let n = dc(count_cap, std::ptr::null_mut());
        let mut out = Vec::new();
        if n <= 0 || n > 4096 {
            return out;
        }
        let n = n as usize;
        let mut ids = vec![0u16; n];
        let mut names = vec![0u16; width * n + 1];
        dc(ids_cap, ids.as_mut_ptr() as *mut _);
        dc(names_cap, names.as_mut_ptr() as *mut _);
        for i in 0..n {
            let nm = win::wide_field(&names[i * width..(i + 1) * width]).trim().to_string();
            if !nm.is_empty() && !out.iter().any(|(x, _): &(String, u16)| *x == nm) {
                out.push((nm, ids[i]));
            }
        }
        out
    };
    caps.papers = list(2, 2, 16, 64); // DC_PAPERS, DC_PAPERNAMES
    caps.bins = list(6, 6, 12, 24); // DC_BINS, DC_BINNAMES
    caps.duplex = dc(7, std::ptr::null_mut()) == 1; // DC_DUPLEX
    let mc = dc(18, std::ptr::null_mut()); // DC_COPIES
    caps.max_copies = if mc > 0 { mc } else { 1 };

    if let Ok(pv) = Provider::open(printer) {
        if let Ok(xml) = pv.capabilities() {
            let text = String::from_utf8_lossy(&xml).to_string();
            caps.staple = finishing_options(&text, &STAPLE_FEATURES);
            caps.punch = finishing_options(&text, &PUNCH_FEATURES);
        }
    }
    caps
}

/// Merge the app's choices into the printer's settings block.
pub fn build_devmode(printer: &str, base: Option<&[u8]>, opts: &JobOptions, caps: &Caps) -> Result<Vec<u8>, String> {
    let p = Printer::open(printer)?;
    let mut dm = p.devmode(base, 0, false)?.ok_or("the printer driver did not answer")?;
    if let Some(v) = opts.duplex {
        dm_set(&mut dm, OFF_DUPLEX, DM_DUPLEX, v);
    }
    if let Some(v) = opts.colour {
        dm_set(&mut dm, OFF_COLOR, DM_COLOR, v);
    }
    if let Some(v) = opts.paper {
        dm_set(&mut dm, OFF_PAPER, DM_PAPERSIZE, v as i16);
    }
    if let Some(v) = opts.bin {
        dm_set(&mut dm, OFF_SOURCE, DM_DEFAULTSOURCE, v as i16);
    }
    if opts.orientation == 1 || opts.orientation == 2 {
        dm_set(&mut dm, OFF_ORIENT, DM_ORIENTATION, opts.orientation);
    }
    let copies = if opts.driver_copies { opts.copies.min(999) as i16 } else { 1 };
    dm_set(&mut dm, OFF_COPIES, DM_COPIES, copies);
    dm_set(&mut dm, OFF_COLLATE, DM_COLLATE, 1);
    let mut dm = p.devmode(Some(&dm), 0, false)?.ok_or("the printer driver did not answer")?;
    drop(p);

    let mut choices = Vec::new();
    let mut ns = None;
    for (info, choice) in [(&caps.staple, &opts.staple), (&caps.punch, &opts.punch)] {
        if let (Some(info), Some(choice)) = (info, choice) {
            choices.push((info.feature.clone(), choice.clone()));
            ns.get_or_insert(info.ns.clone());
        }
    }
    if !choices.is_empty() {
        let pv = Provider::open(printer)?;
        let ticket = pv.devmode_to_ticket(&dm)?;
        let merged = pv.merge(&ticket, &delta_ticket(ns.as_deref().unwrap_or(&[]), &choices))?;
        dm = pv.ticket_to_devmode(&merged)?;
    }
    Ok(dm)
}

// --------------------------------------------------------------------------
// PDFs
// --------------------------------------------------------------------------

/// "All", "1-3, 5, 8-", "last" -> zero-based page indexes.
pub fn parse_pages(text: &str, total: usize) -> Result<Vec<usize>, String> {
    let t = text.trim().to_lowercase();
    if t.is_empty() || t == "all" || t == "alle" {
        return Ok((0..total).collect());
    }
    let mut out = Vec::new();
    for part in t.split([',', ';', ' ', '\t']).map(str::trim).filter(|p| !p.is_empty()) {
        let num = |s: &str, default: usize| -> Result<usize, String> {
            let s = s.trim();
            if s.is_empty() {
                Ok(default)
            } else {
                s.parse::<usize>().map_err(|_| part.to_string())
            }
        };
        let (a, b) = if let Some((x, y)) = part.split_once('-') {
            (num(x, 1)?, num(y, total)?)
        } else if part == "last" || part == "laatste" {
            (total, total)
        } else {
            let n = num(part, 0)?;
            (n, n)
        };
        for n in a.max(1)..=b.min(total) {
            out.push(n - 1);
        }
    }
    Ok(out)
}

/// Print one PDF as one print job. Returns the number of pages sent.
pub fn print_pdf(path: &Path, printer: &str, dm: &[u8], opts: &JobOptions, should_stop: &dyn Fn() -> bool)
                 -> Result<usize, String> {
    print_pdf_to(path, printer, dm, opts, should_stop, None)
}

/// Like print_pdf; `output` sends the job to a file instead (used by the tests).
pub fn print_pdf_to(path: &Path, printer: &str, dm: &[u8], opts: &JobOptions, should_stop: &dyn Fn() -> bool,
                    output: Option<&Path>) -> Result<usize, String> {
    let woutput = output.map(|o| wide(&o.to_string_lossy()));
    let gdi = win::gdi().ok_or("the Windows drawing service isn't available")?;
    let guard = pdfium::lock().ok_or("the PDF drawing library (pdfium.dll) is missing")?;
    let pdf = guard.as_ref().ok_or("the PDF drawing library (pdfium.dll) is missing")?;
    let doc = pdfium::Document::open(pdf, path)?;
    let total = doc.page_count();
    let pages = parse_pages(&opts.pages, total).map_err(|_| "page range not understood".to_string())?;
    if pages.is_empty() {
        return Err("no pages in the chosen range".into());
    }
    let runs = if opts.driver_copies { 1 } else { opts.copies.max(1) };
    let wprinter = wide(printer);
    let wspool = wide("WINSPOOL");
    let docname = wide(&path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default());
    let mut sent = 0;
    for _ in 0..runs {
        let hdc = unsafe { (gdi.CreateDCW)(wspool.as_ptr(), wprinter.as_ptr(), std::ptr::null(), dm.as_ptr()) };
        if hdc.is_null() {
            return Err("cannot connect to the printer".into());
        }
        let result = (|| -> Result<(), String> {
            let di = win::DOCINFOW {
                cbSize: std::mem::size_of::<win::DOCINFOW>() as i32,
                lpszDocName: docname.as_ptr(),
                lpszOutput: woutput.as_ref().map(|w| w.as_ptr()).unwrap_or(std::ptr::null()),
                lpszDatatype: std::ptr::null(),
                fwType: 0,
            };
            if unsafe { (gdi.StartDocW)(hdc, &di) } <= 0 {
                return Err("the printer refused the job".into());
            }
            let pages_result = (|| -> Result<(), String> {
                for &idx in &pages {
                    if should_stop() {
                        return Err("stopped".into());
                    }
                    print_page(gdi, hdc, &doc, idx, opts)?;
                    sent += 1;
                }
                Ok(())
            })();
            unsafe {
                if pages_result.is_ok() {
                    (gdi.EndDoc)(hdc);
                } else {
                    (gdi.AbortDoc)(hdc);
                }
            }
            pages_result
        })();
        unsafe {
            (gdi.DeleteDC)(hdc);
        }
        result?;
    }
    Ok(sent)
}

fn print_page(gdi: &win::Gdi, hdc: HANDLE, doc: &pdfium::Document, idx: usize, opts: &JobOptions) -> Result<(), String> {
    let (mut w_pt, mut h_pt) = doc.page_size(idx)?;
    let cap = |i: i32| unsafe { (gdi.GetDeviceCaps)(hdc, i) };
    let (dpix, dpiy) = (cap(88) as f64, cap(90) as f64);
    let (horz, vert) = (cap(8), cap(10));
    let (mut phys_w, mut phys_h, mut off_x, mut off_y) = (cap(110), cap(111), cap(112), cap(113));
    if phys_w <= 0 || phys_h <= 0 {
        (phys_w, phys_h, off_x, off_y) = (horz, vert, 0, 0);
    }
    let mut rotate = 0;
    // Automatic: turn the page if its shape doesn't match the paper.
    if opts.orientation == 0 && w_pt != h_pt && ((w_pt > h_pt) != (phys_w > phys_h)) {
        rotate = 1;
        std::mem::swap(&mut w_pt, &mut h_pt);
    }
    let w_dev = w_pt as f64 / 72.0 * dpix;
    let h_dev = h_pt as f64 / 72.0 * dpiy;
    let fit = if w_dev > 0.0 && h_dev > 0.0 { (horz as f64 / w_dev).min(vert as f64 / h_dev) } else { 1.0 };
    let scale = match opts.fit {
        Fit::Fit => fit,
        Fit::Shrink => fit.min(1.0),
        Fit::Actual => 1.0,
    };
    let out_w = (w_dev * scale).round() as i32;
    let out_h = (h_dev * scale).round() as i32;
    let mut x = ((phys_w - out_w) as f64 / 2.0 - off_x as f64).round() as i32;
    let mut y = ((phys_h - out_h) as f64 / 2.0 - off_y as f64).round() as i32;
    if opts.fit != Fit::Actual {
        x = x.min(horz - out_w).max(0);
        y = y.min(vert - out_h).max(0);
    }
    if unsafe { (gdi.StartPage)(hdc) } <= 0 {
        return Err("the printer stopped accepting pages".into());
    }
    let mut flags = 0x01 | 0x800; // annotations + printing
    if opts.colour == Some(1) {
        flags |= 0x08; // grayscale
    }
    doc.render(idx, hdc, x, y, out_w, out_h, rotate, flags)?;
    if unsafe { (gdi.EndPage)(hdc) } <= 0 {
        return Err("the printer stopped accepting pages".into());
    }
    Ok(())
}

#[allow(dead_code)]
pub fn guid(s: &str) -> GUID {
    GUID::parse(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pages() {
        assert_eq!(parse_pages("All", 3).unwrap(), vec![0, 1, 2]);
        assert_eq!(parse_pages("1-2, 5", 6).unwrap(), vec![0, 1, 4]);
        assert_eq!(parse_pages("4-", 5).unwrap(), vec![3, 4]);
        assert_eq!(parse_pages("last", 5).unwrap(), vec![4]);
        assert!(parse_pages("abc", 5).is_err());
    }

    /// Reads every printer's abilities and prints the samples to "Microsoft Print to PDF".
    /// cargo test -- --ignored --nocapture
    #[test]
    #[ignore]
    fn printers_and_print_to_file() {
        crate::win::com_init();
        let (names, default) = list_printers();
        println!("printers: {names:?}
default: {default}");
        for n in &names {
            let c = printer_capabilities(n);
            println!("{n}: papers {} bins {} duplex {} copies {} staple {:?} punch {:?} err '{}'",
                     c.papers.len(), c.bins.len(), c.duplex, c.max_copies,
                     c.staple.as_ref().map(|f| f.options.iter().map(|o| o.1.clone()).collect::<Vec<_>>()),
                     c.punch.as_ref().map(|f| f.options.len()), c.error);
        }
        let pdf = "Microsoft Print to PDF";
        if !names.iter().any(|n| n == pdf) {
            println!("no {pdf}; skipping the print test");
            return;
        }
        let caps = printer_capabilities(pdf);
        let opts = JobOptions { copies: 1, driver_copies: true, duplex: None, colour: Some(1), orientation: 0, fit: Fit::Fit,
                                pages: "1-2".into(), paper: None, bin: None, staple: None, punch: None };
        let dm = build_devmode(pdf, None, &opts, &caps).expect("settings block");
        let out_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("shots").join("printtest");
        let _ = std::fs::create_dir_all(&out_dir);
        for f in ["Letter.pdf", "Invoice 2.pdf"] {
            let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("samples").join(f);
            let out = out_dir.join(format!("printed {f}"));
            let _ = std::fs::remove_file(&out);
            let sent = print_pdf_to(&src, pdf, &dm, &opts, &|| false, Some(&out)).expect("print");
            for _ in 0..50 {
                if out.metadata().map(|m| m.len() > 0).unwrap_or(false) { break; }
                std::thread::sleep(std::time::Duration::from_millis(200));
            }
            println!("{f}: {sent} pages sent -> {} bytes", out.metadata().map(|m| m.len()).unwrap_or(0));
            assert!(sent > 0);
        }
    }
}
