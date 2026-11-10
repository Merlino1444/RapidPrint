//! The RapidPrint window: file list, print settings, archive, progress.

use crate::brand;
use crate::common::{self, Match, Rule, Settings, Target, APP_NAME};
use crate::installer;
use crate::pdfium;
use crate::printing::{self, Caps, Fit, JobOptions, AS_PRINTER, COLOUR, FIT, ORIENTATION, TWO_SIDED};
use crate::theme::{self, Palette, DARK, LIGHT};
use eframe::egui::{self, Align, Color32, CornerRadius, Layout, RichText, Sense, Stroke};
use serde_json::{json, Value};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;

const NOT_HERE: &str = "Not on this printer";
const LOADING: &str = "Checking printer…";

pub struct Job {
    id: u64,
    path: PathBuf,
    name: String,
    folder: String,
    pages: Option<usize>,
    counted: bool,
    status: String,
    archived: String,
}

enum Event {
    Caps(String, Caps),
    Pages(u64, Option<usize>),
    Status(u64, String, Option<usize>),
    Archived(u64, String, bool),
    Finished { done: usize, failed: usize, stopped: bool },
}

pub struct Shot {
    pub path: PathBuf,
    pub frames: u32,
    pub asked: bool,
}

pub struct App {
    pal: Palette,
    look_chosen: bool,
    logo_light: Option<egui::TextureHandle>,
    logo_dark: Option<egui::TextureHandle>,

    settings: Settings,
    per_printer: serde_json::Map<String, Value>,

    jobs: Vec<Job>,
    next_id: u64,
    selected: BTreeSet<usize>,
    anchor: Option<usize>,

    printers: Vec<String>,
    printer: String,
    current: String,
    caps: HashMap<String, Caps>,
    caps_loading: HashSet<String>,

    copies: u32,
    duplex: String,
    colour: String,
    pages: String,
    staple: String,
    punch: String,
    paper: String,
    tray: String,
    orientation: String,
    fit: String,
    subfolders: bool,
    clear_done: bool,
    archive_on: bool,
    rules: Vec<Rule>,
    archive_other: String,
    archive_move: bool,
    archive_monthly: bool,

    status: String,
    progress: (usize, usize),
    archive_copied: usize,
    archive_problems: usize,
    busy: bool,
    stop: Arc<AtomicBool>,
    tx: Sender<Event>,
    rx: Receiver<Event>,

    shot: Option<Shot>,
    started: f64,
    offered_install: bool,
    show_install: bool,
    hwnd: isize,
}

fn pick(saved: &str, options: &[&str], default: &str) -> String {
    if options.contains(&saved) {
        saved.to_string()
    } else {
        default.to_string()
    }
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, paths: Vec<PathBuf>, shot: Option<Shot>) -> App {
        theme::install_fonts(&cc.egui_ctx);
        let settings = Settings::load();
        let saved_look = settings.str("theme");
        let look_chosen = saved_look == "light" || saved_look == "dark";
        let forced = std::env::var("RAPIDPRINT_LOOK").unwrap_or_default(); // for test pictures only
        let dark = if forced == "dark" || forced == "light" {
            forced == "dark"
        } else if look_chosen {
            saved_look == "dark"
        } else {
            theme::windows_prefers_dark()
        };
        let pal = if dark { DARK } else { LIGHT };
        theme::apply(&cc.egui_ctx, &pal);

        let load = |name: &str, bytes: &[u8]| {
            theme::decode_png(bytes).map(|img| cc.egui_ctx.load_texture(name, img, egui::TextureOptions::LINEAR))
        };
        let (tx, rx) = channel();
        let rules = load_rules(&settings);
        let two: Vec<&str> = TWO_SIDED.iter().map(|x| x.0).collect();
        let col: Vec<&str> = COLOUR.iter().map(|x| x.0).collect();
        let ori: Vec<&str> = ORIENTATION.iter().map(|x| x.0).collect();
        let fits: Vec<&str> = FIT.iter().map(|x| x.0).collect();

        let mut app = App {
            pal,
            look_chosen,
            logo_light: brand::LOGO_LIGHT.and_then(|b| load("logo-light", b)),
            logo_dark: brand::LOGO_DARK.and_then(|b| load("logo-dark", b)),
            per_printer: settings.obj("per_printer"),
            jobs: vec![],
            next_id: 1,
            selected: BTreeSet::new(),
            anchor: None,
            printers: vec![],
            printer: settings.str("printer"),
            current: String::new(),
            caps: HashMap::new(),
            caps_loading: HashSet::new(),
            copies: settings.int("copies", 1).clamp(1, 999) as u32,
            duplex: pick(&settings.str("two_sided"), &two, AS_PRINTER),
            colour: pick(&settings.str("colour"), &col, AS_PRINTER),
            pages: {
                let p = settings.str("pages");
                if p.is_empty() { "All".into() } else { p }
            },
            staple: LOADING.into(),
            punch: LOADING.into(),
            paper: LOADING.into(),
            tray: LOADING.into(),
            orientation: pick(&settings.str("orientation"), &ori, "Automatic"),
            fit: pick(&settings.str("fit"), &fits, "Fit to page"),
            subfolders: settings.bool("subfolders", true),
            clear_done: settings.bool("clear_done", false),
            archive_on: settings.bool("archive_on", !brand::DEFAULT_RULES.is_empty()),
            rules,
            archive_other: settings.str("archive_other"),
            archive_move: settings.str("archive_mode") == "move",
            archive_monthly: settings.bool("archive_monthly", false),
            status: String::new(),
            progress: (0, 0),
            archive_copied: 0,
            archive_problems: 0,
            busy: false,
            stop: Arc::new(AtomicBool::new(false)),
            tx,
            rx,
            shot,
            started: 0.0,
            offered_install: false,
            show_install: !installer::is_dev_build() && !installer::installed_here(),
            hwnd: 0,
            settings,
        };
        app.refresh_printers(&cc.egui_ctx);
        if !paths.is_empty() {
            app.add_paths(&paths, &cc.egui_ctx);
        }
        app
    }

    // ---- settings ------------------------------------------------------
    fn remember_printer_choices(&mut self) {
        if self.current.is_empty() {
            return;
        }
        let entry = self.per_printer.entry(self.current.clone()).or_insert_with(|| json!({}));
        if let Some(obj) = entry.as_object_mut() {
            for (key, val) in [("staple", &self.staple), ("punch", &self.punch), ("paper", &self.paper), ("tray", &self.tray)] {
                if val != NOT_HERE && val != LOADING {
                    obj.insert(key.into(), json!(val));
                }
            }
        }
    }

    fn save_settings(&mut self) {
        self.remember_printer_choices();
        let s = &mut self.settings;
        s.set("printer", json!(self.printer));
        s.set("copies", json!(self.copies));
        s.set("two_sided", json!(if TWO_SIDED.iter().any(|x| x.0 == self.duplex) { self.duplex.as_str() } else { AS_PRINTER }));
        s.set("colour", json!(self.colour));
        s.set("orientation", json!(self.orientation));
        s.set("fit", json!(self.fit));
        s.set("pages", json!(self.pages));
        s.set("subfolders", json!(self.subfolders));
        s.set("clear_done", json!(self.clear_done));
        s.set("archive_on", json!(self.archive_on));
        let rules: Vec<Value> = self
            .rules
            .iter()
            .map(|r| json!({"match": r.how.key(), "text": r.text, "folder": r.folder.trim()}))
            .collect();
        s.set("archive_rules", Value::Array(rules));
        s.set("archive_other", json!(self.archive_other.trim()));
        s.set("archive_mode", json!(if self.archive_move { "move" } else { "copy" }));
        s.set("archive_monthly", json!(self.archive_monthly));
        s.set("theme", json!(if self.look_chosen { if self.pal.dark { "dark" } else { "light" } } else { "" }));
        s.set("per_printer", Value::Object(self.per_printer.clone()));
        s.save();
    }

    // ---- printers ------------------------------------------------------
    fn refresh_printers(&mut self, ctx: &egui::Context) {
        let (names, default) = printing::list_printers();
        if !names.contains(&self.printer) {
            self.printer = if names.contains(&default) { default } else { names.first().cloned().unwrap_or_default() };
        }
        self.printers = names;
        self.caps.remove(&self.printer);
        self.current.clear();
        self.on_printer_changed(ctx);
    }

    fn on_printer_changed(&mut self, ctx: &egui::Context) {
        self.remember_printer_choices();
        let name = self.printer.clone();
        self.current = name.clone();
        if self.caps.contains_key(&name) {
            self.show_caps();
            return;
        }
        for v in [&mut self.staple, &mut self.punch, &mut self.paper, &mut self.tray] {
            *v = LOADING.into();
        }
        if !name.is_empty() && self.caps_loading.insert(name.clone()) {
            let tx = self.tx.clone();
            let ctx = ctx.clone();
            std::thread::spawn(move || {
                let caps = printing::printer_capabilities(&name);
                let _ = tx.send(Event::Caps(name, caps));
                ctx.request_repaint();
            });
        }
    }

    fn show_caps(&mut self) {
        let caps = self.caps.get(&self.printer).cloned().unwrap_or_default();
        let saved = self.per_printer.get(&self.printer).and_then(|v| v.as_object().cloned()).unwrap_or_default();
        let fill = |labels: Vec<String>, key: &str| -> String {
            if labels.is_empty() {
                return NOT_HERE.into();
            }
            let s = saved.get(key).and_then(|v| v.as_str()).unwrap_or("");
            if labels.iter().any(|l| l == s) { s.to_string() } else { AS_PRINTER.into() }
        };
        self.staple = fill(caps.staple.iter().flat_map(|f| f.options.iter().map(|o| o.1.clone())).collect(), "staple");
        self.punch = fill(caps.punch.iter().flat_map(|f| f.options.iter().map(|o| o.1.clone())).collect(), "punch");
        self.paper = fill(caps.papers.iter().map(|x| x.0.clone()).collect(), "paper");
        self.tray = fill(caps.bins.iter().map(|x| x.0.clone()).collect(), "tray");
        if !caps.duplex {
            self.duplex = NOT_HERE.into();
        } else if !TWO_SIDED.iter().any(|x| x.0 == self.duplex) {
            let s = self.settings.str("two_sided");
            self.duplex = if TWO_SIDED.iter().any(|x| x.0 == s) { s } else { AS_PRINTER.into() };
        }
        if !caps.error.is_empty() {
            self.status = format!("Printer problem: {}", caps.error);
        }
    }

    fn driver_devmode(&self, name: &str) -> Option<Vec<u8>> {
        self.per_printer
            .get(name)
            .and_then(|v| v.get("devmode"))
            .and_then(|v| v.as_str())
            .and_then(printing::dm_from_text)
    }

    fn open_driver_options(&mut self) {
        let name = self.printer.clone();
        if name.is_empty() || self.busy {
            return;
        }
        let base = self.driver_devmode(&name);
        match printing::open_driver_dialog(&name, base.as_deref(), self.hwnd) {
            Err(e) => {
                message(&format!("Could not open the printer’s options:\n{e}"));
            }
            Ok(None) => {}
            Ok(Some(dm)) => {
                let entry = self.per_printer.entry(name).or_insert_with(|| json!({}));
                if let Some(o) = entry.as_object_mut() {
                    o.insert("devmode".into(), json!(printing::dm_to_text(&dm)));
                }
                // From now on the printer window's choices count, unless changed here again.
                for v in [&mut self.duplex, &mut self.colour, &mut self.staple, &mut self.punch, &mut self.paper, &mut self.tray] {
                    if v != NOT_HERE && v != LOADING {
                        *v = AS_PRINTER.into();
                    }
                }
                let c = printing::dm_get(&dm, printing::OFF_COPIES);
                if c > 1 {
                    self.copies = c as u32;
                }
                self.save_settings();
            }
        }
    }

    fn reset_driver_options(&mut self) {
        if let Some(o) = self.per_printer.get_mut(&self.printer).and_then(|v| v.as_object_mut()) {
            o.remove("devmode");
        }
        self.save_settings();
    }

    // ---- list ----------------------------------------------------------
    fn add_paths(&mut self, paths: &[PathBuf], ctx: &egui::Context) {
        if self.busy {
            self.status = "Files can be added when printing has finished.".into();
            return;
        }
        let mut known: HashSet<String> = self.jobs.iter().map(|j| j.path.to_string_lossy().to_lowercase()).collect();
        let pdfs = common::collect_pdfs(paths, self.subfolders);
        let mut to_count = Vec::new();
        for p in &pdfs {
            let abs = std::path::absolute(p).unwrap_or_else(|_| p.clone());
            if !known.insert(abs.to_string_lossy().to_lowercase()) {
                continue;
            }
            let name = abs.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            let id = self.next_id;
            self.next_id += 1;
            to_count.push((id, abs.clone()));
            self.jobs.push(Job {
                id,
                folder: abs.parent().map(|f| f.to_string_lossy().to_string()).unwrap_or_default(),
                name,
                path: abs,
                pages: None,
                counted: false,
                status: "Ready".into(),
                archived: String::new(),
            });
        }
        if !paths.is_empty() && pdfs.is_empty() {
            self.status = "No PDF files found in what you added.".into();
        } else {
            self.status.clear();
        }
        if !to_count.is_empty() {
            let tx = self.tx.clone();
            let ctx = ctx.clone();
            std::thread::spawn(move || {
                for (id, path) in to_count {
                    let n = pdfium::count_pages(&path);
                    let _ = tx.send(Event::Pages(id, n));
                    ctx.request_repaint();
                }
            });
        }
    }

    fn add_files(&mut self, ctx: &egui::Context) {
        if let Some(files) = rfd::FileDialog::new().set_title("Add PDF files").add_filter("PDF files", &["pdf"]).pick_files() {
            self.add_paths(&files, ctx);
        }
    }

    fn add_folder(&mut self, ctx: &egui::Context) {
        if let Some(folder) = rfd::FileDialog::new().set_title("Add a folder of PDF files").pick_folder() {
            self.add_paths(&[folder], ctx);
        }
    }

    fn remove_selected(&mut self) {
        if self.busy {
            return;
        }
        let sel = std::mem::take(&mut self.selected);
        let mut k = 0;
        self.jobs.retain(|_| {
            let keep = !sel.contains(&k);
            k += 1;
            keep
        });
        self.anchor = None;
    }

    fn move_selected(&mut self, step: i32) {
        if self.busy || self.selected.is_empty() {
            return;
        }
        let sel: Vec<usize> = self.selected.iter().copied().collect();
        if (step < 0 && sel[0] == 0) || (step > 0 && *sel.last().unwrap() + 1 >= self.jobs.len()) {
            return;
        }
        let order: Vec<usize> = if step < 0 { sel.clone() } else { sel.iter().rev().copied().collect() };
        for k in order {
            let other = (k as i32 + step) as usize;
            self.jobs.swap(k, other);
        }
        self.selected = sel.iter().map(|&k| (k as i32 + step) as usize).collect();
    }

    /// The folder a file goes to after printing (None: it stays where it is).
    fn target_folder(&self, name: &str) -> Option<String> {
        if !self.archive_on {
            return None;
        }
        match common::target_for(&self.rules, &self.archive_other, name)? {
            Target::Rule(k) => Some(self.rules[k].folder.trim().to_string()),
            Target::Other => Some(self.archive_other.trim().to_string()),
        }
    }

    fn archive_text(&self, job: &Job) -> String {
        if !job.archived.is_empty() {
            return job.archived.clone();
        }
        match self.target_folder(&job.name) {
            None => String::new(),
            Some(f) if f.is_empty() => "⚠ No folder chosen".into(),
            Some(f) => format!("→ {}", common::folder_label(&f)),
        }
    }

    fn summary(&self) -> String {
        if self.busy {
            return self.status.clone();
        }
        if !self.status.is_empty() {
            return self.status.clone();
        }
        let n = self.jobs.len();
        if n == 0 {
            return "No files yet.".into();
        }
        let known: usize = self.jobs.iter().filter_map(|j| j.pages).sum();
        let extra = if self.jobs.iter().all(|j| j.pages.is_some() || !j.counted) { "" } else { " (some unknown)" };
        let counting = if self.jobs.iter().any(|j| !j.counted) { " (counting…)" } else { "" };
        format!(
            "{n} file{} · {known} page{}{extra}{counting}. Ready to print.",
            if n != 1 { "s" } else { "" },
            if known != 1 { "s" } else { "" }
        )
    }

    // ---- printing ------------------------------------------------------
    fn collect_options(&mut self, caps: &Caps) -> JobOptions {
        self.copies = self.copies.clamp(1, 999);
        let finishing = |info: &Option<printing::Finishing>, label: &str| -> Option<String> {
            info.as_ref().and_then(|f| f.options.iter().find(|o| o.1 == label).map(|o| o.0.clone()))
        };
        JobOptions {
            copies: self.copies,
            driver_copies: caps.max_copies >= self.copies as i32,
            duplex: TWO_SIDED.iter().find(|x| x.0 == self.duplex).and_then(|x| x.1),
            colour: COLOUR.iter().find(|x| x.0 == self.colour).and_then(|x| x.1),
            orientation: ORIENTATION.iter().find(|x| x.0 == self.orientation).map(|x| x.1).unwrap_or(0),
            fit: FIT.iter().find(|x| x.0 == self.fit).map(|x| x.1).unwrap_or(Fit::Fit),
            pages: self.pages.clone(),
            paper: caps.papers.iter().find(|x| x.0 == self.paper).map(|x| x.1),
            bin: caps.bins.iter().find(|x| x.0 == self.tray).map(|x| x.1),
            staple: finishing(&caps.staple, &self.staple),
            punch: finishing(&caps.punch, &self.punch),
        }
    }

    fn describe(&self, o: &JobOptions) -> String {
        let mut bits = Vec::new();
        if o.copies > 1 {
            bits.push(format!("{} copies each", o.copies));
        }
        if matches!(o.duplex, Some(2) | Some(3)) {
            bits.push("two-sided".into());
        }
        if o.staple.is_some() && self.staple != "Off" {
            bits.push("stapled".into());
        }
        if o.punch.is_some() && self.punch != "Off" {
            bits.push("hole punched".into());
        }
        if o.colour == Some(1) {
            bits.push("black & white".into());
        }
        if bits.is_empty() { String::new() } else { format!(" ({})", bits.join(", ")) }
    }

    fn start(&mut self, ctx: &egui::Context) {
        if self.busy || self.jobs.is_empty() {
            return;
        }
        let printer = self.printer.clone();
        if printer.is_empty() {
            message("Please choose a printer first.");
            return;
        }
        if !pdfium::available() {
            message("The PDF drawing part of this app (pdfium.dll) is missing, so it can’t print.");
            return;
        }
        let Some(caps) = self.caps.get(&printer).cloned() else {
            self.status = "Still checking what this printer can do — try again in a moment.".into();
            return;
        };
        let opts = self.collect_options(&caps);
        if printing::parse_pages(&opts.pages, 9999).is_err() {
            message("The page range isn’t clear. Use All, or something like 1-3, 5.");
            return;
        }
        let mut todo: Vec<usize> = (0..self.jobs.len()).filter(|&k| self.jobs[k].status != "Printed").collect();
        if todo.is_empty() {
            todo = (0..self.jobs.len()).collect();
        }
        let mut missing: Vec<String> = todo
            .iter()
            .filter_map(|&k| self.target_folder(&self.jobs[k].name))
            .filter(|f| !std::path::Path::new(f).is_dir())
            .map(|f| if f.is_empty() { "(no folder chosen)".to_string() } else { f })
            .collect();
        missing.sort();
        missing.dedup();
        if !missing.is_empty()
            && !ask(&format!(
                "These archive folders aren’t chosen or can’t be found, so some files won’t be {}:\n\n{}\n\nPrint anyway?",
                if self.archive_move { "moved" } else { "copied" },
                missing.join("\n")
            ))
        {
            return;
        }
        self.save_settings();
        let targets: Vec<Option<String>> = todo.iter().map(|&k| self.target_folder(&self.jobs[k].name)).collect();
        let (move_it, monthly) = (self.archive_move, self.archive_monthly);
        let work: Vec<(u64, PathBuf, Option<String>)> = todo
            .iter()
            .zip(targets)
            .map(|(&k, target)| {
                let j = &mut self.jobs[k];
                j.status = "Waiting".into();
                j.archived.clear();
                (j.id, j.path.clone(), target)
            })
            .collect();
        self.archive_copied = 0;
        self.archive_problems = 0;
        self.stop.store(false, Ordering::SeqCst);
        self.progress = (0, work.len());
        self.busy = true;
        self.status = format!(
            "Printing {} file{} on {printer}{}…",
            work.len(),
            if work.len() != 1 { "s" } else { "" },
            self.describe(&opts)
        );
        let base = self.driver_devmode(&printer);
        let tx = self.tx.clone();
        let stop = self.stop.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            crate::win::com_init();
            let send = |e: Event| {
                let _ = tx.send(e);
                ctx.request_repaint();
            };
            let dm = match printing::build_devmode(&printer, base.as_deref(), &opts, &caps) {
                Ok(dm) => dm,
                Err(e) => {
                    for (id, _, _) in &work {
                        send(Event::Status(*id, format!("Failed: {e}"), None));
                    }
                    send(Event::Finished { done: 0, failed: work.len(), stopped: false });
                    return;
                }
            };
            let (mut done, mut failed) = (0, 0);
            for (n, (id, path, target)) in work.iter().enumerate() {
                if stop.load(Ordering::SeqCst) {
                    break;
                }
                send(Event::Status(*id, "Printing…".into(), Some(n)));
                let result = if !path.is_file() {
                    Err("file missing".to_string())
                } else {
                    let stopper = || stop.load(Ordering::SeqCst);
                    printing::print_pdf(path, &printer, &dm, &opts, &stopper)
                };
                match result {
                    Ok(_) => {
                        done += 1;
                        send(Event::Status(*id, "Printed".into(), Some(n + 1)));
                        if let Some(folder) = target {
                            let (ok, msg) = common::file_to_archive(path, folder, move_it, monthly);
                            let mut c = msg.chars();
                            let msg = c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default();
                            let verb = if move_it { "moved" } else { "copied" };
                            let text = if ok { format!("✓ {msg}") } else { format!("✗ Not {verb}: {msg}") };
                            send(Event::Archived(*id, text, ok));
                        }
                    }
                    Err(e) if e == "stopped" => {
                        send(Event::Status(*id, "Ready".into(), Some(n + 1)));
                        break;
                    }
                    Err(e) => {
                        failed += 1;
                        send(Event::Status(*id, format!("Failed: {e}"), Some(n + 1)));
                    }
                }
            }
            let stopped = stop.load(Ordering::SeqCst);
            send(Event::Finished { done, failed, stopped });
        });
    }

    fn poll_events(&mut self) {
        while let Ok(ev) = self.rx.try_recv() {
            match ev {
                Event::Caps(name, caps) => {
                    self.caps_loading.remove(&name);
                    self.caps.insert(name.clone(), caps);
                    if name == self.printer {
                        self.show_caps();
                    }
                }
                Event::Pages(id, n) => {
                    if let Some(j) = self.jobs.iter_mut().find(|j| j.id == id) {
                        j.pages = n;
                        j.counted = true;
                    }
                }
                Event::Status(id, text, prog) => {
                    if let Some(j) = self.jobs.iter_mut().find(|j| j.id == id) {
                        j.status = text;
                    }
                    if let Some(p) = prog {
                        self.progress.0 = p;
                    }
                }
                Event::Archived(id, text, ok) => {
                    if let Some(j) = self.jobs.iter_mut().find(|j| j.id == id) {
                        j.archived = text;
                    }
                    if ok {
                        self.archive_copied += 1;
                    } else {
                        self.archive_problems += 1;
                    }
                }
                Event::Finished { done, failed, stopped } => {
                    self.busy = false;
                    for j in self.jobs.iter_mut() {
                        if j.status == "Waiting" {
                            j.status = "Ready".into();
                        }
                    }
                    if self.clear_done {
                        self.jobs.retain(|j| j.status != "Printed");
                        self.selected.clear();
                    }
                    let mut msg = format!("Done: {done} file{} sent to the printer", if done != 1 { "s" } else { "" });
                    if failed > 0 {
                        msg += &format!(", {failed} failed");
                    }
                    if self.archive_copied > 0 {
                        msg += &format!("; {} file{} archived", self.archive_copied, if self.archive_copied != 1 { "s" } else { "" });
                    }
                    if self.archive_problems > 0 {
                        msg += &format!("; {} couldn’t be archived", self.archive_problems);
                    }
                    if stopped {
                        msg += " (stopped)";
                    }
                    self.status = msg + ".";
                }
            }
        }
    }

    // ---- install --------------------------------------------------------
    fn offer_install(&mut self, asked: bool) {
        if installer::is_dev_build() || installer::installed_here() || self.busy {
            return;
        }
        let here = installer::exe_dir().to_string_lossy().to_string();
        if !asked && self.settings.str("install_declined") == here {
            return;
        }
        let mut question = if installer::is_installed() {
            "A copy of RapidPrint is already installed on this computer.\n\nReplace it with this version?".to_string()
        } else {
            "Install RapidPrint on this computer?\n\nIt will be added to your Start menu and desktop, and to “Send to” when you \
             right-click PDF files or folders. No administrator rights are needed."
                .to_string()
        };
        if let Some(old) = installer::legacy_installed() {
            question += &format!("\n\nThe older copy called “{old}” and its shortcuts will be replaced. Your settings are kept.");
        }
        if !ask(&question) {
            self.settings.set("install_declined", json!(here));
            self.save_settings();
            return;
        }
        match installer::install(true) {
            Ok(_) => {
                self.show_install = false;
                message("Done! RapidPrint is now in your Start menu and on your desktop.\n\n\
                         Tip: right-click PDF files or a folder → Send to → RapidPrint.");
            }
            Err(e) => message(&format!(
                "Installing didn’t work:\n\n{e}\n\nIf RapidPrint is already open in another window, close it and try again."
            )),
        }
    }

    // ---- drawing ---------------------------------------------------------
    fn header(&mut self, ctx: &egui::Context) {
        let p = self.pal;
        egui::TopBottomPanel::top("header")
            .exact_height(64.0)
            .frame(egui::Frame::new().fill(p.card).inner_margin(egui::Margin::symmetric(18, 0)))
            .show_separator_line(true)
            .show(ctx, |ui| {
                ui.horizontal_centered(|ui| {
                    let logo = if p.dark { &self.logo_dark } else { &self.logo_light };
                    if let Some(tex) = logo {
                        let size = tex.size_vec2();
                        let h = 40.0;
                        ui.add(egui::Image::new((tex.id(), egui::vec2(size.x * h / size.y, h))));
                        ui.add_space(14.0);
                        let (r, _) = ui.allocate_exact_size(egui::vec2(1.0, 34.0), Sense::hover());
                        ui.painter().rect_filled(r, 0.0, p.border);
                        ui.add_space(14.0);
                    } else {
                        printer_badge(ui, &p);
                        ui.add_space(10.0);
                    }
                    ui.vertical(|ui| {
                        ui.add_space(11.0);
                        ui.label(RichText::new(APP_NAME).font(theme::semibold(19.0)).color(p.text));
                        ui.add_space(-4.0);
                        ui.label(RichText::new("Prints only your pages — no cover sheets, no reports, no ads").size(12.0).color(p.muted));
                    });
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let text = if p.dark { "☀  Light mode" } else { "☾  Dark mode" };
                        let b = egui::Button::new(RichText::new(text).size(12.5))
                            .fill(p.button)
                            .corner_radius(CornerRadius::same(14))
                            .min_size(egui::vec2(0.0, 30.0));
                        if ui.add(b).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                            self.pal = if p.dark { LIGHT } else { DARK };
                            self.look_chosen = true;
                            theme::apply(ctx, &self.pal);
                            self.save_settings();
                        }
                        if self.show_install && self.shot.is_none() {
                            ui.add_space(4.0);
                            let b = egui::Button::new(RichText::new("⤓  Install on this computer").font(theme::semibold(12.5)).color(p.accent))
                                .fill(p.accent_soft)
                                .corner_radius(CornerRadius::same(14))
                                .min_size(egui::vec2(0.0, 30.0));
                            if ui.add(b).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                                self.offer_install(true);
                            }
                        }
                    });
                });
            });
    }

    fn bottom_bar(&mut self, ctx: &egui::Context) {
        let p = self.pal;
        egui::TopBottomPanel::bottom("bottom")
            .frame(egui::Frame::new().fill(p.bg).inner_margin(egui::Margin { left: 14, right: 14, top: 0, bottom: 14 }))
            .show_separator_line(false)
            .show(ctx, |ui| {
                theme::card(&p).inner_margin(egui::Margin::same(12)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            let print = egui::Button::new(RichText::new("🖨  Print all").font(theme::semibold(15.0)).color(Color32::WHITE))
                                .fill(if self.busy || self.jobs.is_empty() { p.accent_off } else { p.accent })
                                .corner_radius(CornerRadius::same(10))
                                .stroke(Stroke::NONE)
                                .min_size(egui::vec2(132.0, 44.0));
                            let can_print = !self.busy && !self.jobs.is_empty();
                            if ui.add_enabled(can_print, print).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                                self.start(ctx);
                            }
                            let stop = egui::Button::new(RichText::new("Stop").size(14.0).color(if self.busy { p.err } else { p.disabled }))
                                .fill(p.card)
                                .stroke(Stroke::new(1.0, p.field_line))
                                .corner_radius(CornerRadius::same(10))
                                .min_size(egui::vec2(108.0, 44.0));
                            if ui.add_enabled(self.busy, stop).clicked() {
                                self.stop.store(true, Ordering::SeqCst);
                                self.status = "Stopping…".into();
                            }
                            ui.add_space(14.0);
                            ui.with_layout(Layout::top_down(Align::Min), |ui| {
                                ui.add_space(2.0);
                                ui.label(RichText::new(self.summary()).color(p.text));
                                ui.add_space(4.0);
                                let frac = if self.progress.1 > 0 { self.progress.0 as f32 / self.progress.1 as f32 } else { 0.0 };
                                progress_bar(ui, &p, frac);
                            });
                        });
                    });
                });
            });
    }

    fn settings_panel(&mut self, ctx: &egui::Context) {
        let p = self.pal;
        egui::SidePanel::right("settings")
            .exact_width(350.0)
            .resizable(false)
            .show_separator_line(false)
            .frame(egui::Frame::new().fill(p.bg).inner_margin(egui::Margin { left: 0, right: 14, top: 14, bottom: 14 }))
            .show(ctx, |ui| {
                theme::card(&p).show(ui, |ui| {
                    ui.set_min_size(ui.available_size());
                    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| self.settings_content(ui, ctx));
                });
            });
    }

    fn settings_content(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let p = self.pal;
        let busy = self.busy;
        ui.label(RichText::new("Print settings").font(theme::semibold(16.5)));
        ui.add_space(4.0);
        ui.label(RichText::new("Printer").size(12.0).color(p.muted));
        let mut changed_printer = false;
        let mut refresh = false;
        ui.horizontal(|ui| {
            let w = ui.available_width() - 40.0;
            let current = self.printer.clone();
            egui::ComboBox::from_id_salt("printer")
                .selected_text(if current.is_empty() { "(no printers found)".to_string() } else { current.clone() })
                .width(w)
                .height(400.0)
                .show_ui(ui, |ui| {
                    for name in self.printers.clone() {
                        if ui.selectable_label(name == self.printer, &name).clicked() && name != self.printer {
                            self.printer = name;
                            changed_printer = true;
                        }
                    }
                });
            if ui.add_sized([32.0, 28.0], egui::Button::new("↻")).on_hover_text("Look for printers again").clicked() {
                refresh = true;
            }
        });
        if changed_printer {
            self.on_printer_changed(ctx);
        }
        if refresh {
            self.refresh_printers(ctx);
        }

        let caps = self.caps.get(&self.printer).cloned();
        let section = |ui: &mut egui::Ui, t: &str| {
            ui.add_space(8.0);
            ui.label(RichText::new(t).font(theme::semibold(12.0)).color(p.accent));
        };
        let label_w = 82.0;
        let combo = |ui: &mut egui::Ui, id: &str, label: &str, value: &mut String, options: Vec<String>, enabled: bool| {
            ui.horizontal(|ui| {
                field_label(ui, label, label_w);
                let w = ui.available_width();
                ui.add_enabled_ui(enabled && !options.is_empty(), |ui| {
                    egui::ComboBox::from_id_salt(id).selected_text(value.clone()).width(w).height(360.0).show_ui(ui, |ui| {
                        for o in options {
                            ui.selectable_value(value, o.clone(), o);
                        }
                    });
                });
            });
        };
        let with_default = |labels: Vec<String>| -> Vec<String> {
            if labels.is_empty() {
                vec![]
            } else {
                std::iter::once(AS_PRINTER.to_string()).chain(labels).collect()
            }
        };

        ui.add_enabled_ui(!busy, |ui| {
            section(ui, "BASICS");
            ui.horizontal(|ui| {
                field_label(ui, "Copies", label_w);
                ui.add(egui::DragValue::new(&mut self.copies).range(1..=999).speed(0.1));
            });
            let duplex_ok = caps.as_ref().map(|c| c.duplex).unwrap_or(true);
            combo(ui, "two", "Two-sided", &mut self.duplex, TWO_SIDED.iter().map(|x| x.0.to_string()).collect(), duplex_ok);
            combo(ui, "colour", "Colour", &mut self.colour, COLOUR.iter().map(|x| x.0.to_string()).collect(), true);
            ui.horizontal(|ui| {
                field_label(ui, "Pages", label_w);
                ui.add_sized([ui.available_width() - 16.0, 28.0], egui::TextEdit::singleline(&mut self.pages).margin(egui::vec2(8.0, 5.0)));
            });
            ui.horizontal(|ui| {
                ui.add_space(label_w + 8.0);
                ui.label(RichText::new("All, or e.g. 1-3, 5 — used for every file").size(11.5).color(p.muted));
            });

            section(ui, "FINISHING");
            let (st, pu, pa, tr) = match &caps {
                Some(c) => (
                    with_default(c.staple.iter().flat_map(|f| f.options.iter().map(|o| o.1.clone())).collect()),
                    with_default(c.punch.iter().flat_map(|f| f.options.iter().map(|o| o.1.clone())).collect()),
                    with_default(c.papers.iter().map(|x| x.0.clone()).collect()),
                    with_default(c.bins.iter().map(|x| x.0.clone()).collect()),
                ),
                None => (vec![], vec![], vec![], vec![]),
            };
            combo(ui, "staple", "Staple", &mut self.staple, st, true);
            combo(ui, "punch", "Hole punch", &mut self.punch, pu, true);
            section(ui, "PAPER");
            combo(ui, "paper", "Paper size", &mut self.paper, pa, true);
            combo(ui, "tray", "Paper tray", &mut self.tray, tr, true);
            combo(ui, "orient", "Orientation", &mut self.orientation, ORIENTATION.iter().map(|x| x.0.to_string()).collect(), true);
            combo(ui, "fit", "Page fit", &mut self.fit, FIT.iter().map(|x| x.0.to_string()).collect(), true);

            ui.add_space(12.0);
            if ui.add(egui::Button::new("⚙  Printer’s own options…").min_size(egui::vec2(0.0, 30.0))).clicked() {
                self.open_driver_options();
            }
            let has_dm = self.driver_devmode(&self.printer).is_some();
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    RichText::new(if has_dm {
                        "Using the options you set in the printer’s window."
                    } else {
                        "Paper type, output tray, booklets and more."
                    })
                    .size(11.5)
                    .color(p.muted),
                );
                if has_dm && ui.link(RichText::new("Reset").size(11.5)).clicked() {
                    self.reset_driver_options();
                }
            });
        });
    }

    fn list_card(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let p = self.pal;
        theme::card(&p).show(ui, |ui| {
            ui.set_min_size(ui.available_size());
            let busy = self.busy;
            ui.horizontal(|ui| {
                ui.add_enabled_ui(!busy, |ui| {
                    if ui.add(tool_button("+  Add files")).clicked() {
                        self.add_files(ctx);
                    }
                    if ui.add(tool_button("📁  Add folder")).clicked() {
                        self.add_folder(ctx);
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.add(tool_button("Clear list")).clicked() {
                            self.jobs.clear();
                            self.selected.clear();
                        }
                        if ui.add(tool_button("Remove")).clicked() {
                            self.remove_selected();
                        }
                        ui.add_space(6.0);
                        if ui.add(egui::Button::new("▼").min_size(egui::vec2(34.0, 30.0))).on_hover_text("Move down").clicked() {
                            self.move_selected(1);
                        }
                        if ui.add(egui::Button::new("▲").min_size(egui::vec2(34.0, 30.0))).on_hover_text("Move up").clicked() {
                            self.move_selected(-1);
                        }
                    });
                });
            });
            ui.add_space(6.0);

            // Footer switches, laid out from the bottom so the list takes the rest.
            ui.with_layout(Layout::bottom_up(Align::Min), |ui| {
                ui.horizontal(|ui| {
                    theme::switch(ui, &p, &mut self.subfolders, "Include subfolders when adding a folder");
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        theme::switch(ui, &p, &mut self.clear_done, "Remove printed files from the list");
                    });
                });
                ui.add_space(6.0);
                ui.with_layout(Layout::top_down(Align::Min), |ui| self.table(ui));
            });
        });
    }

    fn table(&mut self, ui: &mut egui::Ui) {
        use egui_extras::{Column, TableBuilder};
        let p = self.pal;
        let height = ui.available_height();
        let rect_top = ui.cursor().min;
        let mut clicked: Option<(usize, egui::Modifiers)> = None;
        let jobs_len = self.jobs.len();
        let texts: Vec<[String; 6]> = self
            .jobs
            .iter()
            .enumerate()
            .map(|(k, j)| {
                [
                    (k + 1).to_string(),
                    j.name.clone(),
                    match (j.counted, j.pages) {
                        (false, _) => "…".into(),
                        (true, Some(n)) => n.to_string(),
                        (true, None) => "?".into(),
                    },
                    common::short_folder(&j.folder),
                    self.archive_text(j),
                    j.status.clone(),
                ]
            })
            .collect();
        let status_colour = |s: &str| {
            if s == "Printed" {
                p.ok
            } else if s.starts_with("Failed") {
                p.err
            } else if s == "Printing…" {
                p.accent
            } else {
                p.text
            }
        };
        ui.scope(|ui| {
            ui.style_mut().visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, p.border);
            TableBuilder::new(ui)
                .striped(true)
                .resizable(false)
                .sense(Sense::click())
                .cell_layout(Layout::left_to_right(Align::Center))
                .auto_shrink([false, false])
                .min_scrolled_height(0.0)
                .max_scroll_height(height - 32.0)
                .column(Column::exact(32.0))
                .column(Column::remainder().at_least(140.0).clip(true))
                .column(Column::exact(50.0))
                .column(Column::initial(110.0).at_least(80.0).clip(true))
                .column(Column::exact(150.0).clip(true))
                .column(Column::exact(96.0).clip(true))
                .header(30.0, |mut h| {
                    for t in ["#", "File", "Pages", "Folder", "Goes to", "Status"] {
                        h.col(|ui| {
                            ui.label(RichText::new(t).font(theme::semibold(12.0)).color(p.muted));
                        });
                    }
                })
                .body(|body| {
                    body.rows(30.0, jobs_len, |mut row| {
                        let k = row.index();
                        row.set_selected(self.selected.contains(&k));
                        let t = &texts[k];
                        for (c, text) in t.iter().enumerate() {
                            row.col(|ui| {
                                let mut rt = RichText::new(text);
                                if c == 5 {
                                    rt = rt.color(status_colour(text));
                                    if text == "Printing…" {
                                        rt = rt.font(theme::semibold(13.5));
                                    }
                                } else if c == 4 && text.starts_with('⚠') {
                                    rt = rt.color(p.warn);
                                } else if c == 0 || c == 2 {
                                    rt = rt.color(p.muted);
                                }
                                ui.add(egui::Label::new(rt).truncate().selectable(false));
                            });
                        }
                        if row.response().clicked() {
                            clicked = Some((k, row.response().ctx.input(|i| i.modifiers)));
                        }
                    });
                });
        });
        if let Some((k, m)) = clicked {
            if m.shift {
                let a = self.anchor.unwrap_or(k);
                let (lo, hi) = (a.min(k), a.max(k));
                if !m.ctrl {
                    self.selected.clear();
                }
                self.selected.extend(lo..=hi);
            } else if m.ctrl || m.command {
                if !self.selected.remove(&k) {
                    self.selected.insert(k);
                }
                self.anchor = Some(k);
            } else {
                self.selected.clear();
                self.selected.insert(k);
                self.anchor = Some(k);
            }
        }
        if self.jobs.is_empty() {
            let area = egui::Rect::from_min_size(rect_top + egui::vec2(0.0, 30.0), egui::vec2(ui.available_width().max(300.0), (height - 30.0).max(120.0)));
            draw_empty_hint(ui, &p, area);
        }
    }

    fn archive_card(&mut self, ui: &mut egui::Ui) {
        let p = self.pal;
        let busy = self.busy;
        theme::card(&p).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(RichText::new("ARCHIVE AFTER PRINTING").font(theme::semibold(12.0)).color(p.accent));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.add_enabled_ui(!busy, |ui| {
                        theme::switch(ui, &p, &mut self.archive_on, "Sort printed files into folders");
                    });
                });
            });
            ui.add_space(4.0);
            let mut changed = false;
            ui.add_enabled_ui(!busy && self.archive_on, |ui| {
                let mut remove = None;
                egui::ScrollArea::vertical().id_salt("rules").max_height(104.0).auto_shrink([false, true]).show(ui, |ui| {
                    if self.rules.is_empty() {
                        ui.label(
                            RichText::new("No rules yet. Add one to send files whose name matches to a folder of your choice.")
                                .size(12.0)
                                .color(p.muted),
                        );
                    }
                    for (k, rule) in self.rules.iter_mut().enumerate() {
                        ui.horizontal(|ui| {
                            ui.allocate_ui_with_layout(egui::vec2(150.0, 28.0), Layout::left_to_right(Align::Center), |ui| {
                                ui.set_min_width(150.0);
                                egui::ComboBox::from_id_salt(("rule-how", k)).selected_text(rule.how.label()).width(138.0).show_ui(ui, |ui| {
                                    for m in Match::ALL {
                                        ui.selectable_value(&mut rule.how, m, m.label());
                                    }
                                });
                            });
                            ui.add_sized([136.0, 28.0], egui::TextEdit::singleline(&mut rule.text).hint_text("text").margin(egui::vec2(8.0, 5.0)));
                            ui.label(RichText::new("→").color(p.muted));
                            let gap = ui.spacing().item_spacing.x;
                            let w = (ui.available_width() - 84.0 - 30.0 - 2.0 * gap - 16.0).max(80.0);
                            ui.add_sized([w, 28.0], egui::TextEdit::singleline(&mut rule.folder).hint_text("folder").margin(egui::vec2(8.0, 5.0)));
                            if ui.add_sized([84.0, 28.0], egui::Button::new("Choose…")).clicked() {
                                if let Some(f) = choose_folder(&rule.folder, "Folder for files that match this rule") {
                                    rule.folder = f;
                                    changed = true;
                                }
                            }
                            if ui.add_sized([30.0, 28.0], egui::Button::new("✕")).on_hover_text("Remove this rule").clicked() {
                                remove = Some(k);
                            }
                        });
                    }
                });
                if let Some(k) = remove {
                    self.rules.remove(k);
                    changed = true;
                }
                ui.horizontal(|ui| {
                    if ui.add(egui::Button::new("+  Add rule").min_size(egui::vec2(0.0, 28.0))).clicked() {
                        self.rules.push(Rule { how: Match::Starts, text: String::new(), folder: String::new() });
                    }
                });
                ui.horizontal(|ui| {
                    field_label(ui, "Other files go to", 138.0);
                    let gap = ui.spacing().item_spacing.x;
                    let w = (ui.available_width() - 84.0 - 30.0 - 2.0 * gap - 16.0).max(80.0);
                    ui.add_sized(
                        [w, 28.0],
                        egui::TextEdit::singleline(&mut self.archive_other).hint_text("(leave them where they are)").margin(egui::vec2(8.0, 5.0)),
                    );
                    if ui.add_sized([84.0, 28.0], egui::Button::new("Choose…")).clicked() {
                        if let Some(f) = choose_folder(&self.archive_other, "Folder for all other printed files") {
                            self.archive_other = f;
                            changed = true;
                        }
                    }
                    if ui.add_sized([30.0, 28.0], egui::Button::new("✕")).on_hover_text("Leave other files where they are").clicked() {
                        self.archive_other.clear();
                        changed = true;
                    }
                });
                ui.horizontal(|ui| {
                    field_label(ui, "After printing", 138.0);
                    changed |= ui.radio_value(&mut self.archive_move, false, "Copy the file").changed();
                    ui.add_space(8.0);
                    changed |= ui.radio_value(&mut self.archive_move, true, "Move the file").changed();
                    ui.add_space(18.0);
                    let label = format!("A subfolder per month ({})", common::year_month());
                    changed |= theme::switch(ui, &p, &mut self.archive_monthly, &label).changed();
                });
            });
            if changed {
                self.save_settings();
            }
            ui.label(
                RichText::new(
                    "The first matching rule wins. Files are only archived after they print, and a different file with the same name is never overwritten.",
                )
                .size(11.5)
                .color(p.muted),
            );
        });
    }

    fn central(&mut self, ctx: &egui::Context) {
        let p = self.pal;
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(p.bg).inner_margin(egui::Margin { left: 14, right: 12, top: 14, bottom: 14 }))
            .show(ctx, |ui| {
                egui::TopBottomPanel::bottom("archive")
                    .frame(egui::Frame::new().inner_margin(egui::Margin { left: 0, right: 0, top: 12, bottom: 0 }))
                    .show_separator_line(false)
                    .show_inside(ui, |ui| self.archive_card(ui));
                egui::CentralPanel::default().frame(egui::Frame::new()).show_inside(ui, |ui| self.list_card(ui, ctx));
            });
    }

    fn drop_overlay(&mut self, ctx: &egui::Context) {
        let hovering = ctx.input(|i| !i.raw.hovered_files.is_empty());
        let dropped: Vec<PathBuf> = ctx.input(|i| i.raw.dropped_files.iter().filter_map(|f| f.path.clone()).collect());
        if !dropped.is_empty() {
            self.add_paths(&dropped, ctx);
        }
        if hovering {
            let p = self.pal;
            let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("drop")));
            let r = ctx.screen_rect().shrink(10.0);
            painter.rect_filled(r, CornerRadius::same(16), p.accent.gamma_multiply(0.12));
            painter.rect_stroke(r, CornerRadius::same(16), Stroke::new(2.0, p.accent), egui::StrokeKind::Inside);
            painter.text(r.center(), egui::Align2::CENTER_CENTER, if self.busy { "Wait until printing has finished" } else { "Drop to add PDF files" },
                         theme::semibold(22.0), p.accent);
        }
    }
}

/// Archive rules from the settings; the first time, the edition's default rules
/// (with folders saved by the earlier, fixed archive when there are any).
fn load_rules(settings: &Settings) -> Vec<Rule> {
    if let Some(list) = settings.0.get("archive_rules").and_then(|v| v.as_array()) {
        return list
            .iter()
            .map(|r| Rule {
                how: Match::from_key(r.get("match").and_then(|v| v.as_str()).unwrap_or("")),
                text: r.get("text").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                folder: r.get("folder").and_then(|v| v.as_str()).unwrap_or("").to_string(),
            })
            .collect();
    }
    let old = settings.obj("archive_folders");
    brand::DEFAULT_RULES
        .iter()
        .map(|(how, text, key)| Rule {
            how: Match::from_key(how),
            text: text.to_string(),
            folder: old.get(*key).and_then(|v| v.as_str()).unwrap_or("").to_string(),
        })
        .collect()
}

/// A small printer badge for the top bar (editions without a logo).
fn printer_badge(ui: &mut egui::Ui, p: &Palette) {
    let (r, _) = ui.allocate_exact_size(egui::vec2(36.0, 36.0), Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(r, CornerRadius::same(10), p.accent);
    let c = r.center();
    let white = Color32::WHITE;
    let soft = Color32::from_white_alpha(190);
    let at = |x: f32, y: f32, w: f32, h: f32| egui::Rect::from_min_size(c + egui::vec2(x, y), egui::vec2(w, h));
    painter.rect_filled(at(-6.5, -11.0, 13.0, 7.0), CornerRadius::same(2), white);
    painter.rect_filled(at(-11.0, -5.0, 22.0, 11.0), CornerRadius::same(3), soft);
    painter.rect_filled(at(-6.5, 1.0, 13.0, 10.0), CornerRadius::same(2), white);
    painter.rect_filled(at(-4.5, 4.0, 9.0, 1.5), CornerRadius::same(1), p.accent);
    painter.rect_filled(at(-4.5, 7.0, 6.0, 1.5), CornerRadius::same(1), p.accent);
}

fn field_label(ui: &mut egui::Ui, text: &str, width: f32) {
    ui.allocate_ui_with_layout(egui::vec2(width, 28.0), Layout::left_to_right(Align::Center), |ui| {
        ui.set_min_size(egui::vec2(width, 28.0));
        ui.add(egui::Label::new(text).truncate());
    });
}

fn tool_button(text: &str) -> egui::Button<'_> {
    egui::Button::new(text).min_size(egui::vec2(0.0, 30.0))
}

fn progress_bar(ui: &mut egui::Ui, p: &Palette, frac: f32) {
    let w = ui.available_width();
    let (r, _) = ui.allocate_exact_size(egui::vec2(w, 10.0), Sense::hover());
    ui.painter().rect_filled(r, CornerRadius::same(5), p.trough);
    if frac > 0.0 {
        let mut f = r;
        f.set_width((r.width() * frac.clamp(0.0, 1.0)).max(10.0));
        ui.painter().rect_filled(f, CornerRadius::same(5), p.accent);
    }
}

fn draw_empty_hint(ui: &egui::Ui, p: &Palette, area: egui::Rect) {
    let painter = ui.painter_at(area);
    let c = area.center() - egui::vec2(0.0, 30.0);
    // A page with a folded corner.
    let page = egui::Rect::from_center_size(c, egui::vec2(46.0, 60.0));
    painter.rect(page, CornerRadius::same(6), p.accent_soft, Stroke::new(1.5, p.accent.gamma_multiply(0.5)), egui::StrokeKind::Inside);
    for (k, w) in [(0, 20.0), (1, 30.0), (2, 26.0)] {
        let y = page.top() + 16.0 + k as f32 * 7.0;
        painter.rect_filled(egui::Rect::from_min_size(egui::pos2(page.left() + 8.0, y), egui::vec2(w, 3.0)), CornerRadius::same(1), p.accent.gamma_multiply(0.5));
    }
    painter.rect_filled(egui::Rect::from_min_size(egui::pos2(page.left() + 8.0, page.bottom() - 18.0), egui::vec2(30.0, 10.0)), CornerRadius::same(3), p.accent);
    painter.text(c + egui::vec2(0.0, 52.0), egui::Align2::CENTER_CENTER, "Drop PDF files or folders here", theme::semibold(16.0), p.text);
    painter.text(c + egui::vec2(0.0, 74.0), egui::Align2::CENTER_CENTER, "or use “Add files” / “Add folder” above",
                 egui::FontId::proportional(12.0), p.muted);
}

fn message(text: &str) {
    rfd::MessageDialog::new().set_title(APP_NAME).set_description(text).set_level(rfd::MessageLevel::Info).show();
}

fn ask(text: &str) -> bool {
    rfd::MessageDialog::new()
        .set_title(APP_NAME)
        .set_description(text)
        .set_buttons(rfd::MessageButtons::YesNo)
        .show()
        == rfd::MessageDialogResult::Yes
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        if self.hwnd == 0 {
            use raw_window_handle::{HasWindowHandle, RawWindowHandle};
            if let Ok(h) = frame.window_handle() {
                if let RawWindowHandle::Win32(w) = h.as_raw() {
                    self.hwnd = w.hwnd.get();
                }
            }
        }
        self.poll_events();
        if self.busy || self.jobs.iter().any(|j| !j.counted) {
            ctx.request_repaint_after(std::time::Duration::from_millis(200));
        }

        // Asking before closing while printing.
        if ctx.input(|i| i.viewport().close_requested()) {
            if self.busy && self.shot.is_none() && !ask("Printing is still running. Quit anyway?") {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            } else if self.shot.is_none() {
                self.save_settings();
            }
        }

        self.header(ctx);
        self.bottom_bar(ctx);
        self.settings_panel(ctx);
        self.central(ctx);
        self.drop_overlay(ctx);

        if ctx.input(|i| i.key_pressed(egui::Key::Delete)) && !ctx.wants_keyboard_input() {
            self.remove_selected();
        }
        if ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::A)) && !ctx.wants_keyboard_input() {
            self.selected = (0..self.jobs.len()).collect();
        }

        let now = ctx.input(|i| i.time);
        if self.started == 0.0 {
            self.started = now.max(0.001);
        }
        if self.shot.is_none() && !self.offered_install && now - self.started > 0.7 {
            self.offered_install = true;
            self.offer_install(false);
        } else if self.shot.is_none() && !self.offered_install {
            ctx.request_repaint_after(std::time::Duration::from_millis(300));
        }

        // --screenshot: draw about 120 frames, save the window's picture, close.
        if let Some(shot) = &mut self.shot {
            ctx.request_repaint();
            shot.frames += 1;
            if shot.frames == 120 && !shot.asked {
                shot.asked = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
            }
            let image = ctx.input(|i| {
                i.raw.events.iter().find_map(|e| match e {
                    egui::Event::Screenshot { image, .. } => Some(image.clone()),
                    _ => None,
                })
            });
            if let Some(img) = image {
                let _ = crate::save_png(&shot.path, &img);
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            } else if shot.asked && shot.frames > 400 {
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
                shot.frames = 121;
            }
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        if self.shot.is_none() {
            self.save_settings();
        }
    }
}

fn choose_folder(current: &str, title: &str) -> Option<String> {
    let mut d = rfd::FileDialog::new().set_title(title);
    if std::path::Path::new(current.trim()).is_dir() {
        d = d.set_directory(current.trim());
    }
    d.pick_folder().map(|f| f.to_string_lossy().to_string())
}
