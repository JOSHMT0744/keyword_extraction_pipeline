//! OOXML formats. docx and pptx are zip containers of XML read directly; xlsx goes
//! through `calamine`, which already handles shared strings and the cell type zoo.

use std::io::{Cursor, Read};

use calamine::{Data, Reader, Xlsx};
use quick_xml::events::Event;

use super::{RawText, SourceKind, TextExtractor};
use crate::{config::Config, error::ExtractError};

pub struct DocxExtractor;
pub struct PptxExtractor;
pub struct XlsxExtractor;

impl TextExtractor for DocxExtractor {
    fn extract(&self, bytes: &[u8], _cfg: &Config) -> Result<RawText, ExtractError> {
        let parts = read_parts(bytes, |n| n == "word/document.xml")?;
        let mut text = String::new();
        for part in parts {
            text.push_str(&text_from_ooxml(&part, "t", "p")?);
        }
        Ok(RawText::new(text, SourceKind::WordProcessing))
    }
}

impl TextExtractor for PptxExtractor {
    fn extract(&self, bytes: &[u8], _cfg: &Config) -> Result<RawText, ExtractError> {
        // Slide order matters for reading order, and zip entry order does not guarantee
        // it, so parts are sorted by name before concatenation. `slide10` sorting before
        // `slide2` is accepted: it is stable, which is what determinism requires, and
        // slide ordering does not affect which keywords are found.
        let parts = read_parts(bytes, |n| {
            n.starts_with("ppt/slides/slide") && n.ends_with(".xml")
        })?;
        let mut text = String::new();
        for part in parts {
            text.push_str(&text_from_ooxml(&part, "t", "p")?);
            text.push('\n');
        }
        Ok(RawText::new(text, SourceKind::Presentation))
    }
}

impl TextExtractor for XlsxExtractor {
    fn extract(&self, bytes: &[u8], _cfg: &Config) -> Result<RawText, ExtractError> {
        let mut wb: Xlsx<_> = Xlsx::new(Cursor::new(bytes.to_vec()))
            .map_err(|e| ExtractError::Parse(format!("xlsx: {e:?}")))?;

        let mut text = String::new();
        let names = wb.sheet_names().to_vec();
        for name in names {
            let Ok(range) = wb.worksheet_range(&name) else { continue };
            text.push_str(&name);
            text.push('\n');
            for row in range.rows() {
                let cells: Vec<String> = row.iter().filter_map(cell_text).collect();
                if cells.is_empty() {
                    continue;
                }
                text.push_str(&cells.join("\t"));
                text.push('\n');
            }
        }
        Ok(RawText::new(text, SourceKind::Spreadsheet))
    }
}

/// Render a cell as text, dropping cells that carry no lexical content.
///
/// Bare numbers are excluded: a spreadsheet of measurements would otherwise flood the
/// identifier stage with numerics that carry no identifying power on their own.
fn cell_text(d: &Data) -> Option<String> {
    match d {
        Data::String(s) => {
            let t = s.trim();
            (!t.is_empty()).then(|| t.to_string())
        }
        Data::DateTimeIso(s) | Data::DurationIso(s) => Some(s.clone()),
        Data::Bool(_) | Data::Int(_) | Data::Float(_) | Data::DateTime(_) => None,
        Data::Error(_) | Data::Empty => None,
    }
}

/// Read every zip entry whose name satisfies `want`, sorted by name for determinism.
fn read_parts(bytes: &[u8], want: impl Fn(&str) -> bool) -> Result<Vec<String>, ExtractError> {
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes.to_vec()))
        .map_err(|e| ExtractError::Parse(format!("zip: {e}")))?;

    let mut names: Vec<String> = (0..zip.len())
        .filter_map(|i| zip.by_index_raw(i).ok().map(|f| f.name().to_string()))
        .filter(|n| want(n))
        .collect();
    names.sort();

    let mut out = Vec::with_capacity(names.len());
    for name in names {
        let mut f = zip
            .by_name(&name)
            .map_err(|e| ExtractError::Parse(format!("zip entry {name}: {e}")))?;
        let mut buf = String::new();
        f.read_to_string(&mut buf)
            .map_err(|e| ExtractError::Parse(format!("zip entry {name}: {e}")))?;
        out.push(buf);
    }
    Ok(out)
}

/// Pull text from an OOXML part.
///
/// `text_local` is the local name of the run-text element (`w:t`, `a:t`) and
/// `break_local` the element that ends a paragraph (`w:p`). Matching on `local_name`
/// ignores namespace prefixes, which vary between producers.
fn text_from_ooxml(xml: &str, text_local: &str, break_local: &str) -> Result<String, ExtractError> {
    let mut reader = quick_xml::Reader::from_str(xml);
    reader.config_mut().trim_text(false);

    let mut out = String::new();
    let mut in_text = false;

    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => {
                if e.name().local_name().into_inner() == text_local {
                    in_text = true;
                }
            }
            Ok(Event::End(e)) => {
                let local = e.name().local_name();
                let local = local.into_inner();
                if local == text_local {
                    in_text = false;
                } else if local == break_local {
                    out.push('\n');
                }
            }
            Ok(Event::Text(e)) if in_text => {
                out.push_str(&e.xml10_content());
            }
            Ok(Event::Eof) => break,
            Ok(_) => {}
            Err(e) => return Err(ExtractError::Parse(format!("ooxml: {e}"))),
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_runs_and_paragraph_breaks() {
        let xml = r#"<w:document xmlns:w="x"><w:body>
            <w:p><w:r><w:t>Batch </w:t></w:r><w:r><w:t>DS-2291</w:t></w:r></w:p>
            <w:p><w:r><w:t>Second line</w:t></w:r></w:p>
        </w:body></w:document>"#;
        let out = text_from_ooxml(xml, "t", "p").unwrap();
        assert!(out.contains("Batch DS-2291"), "runs must join without a gap: {out:?}");
        assert!(out.contains('\n'), "paragraphs must break: {out:?}");
    }

    #[test]
    fn ignores_namespace_prefix_differences() {
        let xml = r#"<document xmlns="x"><p><r><t>Capto S</t></r></p></document>"#;
        assert!(text_from_ooxml(xml, "t", "p").unwrap().contains("Capto S"));
    }

    #[test]
    fn numeric_cells_are_dropped_but_strings_kept() {
        assert_eq!(cell_text(&Data::Float(2291.0)), None);
        assert_eq!(cell_text(&Data::String("DS-2291".into())), Some("DS-2291".into()));
        assert_eq!(cell_text(&Data::String("   ".into())), None);
    }
}
