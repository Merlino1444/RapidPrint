# vendor

RapidPrint needs **pdfium.dll** (the PDF engine) next to `RapidPrint.exe`. While developing, it also
looks for it in this folder.

It isn't stored in the repository. To get it:

1. Download `pdfium-win-x64.tgz` from <https://github.com/bblanchon/pdfium-binaries/releases/latest>.
2. Extract it (`tar -xzf pdfium-win-x64.tgz`) and copy `bin\pdfium.dll` here.

The GitHub Actions build does this automatically. `PDFium-LICENSE.txt` is PDFium's licence and ships
with every release.
