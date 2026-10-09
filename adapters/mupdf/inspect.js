// Pinned Debian bookworm MuPDF API: bound() includes CropBox, rotation and UserUnit.
var document = new Document("/input/document");
if (document.needsPassword()) throw new Error("encrypted_pdf_unsupported");
var count = document.countPages();
if (count < 1 || count > 100) throw new Error("page_limit");
var pages = [];
for (var index = 0; index < count; index++) {
    var page = document.loadPage(index);
    var bounds = page.bound();
    // Include annotations/widgets; parse content without creating a raster.
    var display = page.toDisplayList(false);
    pages.push({page: index + 1, widthPoints: bounds[2] - bounds[0], heightPoints: bounds[3] - bounds[1]});
}
print(JSON.stringify({schemaVersion: 1, inspectorRevision: "jrip-mupdf-inspection-v1", pages: pages, contentParsed: true, repaired: false, encrypted: false}));
