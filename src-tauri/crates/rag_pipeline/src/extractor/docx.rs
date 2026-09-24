use super::{
    ExtractedPages,
    util::{
        TEXT_PREVIEW_MAX_BYTES, append_extracted_text, nonpaginated_page, preview_raw_bytes,
        read_bounded_bytes,
    },
};
use domain::{AppError, DocumentPreview};
use quick_xml::{
    NsReader,
    escape::unescape,
    events::Event,
    name::{LocalName, ResolveResult},
};
use std::{borrow::Cow, fs::File, io::Read, path::Path};

pub const DOCX_XML_MAX_BYTES: usize = 16 * 1024 * 1024;
const WORDPROCESSINGML_NAMESPACE: &str =
    "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
const STRICT_WORDPROCESSINGML_NAMESPACE: &str = "http://purl.oclc.org/ooxml/wordprocessingml/main";

pub fn extract_docx_pages(
    source_path: &Path,
    on_page_extracted: impl Fn(usize, usize),
) -> Result<ExtractedPages, AppError> {
    let file = File::open(source_path)
        .map_err(|e| AppError::Io(format!("DOCX extraction failed: {e}")))?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|e| AppError::Io(format!("DOCX archive is invalid: {e}")))?;
    let mut document_xml = archive
        .by_name("word/document.xml")
        .map_err(|_| AppError::Io("DOCX archive does not contain document text".into()))?;
    let declared_xml_size = document_xml.size();
    let xml = bounded_docx_xml(&mut document_xml, declared_xml_size)?;

    let page = nonpaginated_page(docx_body_text(&xml, false)?, "DOCX document")?;
    on_page_extracted(1, 1);
    Ok(ExtractedPages {
        pages: vec![(1, page)],
        total_pages: 1,
    })
}

pub fn docx_preview(source_path: &Path) -> Result<DocumentPreview, AppError> {
    let file = File::open(source_path)
        .map_err(|e| AppError::Io(format!("DOCX preview could not open source: {e}")))?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|e| AppError::Io(format!("DOCX preview archive is invalid: {e}")))?;
    let mut document_xml = archive
        .by_name("word/document.xml")
        .map_err(|_| AppError::Io("DOCX preview archive does not contain document text".into()))?;
    let (mut xml, xml_truncated) =
        preview_raw_bytes(&mut document_xml, TEXT_PREVIEW_MAX_BYTES, "DOCX preview")?;
    if xml_truncated
        && xml.len() % 2 != 0
        && (xml.starts_with(&[0xFF, 0xFE])
            || xml.starts_with(&[0xFE, 0xFF])
            || xml.starts_with(&[b'<', 0, b'?', 0])
            || xml.starts_with(&[0, b'<', 0, b'?']))
    {
        xml.pop();
    }
    let (content, text_truncated) = truncate_preview_text(docx_body_text(&xml, xml_truncated)?);

    Ok(DocumentPreview::PlainText {
        content,
        truncated: xml_truncated || text_truncated,
    })
}

fn truncate_preview_text(mut content: String) -> (String, bool) {
    if content.len() <= TEXT_PREVIEW_MAX_BYTES {
        return (content, false);
    }

    let mut boundary = TEXT_PREVIEW_MAX_BYTES;
    while !content.is_char_boundary(boundary) {
        boundary -= 1;
    }
    content.truncate(boundary);
    (content, true)
}

pub fn docx_body_text(xml: &[u8], allow_truncated_eof: bool) -> Result<String, AppError> {
    let xml = normalized_docx_xml(xml)?;
    let mut reader = NsReader::from_reader(xml.as_ref());
    reader.config_mut().trim_text(false);
    let mut buffer = Vec::new();
    let mut in_text = false;
    let mut text = String::new();

    loop {
        match reader.read_resolved_event_into(&mut buffer) {
            Ok((namespace, Event::Start(event))) => {
                if is_wordprocessing_element(&namespace, event.local_name(), "t") {
                    in_text = true;
                } else if is_wordprocessing_element(&namespace, event.local_name(), "tab") {
                    append_extracted_text(&mut text, "\t")?;
                } else if is_wordprocessing_element(&namespace, event.local_name(), "br")
                    || is_wordprocessing_element(&namespace, event.local_name(), "cr")
                {
                    append_extracted_text(&mut text, "\n")?;
                }
            }
            Ok((namespace, Event::Empty(event))) => {
                if is_wordprocessing_element(&namespace, event.local_name(), "tab") {
                    append_extracted_text(&mut text, "\t")?;
                } else if is_wordprocessing_element(&namespace, event.local_name(), "br")
                    || is_wordprocessing_element(&namespace, event.local_name(), "cr")
                    || is_wordprocessing_element(&namespace, event.local_name(), "p")
                {
                    append_extracted_text(&mut text, "\n")?;
                }
            }
            Ok((namespace, Event::End(event))) => {
                if is_wordprocessing_element(&namespace, event.local_name(), "t") {
                    in_text = false;
                } else if is_wordprocessing_element(&namespace, event.local_name(), "p") {
                    append_extracted_text(&mut text, "\n")?;
                }
            }
            Ok((_, Event::Text(event))) if in_text => {
                let decoded = event
                    .decode()
                    .map_err(|e| AppError::Io(format!("DOCX text decoding failed: {e}")))?;
                append_extracted_text(&mut text, &decoded)?;
            }
            Ok((_, Event::CData(event))) if in_text => {
                let decoded = event
                    .decode()
                    .map_err(|e| AppError::Io(format!("DOCX text decoding failed: {e}")))?;
                append_extracted_text(&mut text, &decoded)?;
            }
            Ok((_, Event::GeneralRef(event))) if in_text => {
                let reference = event
                    .decode()
                    .map_err(|e| AppError::Io(format!("DOCX text decoding failed: {e}")))?;
                let encoded = format!("&{reference};");
                let resolved = unescape(&encoded)
                    .map_err(|e| AppError::Io(format!("DOCX text decoding failed: {e}")))?;
                append_extracted_text(&mut text, &resolved)?;
            }
            Ok((_, Event::Eof)) => break,
            Ok(_) => {}
            Err(_) if allow_truncated_eof => break,
            Err(error) => return Err(AppError::Io(format!("DOCX XML parsing failed: {error}"))),
        }
        buffer.clear();
    }

    Ok(text)
}

fn is_wordprocessing_element(
    namespace: &ResolveResult<'_>,
    local_name: LocalName<'_>,
    expected_local_name: &str,
) -> bool {
    let ResolveResult::Bound(namespace) = namespace else {
        return false;
    };
    let namespace = std::str::from_utf8(namespace.as_ref()).ok();
    let local_name = std::str::from_utf8(local_name.as_ref()).ok();
    matches!(
        (namespace, local_name),
        (
            Some(WORDPROCESSINGML_NAMESPACE | STRICT_WORDPROCESSINGML_NAMESPACE),
            Some(local_name)
        ) if local_name == expected_local_name
    )
}

fn normalized_docx_xml(xml: &[u8]) -> Result<Cow<'_, [u8]>, AppError> {
    let (encoding, xml) = match xml {
        [0xFF, 0xFE, remainder @ ..] => (Some(true), remainder),
        [0xFE, 0xFF, remainder @ ..] => (Some(false), remainder),
        [0xEF, 0xBB, 0xBF, remainder @ ..] => (None, remainder),
        [b'<', 0, b'?', 0, ..] => (Some(true), xml),
        [0, b'<', 0, b'?', ..] => (Some(false), xml),
        _ => (None, xml),
    };
    let Some(little_endian) = encoding else {
        return Ok(Cow::Borrowed(xml));
    };
    let (chunks, remainder) = xml.as_chunks::<2>();
    if !remainder.is_empty() {
        return Err(AppError::InvalidInput(
            "DOCX document XML has an incomplete UTF-16 code unit".into(),
        ));
    }

    let code_units = chunks
        .iter()
        .map(|bytes| {
            if little_endian {
                u16::from_le_bytes(*bytes)
            } else {
                u16::from_be_bytes(*bytes)
            }
        })
        .collect::<Vec<_>>();
    let text = String::from_utf16(&code_units)
        .map_err(|_| AppError::InvalidInput("DOCX document XML is not valid UTF-16".into()))?;
    Ok(Cow::Owned(text.into_bytes()))
}

pub fn bounded_docx_xml(reader: &mut impl Read, declared_size: u64) -> Result<Vec<u8>, AppError> {
    if declared_size > DOCX_XML_MAX_BYTES as u64 {
        return Err(docx_xml_limit_error());
    }

    let mut xml = Vec::new();
    xml.try_reserve_exact(declared_size as usize).map_err(|e| {
        AppError::Internal(format!("DOCX extraction could not allocate buffer: {e}"))
    })?;
    if !read_bounded_bytes(reader, &mut xml, DOCX_XML_MAX_BYTES, "DOCX extraction")? {
        return Err(docx_xml_limit_error());
    }

    Ok(xml)
}

pub fn docx_xml_limit_error() -> AppError {
    AppError::InvalidInput(format!(
        "DOCX document XML exceeds the {DOCX_XML_MAX_BYTES}-byte extraction limit"
    ))
}
