use domain::AppError;
use std::path::Path;

pub fn extract_pages(pdf_path: &Path) -> Result<Vec<(i32, String)>, AppError> {
    let full_text = pdf_extract::extract_text(pdf_path)
        .map_err(|e| AppError::Io(format!("PDF extraction failed: {e}")))?;

    pages_from_text(&full_text)
}

fn pages_from_text(full_text: &str) -> Result<Vec<(i32, String)>, AppError> {
    let pages = full_text
        .split('\x0C')
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
    use super::pages_from_text;

    #[test]
    fn preserves_physical_page_numbers_when_pages_are_blank() {
        let pages = pages_from_text("first\x0C\x0Cthird").expect("text should produce pages");

        assert_eq!(pages, vec![(1, "first".into()), (3, "third".into())]);
    }
}
