use domain::AppError;
use std::path::Path;

pub fn extract_pages(pdf_path: &Path) -> Result<Vec<(i32, String)>, AppError> {
    let full_text = pdf_extract::extract_text(pdf_path)
        .map_err(|e| AppError::Io(format!("PDF extraction failed: {e}")))?;

    let pages: Vec<String> = full_text
        .split('\x0C')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    if pages.is_empty() {
        return Err(AppError::Io("No text extracted from PDF".into()));
    }

    Ok(pages
        .into_iter()
        .enumerate()
        .map(|(i, t)| (i as i32 + 1, t))
        .collect())
}
