use super::{
    ExtractedPages,
    util::{EXTRACTED_TEXT_MAX_BYTES, pages_from_page_texts},
};
use domain::AppError;
use std::path::Path;

pub const PDF_MAX_SOURCE_BYTES: u64 = 64 * 1024 * 1024;
pub const PDF_MAX_PAGES: usize = 10_000;
pub const PDF_MAX_PAGE_TEXT_BYTES: usize = 4 * 1024 * 1024;
pub const PDF_MAX_RETAINED_TEXT_BYTES: usize = EXTRACTED_TEXT_MAX_BYTES;

pub struct BoundedPdfText {
    text: String,
    max_bytes: usize,
    pub limit_reached: bool,
}

impl BoundedPdfText {
    pub fn new(max_bytes: usize) -> Self {
        Self {
            text: String::with_capacity(max_bytes),
            max_bytes,
            limit_reached: false,
        }
    }

    pub fn into_text(self) -> String {
        self.text
    }
}

impl std::io::Write for BoundedPdfText {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let Some(total_bytes) = self.text.len().checked_add(bytes.len()) else {
            self.limit_reached = true;
            return Err(std::io::Error::other("PDF extracted text limit exceeded"));
        };
        if total_bytes > self.max_bytes {
            self.limit_reached = true;
            return Err(std::io::Error::other("PDF extracted text limit exceeded"));
        }

        let text = std::str::from_utf8(bytes).map_err(|_| {
            std::io::Error::new(std::io::ErrorKind::InvalidData, "invalid PDF text")
        })?;
        self.text.push_str(text);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

pub fn pdf_text_limit_error() -> AppError {
    AppError::InvalidInput(format!(
        "PDF extracted text exceeds the {PDF_MAX_PAGE_TEXT_BYTES}-byte per-page or {PDF_MAX_RETAINED_TEXT_BYTES}-byte total extraction limit"
    ))
}

pub fn ensure_pdf_source_within_limit(source_size: u64) -> Result<(), AppError> {
    if source_size > PDF_MAX_SOURCE_BYTES {
        return Err(AppError::InvalidInput(format!(
            "PDF source exceeds the {PDF_MAX_SOURCE_BYTES}-byte extraction limit"
        )));
    }
    Ok(())
}

pub fn ensure_pdf_page_count_within_limit(page_count: usize) -> Result<(), AppError> {
    if page_count > PDF_MAX_PAGES {
        return Err(AppError::InvalidInput(format!(
            "PDF has more than the {PDF_MAX_PAGES}-page extraction limit"
        )));
    }
    Ok(())
}

pub fn pdf_page_text_limit(retained_text_bytes: usize) -> Result<usize, AppError> {
    PDF_MAX_RETAINED_TEXT_BYTES
        .checked_sub(retained_text_bytes)
        .map(|remaining| remaining.min(PDF_MAX_PAGE_TEXT_BYTES))
        .filter(|limit| *limit > 0)
        .ok_or_else(pdf_text_limit_error)
}

pub fn extract_pdf_pages(
    pdf_path: &Path,
    on_page_extracted: impl Fn(usize, usize),
) -> Result<ExtractedPages, AppError> {
    let source_size = std::fs::metadata(pdf_path)
        .map_err(|e| AppError::Io(format!("PDF extraction failed: {e}")))?
        .len();
    ensure_pdf_source_within_limit(source_size)?;

    let mut document = pdf_extract::Document::load(pdf_path)
        .map_err(|e| AppError::Io(format!("PDF page enumeration failed: {e}")))?;
    if document.is_encrypted() {
        document
            .decrypt("")
            .map_err(|e| AppError::Io(format!("PDF extraction failed: {e}")))?;
    }

    let total_pages = document.get_pages().len();
    ensure_pdf_page_count_within_limit(total_pages)?;
    let mut page_texts = Vec::with_capacity(total_pages);
    let mut retained_text_bytes = 0;
    for page_index in 1..=total_pages {
        let page_number = u32::try_from(page_index)
            .map_err(|_| AppError::Io("PDF has more pages than this extractor supports".into()))?;
        let mut page_text = BoundedPdfText::new(pdf_page_text_limit(retained_text_bytes)?);
        let extraction = {
            let mut output =
                pdf_extract::PlainTextOutput::new(&mut page_text as &mut dyn std::io::Write);
            pdf_extract::output_doc_page(&document, &mut output, page_number)
        };
        if page_text.limit_reached {
            return Err(pdf_text_limit_error());
        }
        if extraction.is_err() {
            break;
        }

        let text = page_text.into_text();
        retained_text_bytes += text.len();
        on_page_extracted(page_index, total_pages);
        page_texts.push(text);
    }

    Ok(ExtractedPages {
        pages: pages_from_page_texts(page_texts, total_pages, "PDF")?,
        total_pages,
    })
}
