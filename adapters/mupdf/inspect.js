// Pinned Debian bookworm MuPDF API: bound() includes CropBox, rotation and UserUnit.
var document = new Document(scriptArgs.length ? scriptArgs[0] : "/input/document");
if (document.needsPassword()) throw new Error("encrypted_pdf_unsupported");
var count = document.countPages();
if (count < 1 || count > 100) throw new Error("page_limit");
var pages = [];
// Text operations observed while replaying parsed pages, not declaration metadata.
var usage = {schemaVersion: 1, profile: "org.jrip.mupdf-font-usage-v1", engine: "mupdf",
    phase: "pdf-inspection", pageCount: count, textOperations: 0, glyphCallbacks: 0, fonts: []};
function observe(text, operation, pageNumber) {
    if (++usage.textOperations > 100000) throw new Error("font_usage_operation_limit");
    text.walk({showGlyph: function(font, trm, gid, ucs, wmode) {
        if (++usage.glyphCallbacks > 1000000) throw new Error("font_usage_glyph_limit");
        var name = font.getName();
        if (typeof name !== "string" || name.length > 128) throw new Error("font_usage_name_limit");
        var record = null;
        for (var n = 0; n < usage.fonts.length; n++) {
            if (usage.fonts[n].name === name) {record = usage.fonts[n]; break;}
        }
        if (!record) {
            if (usage.fonts.length >= 64) throw new Error("font_usage_font_limit");
            record = {name: name, glyphCallbacks: 0, glyphInstances: 0, negativeGlyphIds: 0,
                zeroGlyphIds: 0, horizontalGlyphs: 0, verticalGlyphs: 0, firstPage: pageNumber,
                lastPage: pageNumber, pageCount: 0, glyphIdSample: [], sampleTruncated: false,
                operations: {fill: 0, stroke: 0, clip: 0, clipStroke: 0, ignore: 0}};
            usage.fonts.push(record);
        }
        if (record.lastPage !== pageNumber || record.pageCount === 0) record.pageCount++;
        record.lastPage = pageNumber;
        record.glyphCallbacks++;
        record.operations[operation]++;
        if (wmode) record.verticalGlyphs++; else record.horizontalGlyphs++;
        if (gid < 0) {record.negativeGlyphIds++; return;}
        record.glyphInstances++;
        if (gid === 0) record.zeroGlyphIds++;
        if (record.glyphIdSample.indexOf(gid) < 0) {
            if (record.glyphIdSample.length < 16) record.glyphIdSample.push(gid);
            else record.sampleTruncated = true;
        }
    }});
}
function textDevice(pageNumber) {
    return {
        fillText: function(text) {observe(text, "fill", pageNumber);},
        strokeText: function(text) {observe(text, "stroke", pageNumber);},
        clipText: function(text) {observe(text, "clip", pageNumber);},
        clipStrokeText: function(text) {observe(text, "clipStroke", pageNumber);},
        ignoreText: function(text) {observe(text, "ignore", pageNumber);}
    };
}
for (var index = 0; index < count; index++) {
    var page = document.loadPage(index);
    var bounds = page.bound();
    // Include annotations/widgets; parse content without creating a raster.
    var display = page.toDisplayList(false);
    display.run(textDevice(index + 1), Identity);
    pages.push({page: index + 1, widthPoints: bounds[2] - bounds[0], heightPoints: bounds[3] - bounds[1]});
}
for (var f = 0; f < usage.fonts.length; f++) usage.fonts[f].glyphIdSample.sort(function(a,b){return a-b;});
print(JSON.stringify({schemaVersion: 1, inspectorRevision: "jrip-mupdf-inspection-v1", pages: pages, fontUsage: usage, contentParsed: true, repaired: false, encrypted: false}));
