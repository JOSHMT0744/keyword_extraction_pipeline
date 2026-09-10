#!/usr/bin/env python3
"""Generate the parsing fixture corpus.

Fixtures are committed alongside this script so tests need no generation step, but the
script is committed too so they can be regenerated and reviewed rather than being opaque
binaries. PDFs are produced with PyMuPDF, a different implementation from the reader
under test, so a passing test means two independent implementations agree.

Every fixture carries planted identifiers (DS-2291, HEK293T, SOP-114) whose recovery the
integration tests assert.
"""
import pathlib
import zipfile

import fitz

OUT = pathlib.Path(__file__).resolve().parent.parent / "tests" / "fixtures"
OUT.mkdir(parents=True, exist_ok=True)


def simple_pdf():
    doc = fitz.open()
    page = doc.new_page()
    page.insert_text((72, 100), "Column Regeneration Report", fontsize=16)
    body = (
        "Batch DS-2291 was purified on the MabSelect SuRe column.\n"
        "Cell line HEK293T was cultured according to SOP-114 revision 3.\n"
        "The chromatography step achieved the expected yield and the\n"
        "column was regenerated using the standard procedure."
    )
    page.insert_text((72, 140), body, fontsize=11)
    doc.save(OUT / "simple.pdf")
    doc.close()


def two_column_pdf():
    """Multi-column layout. Reading order must not interleave the columns."""
    doc = fitz.open()
    page = doc.new_page()
    left = "The left column discusses DS-2291 at length and continues for several lines of text here."
    right = "The right column instead covers HEK293T and its culture conditions in similar detail."
    page.insert_textbox(fitz.Rect(50, 80, 280, 400), left, fontsize=11)
    page.insert_textbox(fitz.Rect(310, 80, 545, 400), right, fontsize=11)
    doc.save(OUT / "two_column.pdf")
    doc.close()


def scanned_pdf():
    """Image-only page: parses cleanly, has no text layer. Must not read as 'bland'."""
    doc = fitz.open()
    page = doc.new_page()
    pix = fitz.Pixmap(fitz.csRGB, fitz.IRect(0, 0, 60, 30))
    pix.set_rect(pix.irect, (235, 235, 235))
    page.insert_image(fitz.Rect(72, 72, 472, 272), pixmap=pix)
    doc.save(OUT / "scanned.pdf")
    doc.close()


CONTENT_TYPES_DOCX = """<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
<Default Extension="xml" ContentType="application/xml"/>
<Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/>
</Types>"""

RELS = """<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="{target}"/>
</Relationships>"""

DOCUMENT_XML = """<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>
<w:p><w:r><w:t>Column Regeneration SOP</w:t></w:r></w:p>
<w:p><w:r><w:t>Batch </w:t></w:r><w:r><w:t>DS-2291</w:t></w:r><w:r><w:t> was purified on MabSelect SuRe.</w:t></w:r></w:p>
<w:p><w:r><w:t>Cell line HEK293T follows SOP-114 for chromatography and regeneration.</w:t></w:r></w:p>
</w:body></w:document>"""


def docx():
    with zipfile.ZipFile(OUT / "simple.docx", "w", zipfile.ZIP_DEFLATED) as z:
        z.writestr("[Content_Types].xml", CONTENT_TYPES_DOCX)
        z.writestr("_rels/.rels", RELS.format(target="word/document.xml"))
        z.writestr("word/document.xml", DOCUMENT_XML)


CONTENT_TYPES_XLSX = """<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
<Default Extension="xml" ContentType="application/xml"/>
<Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>
<Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>
</Types>"""

WORKBOOK_XML = """<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"
 xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
<sheets><sheet name="Runs" sheetId="1" r:id="rId1"/></sheets></workbook>"""

WORKBOOK_RELS = """<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>
</Relationships>"""


def _row(idx, cells):
    out = [f'<row r="{idx}">']
    for col, value in zip("ABCDEFG", cells):
        if isinstance(value, str):
            out.append(f'<c r="{col}{idx}" t="inlineStr"><is><t>{value}</t></is></c>')
        else:
            out.append(f'<c r="{col}{idx}"><v>{value}</v></c>')
    out.append("</row>")
    return "".join(out)


def xlsx():
    rows = "".join([
        _row(1, ["Batch", "Cell line", "Yield"]),
        _row(2, ["DS-2291", "HEK293T", 91.4]),
        _row(3, ["DS-2292", "HEK293T", 88.2]),
    ])
    sheet = (
        '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
        '<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">'
        f"<sheetData>{rows}</sheetData></worksheet>"
    )
    with zipfile.ZipFile(OUT / "simple.xlsx", "w", zipfile.ZIP_DEFLATED) as z:
        z.writestr("[Content_Types].xml", CONTENT_TYPES_XLSX)
        z.writestr("_rels/.rels", RELS.format(target="xl/workbook.xml"))
        z.writestr("xl/workbook.xml", WORKBOOK_XML)
        z.writestr("xl/_rels/workbook.xml.rels", WORKBOOK_RELS)
        z.writestr("xl/worksheets/sheet1.xml", sheet)


CONTENT_TYPES_PPTX = """<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
<Default Extension="xml" ContentType="application/xml"/>
<Override PartName="/ppt/presentation.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml"/>
<Override PartName="/ppt/slides/slide1.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.slide+xml"/>
</Types>"""

SLIDE_XML = """<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:sld xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"
 xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main">
<p:cSld><p:spTree>
<p:sp><p:txBody><a:p><a:r><a:t>Purification of DS-2291</a:t></a:r></a:p>
<a:p><a:r><a:t>Performed on MabSelect SuRe under SOP-114</a:t></a:r></a:p></p:txBody></p:sp>
</p:spTree></p:cSld></p:sld>"""


def pptx():
    with zipfile.ZipFile(OUT / "simple.pptx", "w", zipfile.ZIP_DEFLATED) as z:
        z.writestr("[Content_Types].xml", CONTENT_TYPES_PPTX)
        z.writestr("_rels/.rels", RELS.format(target="ppt/presentation.xml"))
        z.writestr("ppt/presentation.xml",
                   '<?xml version="1.0"?><p:presentation xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main"/>')
        z.writestr("ppt/slides/slide1.xml", SLIDE_XML)


def email_thread():
    """A quoted thread. Term frequency here is inflated until quotes are stripped."""
    (OUT / "thread.eml").write_bytes(
        b"From: alice@example.com\r\n"
        b"To: bob@example.com\r\n"
        b"Message-ID: <QQ7F3K2M@mail.example.com>\r\n"
        b"Subject: Re: Batch DS-2291 release\r\n"
        b"MIME-Version: 1.0\r\n"
        b"Content-Type: text/plain; charset=utf-8\r\n"
        b"\r\n"
        b"Confirmed, the column was regenerated per SOP-114.\r\n"
        b"\r\n"
        b"On Tue, Bob wrote:\r\n"
        b"> Has batch DS-2291 been released yet?\r\n"
        b"> The HEK293T culture is ready.\r\n"
        b">\r\n"
        b"> > Original question about DS-2291 from earlier.\r\n"
        b"\r\n"
        b"-- \r\n"
        b"Alice Example | Process Development | example.com\r\n"
    )


def plain_text():
    (OUT / "simple.txt").write_text(
        "Column Regeneration Notes\n\n"
        "Batch DS-2291 was purified on the MabSelect SuRe column. Cell line HEK293T\n"
        "was cultured according to SOP-114 revision 3. The chromatography step\n"
        "achieved the expected yield and the column was regenerated afterwards.\n",
        encoding="utf-8",
    )


if __name__ == "__main__":
    simple_pdf()
    two_column_pdf()
    scanned_pdf()
    docx()
    xlsx()
    pptx()
    email_thread()
    plain_text()
    for f in sorted(OUT.iterdir()):
        print(f"{f.name:20s} {f.stat().st_size:>8d} bytes")
