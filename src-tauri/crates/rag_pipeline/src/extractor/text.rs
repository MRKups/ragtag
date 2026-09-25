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
    let mut file = File::open(source_path).map_err(|e| {
        AppError::storage(
            "Open text file",
            format!("Text extraction failed: {e}"),
            "Verify file permissions and accessibility.",
        )
    })?;
    let source_size = file
        .metadata()
        .map_err(|e| {
            AppError::storage(
                "Inspect text metadata",
                format!("Text extraction failed: {e}"),
                "Verify file permissions and accessibility.",
            )
        })?
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
            AppError::internal(
                "Allocate text buffer",
                format!("Text extraction could not allocate buffer: {e}"),
                "Ensure sufficient available system memory.",
            )
        })?;
    if !read_bounded_bytes(
        reader,
        &mut bytes,
        EXTRACTED_TEXT_MAX_BYTES,
        "Text extraction",
    )? {
        return Err(extracted_text_limit_error());
    }

    let text = String::from_utf8(bytes).map_err(|_| {
        AppError::invalid_input(
            "Decode text file",
            "Text extraction source is not valid UTF-8",
            "Save the file with UTF-8 encoding before uploading.",
        )
    })?;
    Ok(text.strip_prefix('\u{feff}').unwrap_or(&text).to_string())
}
