pub mod docx;
pub mod markdown;
pub mod pdf;
pub mod text;
pub mod util;

#[cfg(test)]
mod tests;

pub use util::{EXTRACTED_TEXT_MAX_BYTES, TEXT_PREVIEW_MAX_BYTES};

use domain::{AppError, DocumentPreview};
use std::path::Path;

pub struct ExtractedPages {
    pub pages: Vec<(i32, String)>,
    pub total_pages: usize,
}

pub fn extract_pages(
    source_path: &Path,
    file_type: &str,
    on_page_extracted: impl Fn(usize, usize),
) -> Result<ExtractedPages, AppError> {
    let extracted = match file_type {
        "pdf" => pdf::extract_pdf_pages(source_path, on_page_extracted),
        "docx" => docx::extract_docx_pages(source_path, on_page_extracted),
        "text" | "txt" | "md" | "markdown" => text::extract_text_pages(source_path, on_page_extracted),
        _ => Err(AppError::invalid_input(
            "Extract document pages",
            format!("Unsupported document file type '{file_type}'"),
            "Provide a supported document format (PDF, DOCX, TXT, or MD).",
        )),
    }?;
    util::ensure_extracted_pages_within_limit(&extracted.pages)?;
    Ok(extracted)
}

pub fn document_preview(
    source_path: &Path,
    file_type: &str,
    filename: &str,
) -> Result<DocumentPreview, AppError> {
    match file_type {
        "text" | "txt" | "md" | "markdown" => {
            let (content, truncated) = util::utf8_preview(source_path, "Text preview")?;
            if markdown::is_markdown_filename(filename) || file_type == "md" || file_type == "markdown" {
                Ok(DocumentPreview::Markdown {
                    blocks: markdown::markdown_preview_blocks(&content),
                    truncated,
                })
            } else {
                Ok(DocumentPreview::PlainText { content, truncated })
            }
        }
        "docx" => docx::docx_preview(source_path),
        _ => Err(AppError::invalid_input(
            "Generate document preview",
            "A preview is unavailable for this document type",
            "Previews are supported for PDF, DOCX, TXT, and Markdown documents.",
        )),
    }
}
