use domain::AppError;
use std::path::Path;

pub struct ExtractedPages {
    pub pages: Vec<(i32, String)>,
    pub total_pages: usize,
}

pub fn extract_pages(
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
        pages: pages_from_page_texts(page_texts, total_pages)?,
        total_pages,
    })
}

fn pages_from_page_texts(
    page_texts: Vec<String>,
    expected_page_count: usize,
) -> Result<Vec<(i32, String)>, AppError> {
    if page_texts.len() != expected_page_count {
        return Err(AppError::Io(format!(
            "PDF extraction stopped after {} of {expected_page_count} pages",
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
        return Err(AppError::Io("No text extracted from PDF".into()));
    }

    Ok(pages)
}

#[cfg(test)]
mod tests {
    use super::pages_from_page_texts;

    #[test]
    fn preserves_physical_page_numbers_when_pages_are_blank() {
        let pages = pages_from_page_texts(vec!["first".into(), String::new(), "third".into()], 3)
            .expect("text should produce pages");

        assert_eq!(pages, vec![(1, "first".into()), (3, "third".into())]);
    }

    #[test]
    fn rejects_documents_with_no_extractable_page_text() {
        assert!(pages_from_page_texts(vec![String::new(), "  ".into()], 2).is_err());
    }

    #[test]
    fn rejects_partial_page_extraction() {
        let error = pages_from_page_texts(vec!["first".into()], 2)
            .expect_err("a missing physical page must fail extraction");

        assert!(matches!(error, domain::AppError::Io(_)));
    }
}
