use domain::AppError;
use std::{fs::File, io::Read, path::Path};

pub const TEXT_PREVIEW_MAX_BYTES: usize = 1024 * 1024;
/// Indexed text is capped well below the 50 MiB upload envelope. This matches
/// the existing retained-PDF-text limit and keeps extraction, chunking, and
/// embedding memory proportional to a single document.
pub use domain::EXTRACTED_TEXT_MAX_BYTES;

pub fn extracted_text_limit_error() -> AppError {
    AppError::invalid_input(
        "Extract document text",
        format!(
            "Extracted document text exceeds the {} MiB extraction limit",
            EXTRACTED_TEXT_MAX_BYTES / (1024 * 1024)
        ),
        "Upload a smaller document or split it into sections under 16 MiB.",
    )
}

pub fn append_extracted_text(text: &mut String, suffix: &str) -> Result<(), AppError> {
    let Some(total_bytes) = text.len().checked_add(suffix.len()) else {
        return Err(extracted_text_limit_error());
    };
    if total_bytes > EXTRACTED_TEXT_MAX_BYTES {
        return Err(extracted_text_limit_error());
    }
    text.push_str(suffix);
    Ok(())
}

pub fn ensure_extracted_pages_within_limit(pages: &[(i32, String)]) -> Result<(), AppError> {
    let total_bytes = pages.iter().try_fold(0usize, |total, (_, text)| {
        total
            .checked_add(text.len())
            .ok_or_else(extracted_text_limit_error)
    })?;
    if total_bytes > EXTRACTED_TEXT_MAX_BYTES {
        return Err(extracted_text_limit_error());
    }
    Ok(())
}

pub fn read_bounded_bytes(
    reader: &mut impl Read,
    bytes: &mut Vec<u8>,
    max_bytes: usize,
    operation: &str,
) -> Result<bool, AppError> {
    let mut buffer = [0u8; 8 * 1024];
    loop {
        let read = reader
            .read(&mut buffer)
            .map_err(|e| {
                AppError::storage(
                    "Read document stream",
                    format!("{operation} failed: {e}"),
                    "Verify file readability and storage integrity.",
                )
            })?;
        if read == 0 {
            return Ok(true);
        }
        let Some(total_bytes) = bytes.len().checked_add(read) else {
            return Ok(false);
        };
        if total_bytes > max_bytes {
            return Ok(false);
        }
        bytes.try_reserve_exact(read).map_err(|e| {
            AppError::internal(
                "Allocate buffer",
                format!("{operation} could not allocate buffer: {e}"),
                "Ensure sufficient available system memory.",
            )
        })?;
        bytes.extend_from_slice(&buffer[..read]);
    }
}

pub fn utf8_preview(source_path: &Path, operation: &str) -> Result<(String, bool), AppError> {
    let mut file = File::open(source_path).map_err(|e| {
        AppError::storage(
            "Open preview file",
            format!("{operation} could not open source: {e}"),
            "Verify file permissions and accessibility.",
        )
    })?;
    let (bytes, truncated) = preview_bytes(&mut file, operation)?;
    let content = String::from_utf8(bytes).map_err(|_| {
        AppError::invalid_input(
            "Decode preview text",
            format!("{operation} source is not valid UTF-8"),
            "Ensure the source file uses valid UTF-8 encoding.",
        )
    })?;
    let content = content.strip_prefix('\u{feff}').unwrap_or(&content).to_string();
    Ok((content, truncated))
}

pub fn preview_raw_bytes(
    reader: &mut impl Read,
    max_bytes: usize,
    operation: &str,
) -> Result<(Vec<u8>, bool), AppError> {
    let mut bytes = Vec::with_capacity(max_bytes + 1);
    reader
        .take((max_bytes + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|e| {
            AppError::storage(
                "Read preview stream",
                format!("{operation} could not read source: {e}"),
                "Verify file readability and storage integrity.",
            )
        })?;

    let truncated = bytes.len() > max_bytes;
    if truncated {
        bytes.truncate(max_bytes);
    }

    Ok((bytes, truncated))
}

pub fn preview_bytes(reader: &mut impl Read, operation: &str) -> Result<(Vec<u8>, bool), AppError> {
    let (mut bytes, truncated) = preview_raw_bytes(reader, TEXT_PREVIEW_MAX_BYTES, operation)?;
    if truncated {
        while let Err(error) = std::str::from_utf8(&bytes) {
            if error.error_len().is_some() {
                return Err(AppError::invalid_input(
                    "Decode preview text",
                    format!("{operation} source is not valid UTF-8"),
                    "Ensure the source file uses valid UTF-8 encoding.",
                ));
            }
            bytes.truncate(error.valid_up_to());
        }
    }

    Ok((bytes, truncated))
}

pub fn pages_from_page_texts(
    page_texts: Vec<String>,
    expected_page_count: usize,
    format: &str,
) -> Result<Vec<(i32, String)>, AppError> {
    if page_texts.len() != expected_page_count {
        return Err(AppError::invalid_input(
            "Extract document pages",
            format!(
                "{format} extraction stopped after {} of {expected_page_count} pages",
                page_texts.len()
            ),
            "Ensure the document is complete and not corrupted.",
        ));
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
        return Err(AppError::invalid_input(
            "Extract document pages",
            format!("No text extracted from {format}"),
            "Ensure the document contains extractable text, not only scanned images.",
        ));
    }

    Ok(pages)
}

pub fn nonpaginated_page(text: String, format: &str) -> Result<String, AppError> {
    let text = text.trim();
    if text.is_empty() {
        return Err(AppError::invalid_input(
            "Extract document text",
            format!("No text extracted from {format}"),
            "Ensure the document contains extractable text, not only scanned images.",
        ));
    }
    Ok(text.to_string())
}
