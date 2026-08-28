use domain::AppError;
use std::path::Path;

pub fn extract_pages(pdf_path: &Path) -> Result<Vec<(i32, String)>, AppError> {
    let pages = pdf_extract::extract_text_by_pages(pdf_path)
        .map_err(|e| AppError::Io(format!("PDF extraction failed: {e}")))?;

    pages_from_page_texts(pages)
}

fn pages_from_page_texts(page_texts: Vec<String>) -> Result<Vec<(i32, String)>, AppError> {
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
        let pages = pages_from_page_texts(vec!["first".into(), String::new(), "third".into()])
            .expect("text should produce pages");

        assert_eq!(pages, vec![(1, "first".into()), (3, "third".into())]);
    }

    #[test]
    fn rejects_documents_with_no_extractable_page_text() {
        assert!(pages_from_page_texts(vec![String::new(), "  ".into()]).is_err());
    }
}
