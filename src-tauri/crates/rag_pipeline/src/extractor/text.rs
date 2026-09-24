use super::{
    ExtractedPages,
    util::{EXTRACTED_TEXT_MAX_BYTES, extracted_text_limit_error, nonpaginated_page, read_bounded_bytes},
};
use domain::AppError;
use std::{fs::File, io::Read, path::Path};

pub fn extract_text_pages(
    source_path: &Path,
    on_page_extracted: impl Fn(usize, usize),
) -> Result<ExtractedPages, AppError> {
    let text = bounded_utf8_text(source_path)?;
    let page = nonpaginated_page(text, "text document")?;
    on_page_extracted(1, 1);
    Ok(ExtractedPages {
        pages: vec![(1, page)],
        total_pages: 1,
    })
}

fn bounded_utf8_text(source_path: &Path) -> Result<String, AppError> {
    let mut file = File::open(source_path)
        .map_err(|e| AppError::Io(format!("Text extraction failed: {e}")))?;
    let source_size = file
        .metadata()
        .map_err(|e| AppError::Io(format!("Text extraction failed: {e}")))?
        .len();
    if source_size > EXTRACTED_TEXT_MAX_BYTES as u64 {
        return Err(extracted_text_limit_error());
    }

    let capacity = usize::try_from(source_size)
        .unwrap_or(EXTRACTED_TEXT_MAX_BYTES)
        .min(EXTRACTED_TEXT_MAX_BYTES);
    bounded_utf8_text_reader(&mut file, capacity)
}

pub fn bounded_utf8_text_reader(reader: &mut impl Read, capacity: usize) -> Result<String, AppError> {
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(capacity.min(EXTRACTED_TEXT_MAX_BYTES))
        .map_err(|e| {
            AppError::Internal(format!("Text extraction could not allocate buffer: {e}"))
        })?;
    if !read_bounded_bytes(
        reader,
        &mut bytes,
        EXTRACTED_TEXT_MAX_BYTES,
        "Text extraction",
    )? {
        return Err(extracted_text_limit_error());
    }

    let text = String::from_utf8(bytes)
        .map_err(|_| AppError::InvalidInput("Text extraction source is not valid UTF-8".into()))?;
    Ok(text.strip_prefix('\u{feff}').unwrap_or(&text).to_string())
}
