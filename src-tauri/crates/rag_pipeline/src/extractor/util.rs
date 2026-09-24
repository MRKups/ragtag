use domain::AppError;
use std::{fs::File, io::Read, path::Path};

pub const TEXT_PREVIEW_MAX_BYTES: usize = 1024 * 1024;
/// Indexed text is capped well below the 50 MiB upload envelope. This matches
/// the existing retained-PDF-text limit and keeps extraction, chunking, and
/// embedding memory proportional to a single document.
pub const EXTRACTED_TEXT_MAX_BYTES: usize = 16 * 1024 * 1024;

pub fn extracted_text_limit_error() -> AppError {
    AppError::InvalidInput(format!(
        "Extracted document text exceeds the {EXTRACTED_TEXT_MAX_BYTES}-byte extraction limit"
    ))
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
            .map_err(|e| AppError::Io(format!("{operation} failed: {e}")))?;
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
            AppError::Internal(format!("{operation} could not allocate buffer: {e}"))
        })?;
        bytes.extend_from_slice(&buffer[..read]);
    }
}

pub fn utf8_preview(source_path: &Path, operation: &str) -> Result<(String, bool), AppError> {
    let mut file = File::open(source_path)
        .map_err(|e| AppError::Io(format!("{operation} could not open source: {e}")))?;
    let (bytes, truncated) = preview_bytes(&mut file, operation)?;
    let content = String::from_utf8(bytes)
        .map_err(|_| AppError::InvalidInput(format!("{operation} source is not valid UTF-8")))?;
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
        .map_err(|e| AppError::Io(format!("{operation} could not read source: {e}")))?;

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
                return Err(AppError::InvalidInput(format!(
                    "{operation} source is not valid UTF-8"
                )));
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

pub fn nonpaginated_page(text: String, format: &str) -> Result<String, AppError> {
    let text = text.trim();
    if text.is_empty() {
        return Err(AppError::Io(format!("No text extracted from {format}")));
    }
    Ok(text.to_string())
}
