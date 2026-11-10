# Third-party notices

RapidPrint is built on these open-source projects. Thank you to their authors.

| Component | Used for | License |
|---|---|---|
| [PDFium](https://pdfium.googlesource.com/pdfium/) (`pdfium.dll`, prebuilt by [pdfium-binaries](https://github.com/bblanchon/pdfium-binaries)) | Drawing PDF pages | BSD-3-Clause, plus Apache-2.0 for some parts. The text ships with the program as `PDFium-LICENSE.txt` |
| [egui / eframe / egui_extras](https://github.com/emilk/egui) | The window and controls | MIT or Apache-2.0 |
| [winit](https://github.com/rust-windowing/winit), [glutin](https://github.com/rust-windowing/glutin), [glow](https://github.com/grovesNL/glow) | Window, OpenGL | Apache-2.0 / MIT |
| [rfd](https://github.com/PolyMeilex/rfd) | File, folder and message dialogs | MIT |
| [serde_json](https://github.com/serde-rs/json) | Settings file | MIT or Apache-2.0 |
| [libloading](https://github.com/nagisa/rust_libloading) | Loading Windows and PDFium functions | ISC |
| [png](https://github.com/image-rs/image-png) | Pictures | MIT or Apache-2.0 |
| [roxmltree](https://github.com/RazrFalcon/roxmltree) | Reading printer capabilities (XML) | MIT or Apache-2.0 |
| [base64](https://github.com/marshallpierce/rust-base64) | Saving printer settings | MIT or Apache-2.0 |
| [winreg](https://github.com/gentoo90/winreg-rs) | Settings › Apps entry, dark-mode detection | MIT |
| [raw-window-handle](https://github.com/rust-windowing/raw-window-handle) | Parent window for the printer's dialog | MIT, Apache-2.0 or Zlib |

These depend on further crates. `Cargo.lock` lists them all with exact versions, and
`cargo tree` or [cargo-about](https://github.com/EmbarkStudios/cargo-about) shows their licenses. The
Segoe UI font is not included: it is used from Windows when present.
