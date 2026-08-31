use domain::AppError;
use std::path::Path;

pub fn extract_pages(pdf_path: &Path) -> Result<Vec<(i32, String)>, AppError> {
    let page_count = pdf_extract::Document::load(pdf_path)
        .map_err(|e| AppError::Io(format!("PDF page enumeration failed: {e}")))?
        .get_pages()
        .len();
    let pages = pdf_extract::extract_text_by_pages(pdf_path)
        .map_err(|e| AppError::Io(format!("PDF extraction failed: {e}")))?;

    pages_from_page_texts(pages, page_count)
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
