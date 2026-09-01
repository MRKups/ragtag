use domain::AppError;
use quick_xml::{Reader, escape::unescape, events::Event};
use std::{fs::File, io::Read, path::Path};

pub struct ExtractedPages {
    pub pages: Vec<(i32, String)>,
    pub total_pages: usize,
}

pub fn extract_pages(
    source_path: &Path,
    file_type: &str,
    on_page_extracted: impl Fn(usize, usize),
) -> Result<ExtractedPages, AppError> {
    match file_type {
        "pdf" => extract_pdf_pages(source_path, on_page_extracted),
        "docx" => extract_docx_pages(source_path, on_page_extracted),
        "text" => extract_text_pages(source_path, on_page_extracted),
        _ => Err(AppError::InvalidInput(format!(
            "Unsupported document file type '{file_type}'"
        ))),
    }
}

fn extract_pdf_pages(
    pdf_path: &Path,
    on_page_extracted: impl Fn(usize, usize),
) -> Result<ExtractedPages, AppError> {
    let mut document = pdf_extract::Document::load(pdf_path)
        .map_err(|e| AppError::Io(format!("PDF page enumeration failed: {e}")))?;
    if document.is_encrypted() {
        document
            .decrypt("")
            .map_err(|e| AppError::Io(format!("PDF extraction failed: {e}")))?;
    }

    let total_pages = document.get_pages().len();
    let mut page_texts = Vec::with_capacity(total_pages);
    for page_index in 1..=total_pages {
        let page_number = u32::try_from(page_index)
            .map_err(|_| AppError::Io("PDF has more pages than this extractor supports".into()))?;
        let mut text = String::new();
        let extraction = {
            let mut output = pdf_extract::PlainTextOutput::new(&mut text);
            pdf_extract::output_doc_page(&document, &mut output, page_number)
        };
        if extraction.is_err() {
            break;
        }
        on_page_extracted(page_index, total_pages);
        page_texts.push(text);
    }

    Ok(ExtractedPages {
        pages: pages_from_page_texts(page_texts, total_pages, "PDF")?,
        total_pages,
    })
}

fn extract_text_pages(
    source_path: &Path,
    on_page_extracted: impl Fn(usize, usize),
) -> Result<ExtractedPages, AppError> {
    let text = std::fs::read_to_string(source_path)
        .map_err(|e| AppError::Io(format!("Text extraction failed: {e}")))?;
    let page = nonpaginated_page(text, "text document")?;
    on_page_extracted(1, 1);
    Ok(ExtractedPages {
        pages: vec![(1, page)],
        total_pages: 1,
    })
}

fn extract_docx_pages(
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
    let mut xml = Vec::new();
    document_xml
        .read_to_end(&mut xml)
        .map_err(|e| AppError::Io(format!("DOCX extraction failed: {e}")))?;

    let page = nonpaginated_page(docx_body_text(&xml)?, "DOCX document")?;
    on_page_extracted(1, 1);
    Ok(ExtractedPages {
        pages: vec![(1, page)],
        total_pages: 1,
    })
}

fn docx_body_text(xml: &[u8]) -> Result<String, AppError> {
    let mut reader = Reader::from_reader(xml);
    reader.config_mut().trim_text(false);
    let mut buffer = Vec::new();
    let mut in_text = false;
    let mut text = String::new();

    loop {
        match reader.read_event_into(&mut buffer) {
            Ok(Event::Start(event)) => match event.name().as_ref() {
                b"w:t" | b"t" => in_text = true,
                b"w:tab" | b"tab" => text.push('\t'),
                b"w:br" | b"br" | b"w:cr" | b"cr" => text.push('\n'),
                _ => {}
            },
            Ok(Event::Empty(event)) => match event.name().as_ref() {
                b"w:tab" | b"tab" => text.push('\t'),
                b"w:br" | b"br" | b"w:cr" | b"cr" => text.push('\n'),
                _ => {}
            },
            Ok(Event::End(event)) => match event.name().as_ref() {
                b"w:t" | b"t" => in_text = false,
                b"w:p" | b"p" => text.push('\n'),
                _ => {}
            },
            Ok(Event::Text(event)) if in_text => {
                let decoded = event
                    .decode()
                    .map_err(|e| AppError::Io(format!("DOCX text decoding failed: {e}")))?;
                text.push_str(&decoded);
            }
            Ok(Event::GeneralRef(event)) if in_text => {
                let reference = event
                    .decode()
                    .map_err(|e| AppError::Io(format!("DOCX text decoding failed: {e}")))?;
                let encoded = format!("&{reference};");
                let resolved = unescape(&encoded)
                    .map_err(|e| AppError::Io(format!("DOCX text decoding failed: {e}")))?;
                text.push_str(&resolved);
            }
            Ok(Event::Eof) => break,
            Ok(_) => {}
            Err(error) => return Err(AppError::Io(format!("DOCX XML parsing failed: {error}"))),
        }
        buffer.clear();
    }

    Ok(text)
}

fn pages_from_page_texts(
    page_texts: Vec<String>,
    expected_page_count: usize,
    format: &str,
) -> Result<Vec<(i32, String)>, AppError> {
    if page_texts.len() != expected_page_count {
        return Err(AppError::Io(format!(
            "{format} extraction stopped after {} of {expected_page_count} pages",
            page_texts.len()
        )));
    }

    let pages = page_texts
        .into_iter()
        .enumerate()
        .filter_map(|(index, text)| {
            let text = text.trim();
            (!text.is_empty()).then(|| (index as i32 + 1, text.to_string()))
        })
        .collect::<Vec<_>>();

    if pages.is_empty() {
        return Err(AppError::Io(format!("No text extracted from {format}")));
    }

    Ok(pages)
}

fn nonpaginated_page(text: String, format: &str) -> Result<String, AppError> {
    let text = text.trim();
    if text.is_empty() {
        return Err(AppError::Io(format!("No text extracted from {format}")));
    }
    Ok(text.to_string())
}

#[cfg(test)]
mod tests {
    use super::{docx_body_text, extract_pages, nonpaginated_page, pages_from_page_texts};
    use std::{fs::File, io::Write};
    use zip::{ZipWriter, write::SimpleFileOptions};

    #[test]
    fn preserves_physical_page_numbers_when_pdf_pages_are_blank() {
        let pages = pages_from_page_texts(
            vec!["first".into(), String::new(), "third".into()],
            3,
            "PDF",
        )
        .expect("text should produce pages");

        assert_eq!(pages, vec![(1, "first".into()), (3, "third".into())]);
    }

    #[test]
    fn rejects_documents_with_no_extractable_text() {
        assert!(pages_from_page_texts(vec![String::new(), "  ".into()], 2, "PDF").is_err());
        assert!(nonpaginated_page("  ".into(), "text document").is_err());
    }

    #[test]
    fn rejects_partial_pdf_extraction() {
        let error = pages_from_page_texts(vec!["first".into()], 2, "PDF")
            .expect_err("a missing physical page must fail extraction");

        assert!(matches!(error, domain::AppError::Io(_)));
    }

    #[test]
    fn extracts_docx_paragraph_text() {
        let xml = br#"<w:document xmlns:w="word"><w:body><w:p><w:r><w:t>First &amp; second</w:t></w:r></w:p><w:p><w:r><w:t>Third</w:t><w:tab/><w:t>column</w:t></w:r></w:p></w:body></w:document>"#;

        assert_eq!(
            docx_body_text(xml).unwrap(),
            "First & second\nThird\tcolumn\n"
        );
    }

    #[test]
    fn extracts_text_from_a_docx_archive() {
        let unique = format!(
            "{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let path = std::env::temp_dir().join(format!("ragtag-docx-{unique}.docx"));
        {
            let file = File::create(&path).unwrap();
            let mut archive = ZipWriter::new(file);
            archive
                .start_file("word/document.xml", SimpleFileOptions::default())
                .unwrap();
            archive
                .write_all(
                    br#"<w:document xmlns:w="word"><w:body><w:p><w:r><w:t>DOCX text</w:t></w:r></w:p></w:body></w:document>"#,
                )
                .unwrap();
            archive.finish().unwrap();
        }

        let pages = extract_pages(&path, "docx", |_, _| {}).unwrap();
        std::fs::remove_file(path).unwrap();

        assert_eq!(pages.total_pages, 1);
        assert_eq!(pages.pages, vec![(1, "DOCX text".into())]);
    }
}
