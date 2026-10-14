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
    raw: Vec<u8>,
    max_bytes: usize,
    pub limit_reached: bool,
}

impl BoundedPdfText {
    pub fn new(max_bytes: usize) -> Self {
        Self {
            raw: Vec::new(),
            max_bytes,
            limit_reached: false,
        }
    }

    pub fn into_text(self) -> String {
        match String::from_utf8(self.raw) {
            Ok(s) => s,
            Err(e) => String::from_utf8_lossy(&e.into_bytes()).into_owned(),
        }
    }
}

impl std::io::Write for BoundedPdfText {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let Some(total_bytes) = self.raw.len().checked_add(bytes.len()) else {
            self.limit_reached = true;
            return Err(std::io::Error::other("PDF extracted text limit exceeded"));
        };
        if total_bytes > self.max_bytes {
            self.limit_reached = true;
            return Err(std::io::Error::other("PDF extracted text limit exceeded"));
        }

        self.raw.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

pub fn pdf_text_limit_error() -> AppError {
    AppError::invalid_input(
        "Extract PDF text",
        format!(
            "PDF extracted text exceeds the {} MiB per-page or {} MiB total extraction limit",
            PDF_MAX_PAGE_TEXT_BYTES / (1024 * 1024),
            PDF_MAX_RETAINED_TEXT_BYTES / (1024 * 1024)
        ),
        "Upload a smaller PDF or split the document into smaller chapters.",
    )
}

pub fn ensure_pdf_source_within_limit(source_size: u64) -> Result<(), AppError> {
    if source_size > PDF_MAX_SOURCE_BYTES {
        return Err(AppError::invalid_input(
            "Extract PDF document",
            format!(
                "PDF source exceeds the {} MiB extraction limit",
                PDF_MAX_SOURCE_BYTES / (1024 * 1024)
            ),
            "Upload a PDF file smaller than 50 MiB.",
        ));
    }
    Ok(())
}

pub fn ensure_pdf_page_count_within_limit(page_count: usize) -> Result<(), AppError> {
    if page_count > PDF_MAX_PAGES {
        return Err(AppError::invalid_input(
            "Extract PDF document",
            format!(
                "PDF has {page_count} pages, exceeding the {PDF_MAX_PAGES}-page extraction limit"
            ),
            "Split the document into sections containing at most 10,000 pages.",
        ));
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
        .map_err(|e| {
            AppError::storage(
                "Inspect PDF metadata",
                format!("PDF extraction failed: {e}"),
                "Check file permissions and accessibility.",
            )
        })?
        .len();
    ensure_pdf_source_within_limit(source_size)?;

    let mut document = pdf_extract::Document::load(pdf_path).map_err(|e| {
        AppError::invalid_input(
            "Load PDF document",
            format!("PDF page enumeration failed: {e}"),
            "Verify the file is a valid, uncorrupted PDF document.",
        )
    })?;
    if document.is_encrypted() {
        document.decrypt("").map_err(|e| {
            AppError::invalid_input(
                "Decrypt PDF document",
                format!("PDF decryption failed: {e}"),
                "Remove the password protection from the PDF before uploading.",
            )
        })?;
    }

    let total_pages = document.get_pages().len();
    ensure_pdf_page_count_within_limit(total_pages)?;
    let mut page_texts = Vec::with_capacity(total_pages);
    let mut retained_text_bytes = 0;
    for page_index in 1..=total_pages {
        let page_number = u32::try_from(page_index).map_err(|_| {
            AppError::invalid_input(
                "Extract PDF page",
                "PDF has more pages than this extractor supports",
                "Split the PDF into smaller sections.",
            )
        })?;
        let mut page_text = BoundedPdfText::new(pdf_page_text_limit(retained_text_bytes)?);
        let extraction = {
            let mut output =
                pdf_extract::PlainTextOutput::new(&mut page_text as &mut dyn std::io::Write);
            pdf_extract::output_doc_page(&document, &mut output, page_number)
        };
        if page_text.limit_reached {
            return Err(pdf_text_limit_error());
        }
        if let Err(err) = extraction {
            return Err(AppError::invalid_input(
                "Extract PDF page",
                format!("Failed to extract PDF page {page_number} of {total_pages}: {err}"),
                "Verify that the PDF is valid, unencrypted, and uncorrupted.",
            ));
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
