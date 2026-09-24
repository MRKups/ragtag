use domain::{AppError, DocumentPreview, MarkdownBlock, MarkdownInline};
use pulldown_cmark::{
    CodeBlockKind, Event as MarkdownEvent, MetadataBlockKind, Options, Parser, Tag, TagEnd,
};
use quick_xml::{
    NsReader,
    escape::unescape,
    events::Event,
    name::{LocalName, ResolveResult},
};
use std::{borrow::Cow, fs::File, io::Read, path::Path};

pub struct ExtractedPages {
    pub pages: Vec<(i32, String)>,
    pub total_pages: usize,
}

pub const TEXT_PREVIEW_MAX_BYTES: usize = 1024 * 1024;
/// Indexed text is capped well below the 50 MiB upload envelope. This matches
/// the existing retained-PDF-text limit and keeps extraction, chunking, and
/// embedding memory proportional to a single document.
pub const EXTRACTED_TEXT_MAX_BYTES: usize = 16 * 1024 * 1024;
const DOCX_XML_MAX_BYTES: usize = 16 * 1024 * 1024;
const PDF_MAX_SOURCE_BYTES: u64 = 64 * 1024 * 1024;
const PDF_MAX_PAGES: usize = 10_000;
const PDF_MAX_PAGE_TEXT_BYTES: usize = 4 * 1024 * 1024;
const PDF_MAX_RETAINED_TEXT_BYTES: usize = EXTRACTED_TEXT_MAX_BYTES;
const MARKDOWN_MAX_NESTING: usize = 32;
const WORDPROCESSINGML_NAMESPACE: &str =
    "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
const STRICT_WORDPROCESSINGML_NAMESPACE: &str = "http://purl.oclc.org/ooxml/wordprocessingml/main";

pub fn extract_pages(
    source_path: &Path,
    file_type: &str,
    on_page_extracted: impl Fn(usize, usize),
) -> Result<ExtractedPages, AppError> {
    let extracted = match file_type {
        "pdf" => extract_pdf_pages(source_path, on_page_extracted),
        "docx" => extract_docx_pages(source_path, on_page_extracted),
        "text" => extract_text_pages(source_path, on_page_extracted),
        _ => Err(AppError::InvalidInput(format!(
            "Unsupported document file type '{file_type}'"
        ))),
    }?;
    ensure_extracted_pages_within_limit(&extracted.pages)?;
    Ok(extracted)
}

struct BoundedPdfText {
    text: String,
    max_bytes: usize,
    limit_reached: bool,
}

impl BoundedPdfText {
    fn new(max_bytes: usize) -> Self {
        Self {
            text: String::with_capacity(max_bytes),
            max_bytes,
            limit_reached: false,
        }
    }

    fn into_text(self) -> String {
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

fn pdf_text_limit_error() -> AppError {
    AppError::InvalidInput(format!(
        "PDF extracted text exceeds the {PDF_MAX_PAGE_TEXT_BYTES}-byte per-page or {PDF_MAX_RETAINED_TEXT_BYTES}-byte total extraction limit"
    ))
}

fn ensure_pdf_source_within_limit(source_size: u64) -> Result<(), AppError> {
    if source_size > PDF_MAX_SOURCE_BYTES {
        return Err(AppError::InvalidInput(format!(
            "PDF source exceeds the {PDF_MAX_SOURCE_BYTES}-byte extraction limit"
        )));
    }
    Ok(())
}

fn ensure_pdf_page_count_within_limit(page_count: usize) -> Result<(), AppError> {
    if page_count > PDF_MAX_PAGES {
        return Err(AppError::InvalidInput(format!(
            "PDF has more than the {PDF_MAX_PAGES}-page extraction limit"
        )));
    }
    Ok(())
}

fn pdf_page_text_limit(retained_text_bytes: usize) -> Result<usize, AppError> {
    PDF_MAX_RETAINED_TEXT_BYTES
        .checked_sub(retained_text_bytes)
        .map(|remaining| remaining.min(PDF_MAX_PAGE_TEXT_BYTES))
        .filter(|limit| *limit > 0)
        .ok_or_else(pdf_text_limit_error)
}
fn extract_pdf_pages(
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

fn extract_text_pages(
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

fn bounded_utf8_text_reader(reader: &mut impl Read, capacity: usize) -> Result<String, AppError> {
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

fn read_bounded_bytes(
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

pub fn document_preview(
    source_path: &Path,
    file_type: &str,
    filename: &str,
) -> Result<DocumentPreview, AppError> {
    match file_type {
        "text" => {
            let (content, truncated) = utf8_preview(source_path, "Text preview")?;
            if is_markdown_filename(filename) {
                Ok(DocumentPreview::Markdown {
                    blocks: markdown_preview_blocks(&content),
                    truncated,
                })
            } else {
                Ok(DocumentPreview::PlainText { content, truncated })
            }
        }
        "docx" => docx_preview(source_path),
        _ => Err(AppError::InvalidInput(
            "A preview is unavailable for this document type".into(),
        )),
    }
}

fn is_markdown_filename(filename: &str) -> bool {
    Path::new(filename)
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            extension.eq_ignore_ascii_case("md") || extension.eq_ignore_ascii_case("markdown")
        })
}

fn utf8_preview(source_path: &Path, operation: &str) -> Result<(String, bool), AppError> {
    let mut file = File::open(source_path)
        .map_err(|e| AppError::Io(format!("{operation} could not open source: {e}")))?;
    let (bytes, truncated) = preview_bytes(&mut file, operation)?;
    let content = String::from_utf8(bytes)
        .map_err(|_| AppError::InvalidInput(format!("{operation} source is not valid UTF-8")))?;
    let content = content.strip_prefix('\u{feff}').unwrap_or(&content).to_string();
    Ok((content, truncated))
}

fn preview_raw_bytes(
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

fn preview_bytes(reader: &mut impl Read, operation: &str) -> Result<(Vec<u8>, bool), AppError> {
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

fn extract_docx_pages(
    source_path: &Path,
    on_page_extracted: impl Fn(usize, usize),
) -> Result<ExtractedPages, AppError> {
    let file = File::open(source_path)
        .map_err(|e| AppError::Io(format!("DOCX extraction failed: {e}")))?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|e| AppError::Io(format!("DOCX archive is invalid: {e}")))?;
    let mut document_xml = archive
        .by_name("word/document.xml")
        .map_err(|_| AppError::Io("DOCX archive does not contain document text".into()))?;
    let declared_xml_size = document_xml.size();
    let xml = bounded_docx_xml(&mut document_xml, declared_xml_size)?;

    let page = nonpaginated_page(docx_body_text(&xml, false)?, "DOCX document")?;
    on_page_extracted(1, 1);
    Ok(ExtractedPages {
        pages: vec![(1, page)],
        total_pages: 1,
    })
}

fn docx_preview(source_path: &Path) -> Result<DocumentPreview, AppError> {
    let file = File::open(source_path)
        .map_err(|e| AppError::Io(format!("DOCX preview could not open source: {e}")))?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|e| AppError::Io(format!("DOCX preview archive is invalid: {e}")))?;
    let mut document_xml = archive
        .by_name("word/document.xml")
        .map_err(|_| AppError::Io("DOCX preview archive does not contain document text".into()))?;
    let (mut xml, xml_truncated) =
        preview_raw_bytes(&mut document_xml, TEXT_PREVIEW_MAX_BYTES, "DOCX preview")?;
    if xml_truncated
        && xml.len() % 2 != 0
        && (xml.starts_with(&[0xFF, 0xFE])
            || xml.starts_with(&[0xFE, 0xFF])
            || xml.starts_with(&[b'<', 0, b'?', 0])
            || xml.starts_with(&[0, b'<', 0, b'?']))
    {
        xml.pop();
    }
    let (content, text_truncated) = truncate_preview_text(docx_body_text(&xml, xml_truncated)?);

    Ok(DocumentPreview::PlainText {
        content,
        truncated: xml_truncated || text_truncated,
    })
}

fn truncate_preview_text(mut content: String) -> (String, bool) {
    if content.len() <= TEXT_PREVIEW_MAX_BYTES {
        return (content, false);
    }

    let mut boundary = TEXT_PREVIEW_MAX_BYTES;
    while !content.is_char_boundary(boundary) {
        boundary -= 1;
    }
    content.truncate(boundary);
    (content, true)
}

fn docx_body_text(xml: &[u8], allow_truncated_eof: bool) -> Result<String, AppError> {
    let xml = normalized_docx_xml(xml)?;
    let mut reader = NsReader::from_reader(xml.as_ref());
    reader.config_mut().trim_text(false);
    let mut buffer = Vec::new();
    let mut in_text = false;
    let mut text = String::new();

    loop {
        match reader.read_resolved_event_into(&mut buffer) {
            Ok((namespace, Event::Start(event))) => {
                if is_wordprocessing_element(&namespace, event.local_name(), "t") {
                    in_text = true;
                } else if is_wordprocessing_element(&namespace, event.local_name(), "tab") {
                    append_extracted_text(&mut text, "\t")?;
                } else if is_wordprocessing_element(&namespace, event.local_name(), "br")
                    || is_wordprocessing_element(&namespace, event.local_name(), "cr")
                {
                    append_extracted_text(&mut text, "\n")?;
                }
            }
            Ok((namespace, Event::Empty(event))) => {
                if is_wordprocessing_element(&namespace, event.local_name(), "tab") {
                    append_extracted_text(&mut text, "\t")?;
                } else if is_wordprocessing_element(&namespace, event.local_name(), "br")
                    || is_wordprocessing_element(&namespace, event.local_name(), "cr")
                    || is_wordprocessing_element(&namespace, event.local_name(), "p")
                {
                    append_extracted_text(&mut text, "\n")?;
                }
            }
            Ok((namespace, Event::End(event))) => {
                if is_wordprocessing_element(&namespace, event.local_name(), "t") {
                    in_text = false;
                } else if is_wordprocessing_element(&namespace, event.local_name(), "p") {
                    append_extracted_text(&mut text, "\n")?;
                }
            }
            Ok((_, Event::Text(event))) if in_text => {
                let decoded = event
                    .decode()
                    .map_err(|e| AppError::Io(format!("DOCX text decoding failed: {e}")))?;
                append_extracted_text(&mut text, &decoded)?;
            }
            Ok((_, Event::CData(event))) if in_text => {
                let decoded = event
                    .decode()
                    .map_err(|e| AppError::Io(format!("DOCX text decoding failed: {e}")))?;
                append_extracted_text(&mut text, &decoded)?;
            }
            Ok((_, Event::GeneralRef(event))) if in_text => {
                let reference = event
                    .decode()
                    .map_err(|e| AppError::Io(format!("DOCX text decoding failed: {e}")))?;
                let encoded = format!("&{reference};");
                let resolved = unescape(&encoded)
                    .map_err(|e| AppError::Io(format!("DOCX text decoding failed: {e}")))?;
                append_extracted_text(&mut text, &resolved)?;
            }
            Ok((_, Event::Eof)) => break,
            Ok(_) => {}
            Err(_) if allow_truncated_eof => break,
            Err(error) => return Err(AppError::Io(format!("DOCX XML parsing failed: {error}"))),
        }
        buffer.clear();
    }

    Ok(text)
}

fn is_wordprocessing_element(
    namespace: &ResolveResult<'_>,
    local_name: LocalName<'_>,
    expected_local_name: &str,
) -> bool {
    let ResolveResult::Bound(namespace) = namespace else {
        return false;
    };
    let namespace = std::str::from_utf8(namespace.as_ref()).ok();
    let local_name = std::str::from_utf8(local_name.as_ref()).ok();
    matches!(
        (namespace, local_name),
        (
            Some(WORDPROCESSINGML_NAMESPACE | STRICT_WORDPROCESSINGML_NAMESPACE),
            Some(local_name)
        ) if local_name == expected_local_name
    )
}

fn normalized_docx_xml(xml: &[u8]) -> Result<Cow<'_, [u8]>, AppError> {
    let (encoding, xml) = match xml {
        [0xFF, 0xFE, remainder @ ..] => (Some(true), remainder),
        [0xFE, 0xFF, remainder @ ..] => (Some(false), remainder),
        [0xEF, 0xBB, 0xBF, remainder @ ..] => (None, remainder),
        [b'<', 0, b'?', 0, ..] => (Some(true), xml),
        [0, b'<', 0, b'?', ..] => (Some(false), xml),
        _ => (None, xml),
    };
    let Some(little_endian) = encoding else {
        return Ok(Cow::Borrowed(xml));
    };
    let chunks = xml.chunks_exact(2);
    if !chunks.remainder().is_empty() {
        return Err(AppError::InvalidInput(
            "DOCX document XML has an incomplete UTF-16 code unit".into(),
        ));
    }

    let code_units = chunks
        .map(|bytes| {
            if little_endian {
                u16::from_le_bytes([bytes[0], bytes[1]])
            } else {
                u16::from_be_bytes([bytes[0], bytes[1]])
            }
        })
        .collect::<Vec<_>>();
    let text = String::from_utf16(&code_units)
        .map_err(|_| AppError::InvalidInput("DOCX document XML is not valid UTF-16".into()))?;
    Ok(Cow::Owned(text.into_bytes()))
}

type MarkdownEvents<'a> = std::iter::Peekable<std::vec::IntoIter<MarkdownEvent<'a>>>;

#[derive(Clone, Copy)]
enum BlockEnd {
    BlockQuote,
}

#[derive(Clone, Copy)]
enum InlineEnd {
    Paragraph,
    Heading,
    TableCell,
    Item,
    Emphasis,
    Strong,
    Strikethrough,
    Link,
    Image,
}

fn markdown_preview_blocks(markdown: &str) -> Vec<MarkdownBlock> {
    markdown_blocks(without_leading_yaml_front_matter(markdown))
}

fn without_leading_yaml_front_matter(markdown: &str) -> &str {
    let metadata_source = markdown.strip_prefix('\u{feff}').unwrap_or(markdown);
    let mut events = Parser::new_ext(metadata_source, Options::ENABLE_YAML_STYLE_METADATA_BLOCKS)
        .into_offset_iter();

    let Some((MarkdownEvent::Start(Tag::MetadataBlock(MetadataBlockKind::YamlStyle)), range)) =
        events.next()
    else {
        return metadata_source;
    };
    if range.start != 0 {
        return metadata_source;
    }

    let Some(end) = events.find_map(|(event, range)| {
        matches!(
            event,
            MarkdownEvent::End(TagEnd::MetadataBlock(MetadataBlockKind::YamlStyle))
        )
        .then_some(range.end)
    }) else {
        return metadata_source;
    };

    &metadata_source[end..]
}

fn markdown_blocks(markdown: &str) -> Vec<MarkdownBlock> {
    let options =
        Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TABLES | Options::ENABLE_TASKLISTS;
    let mut events = Parser::new_ext(markdown, options)
        .collect::<Vec<_>>()
        .into_iter()
        .peekable();
    parse_blocks(&mut events, None, 0)
}

fn parse_blocks(
    events: &mut MarkdownEvents<'_>,
    end: Option<BlockEnd>,
    nesting: usize,
) -> Vec<MarkdownBlock> {
    let mut blocks = Vec::new();

    while let Some(event) = events.next() {
        if let MarkdownEvent::End(tag_end) = &event {
            if matches_block_end(tag_end, end) {
                break;
            }
        }

        match event {
            MarkdownEvent::Start(Tag::Paragraph) => blocks.push(MarkdownBlock::Paragraph {
                content: parse_inlines(events, InlineEnd::Paragraph, 0),
            }),
            MarkdownEvent::Start(Tag::Heading { level, .. }) => {
                blocks.push(MarkdownBlock::Heading {
                    level: level as u8,
                    content: parse_inlines(events, InlineEnd::Heading, 0),
                });
            }
            MarkdownEvent::Start(Tag::BlockQuote(_)) => {
                if nesting < MARKDOWN_MAX_NESTING {
                    blocks.push(MarkdownBlock::BlockQuote {
                        blocks: parse_blocks(events, Some(BlockEnd::BlockQuote), nesting + 1),
                    });
                } else {
                    push_flattened_block(events, &mut blocks);
                }
            }
            MarkdownEvent::Start(Tag::List(start)) => {
                if nesting < MARKDOWN_MAX_NESTING {
                    blocks.push(parse_list(events, start, nesting + 1));
                } else {
                    push_flattened_block(events, &mut blocks);
                }
            }
            MarkdownEvent::Start(Tag::CodeBlock(kind)) => {
                blocks.push(parse_code_block(events, kind));
            }
            MarkdownEvent::Start(Tag::Table(_)) => blocks.push(parse_table(events)),
            MarkdownEvent::Start(Tag::HtmlBlock) => skip_until(events, TagEnd::HtmlBlock),
            MarkdownEvent::Rule => blocks.push(MarkdownBlock::ThematicBreak),
            _ => {}
        }
    }

    blocks
}

fn matches_block_end(tag_end: &TagEnd, expected: Option<BlockEnd>) -> bool {
    matches!(
        (tag_end, expected),
        (TagEnd::BlockQuote(_), Some(BlockEnd::BlockQuote))
    )
}

fn parse_list(
    events: &mut MarkdownEvents<'_>,
    start: Option<u64>,
    nesting: usize,
) -> MarkdownBlock {
    let mut items = Vec::new();

    while let Some(event) = events.next() {
        match event {
            MarkdownEvent::Start(Tag::Item) => items.push(parse_list_item(events, nesting)),
            MarkdownEvent::End(TagEnd::List(_)) => break,
            _ => {}
        }
    }

    MarkdownBlock::List { start, items }
}

fn parse_list_item(events: &mut MarkdownEvents<'_>, nesting: usize) -> Vec<MarkdownBlock> {
    let mut blocks = Vec::new();
    let mut inline_events = Vec::new();

    while let Some(event) = events.next() {
        match event {
            MarkdownEvent::End(TagEnd::Item) => {
                push_tight_list_text(&mut blocks, &mut inline_events);
                break;
            }
            MarkdownEvent::Start(Tag::Paragraph) => {
                push_tight_list_text(&mut blocks, &mut inline_events);
                blocks.push(MarkdownBlock::Paragraph {
                    content: parse_inlines(events, InlineEnd::Paragraph, 0),
                });
            }
            MarkdownEvent::Start(Tag::Heading { level, .. }) => {
                push_tight_list_text(&mut blocks, &mut inline_events);
                blocks.push(MarkdownBlock::Heading {
                    level: level as u8,
                    content: parse_inlines(events, InlineEnd::Heading, 0),
                });
            }
            MarkdownEvent::Start(Tag::BlockQuote(_)) => {
                push_tight_list_text(&mut blocks, &mut inline_events);
                if nesting < MARKDOWN_MAX_NESTING {
                    blocks.push(MarkdownBlock::BlockQuote {
                        blocks: parse_blocks(events, Some(BlockEnd::BlockQuote), nesting + 1),
                    });
                } else {
                    push_flattened_block(events, &mut blocks);
                }
            }
            MarkdownEvent::Start(Tag::List(start)) => {
                push_tight_list_text(&mut blocks, &mut inline_events);
                if nesting < MARKDOWN_MAX_NESTING {
                    blocks.push(parse_list(events, start, nesting + 1));
                } else {
                    push_flattened_block(events, &mut blocks);
                }
            }
            MarkdownEvent::Start(Tag::CodeBlock(kind)) => {
                push_tight_list_text(&mut blocks, &mut inline_events);
                blocks.push(parse_code_block(events, kind));
            }
            MarkdownEvent::Start(Tag::Table(_)) => {
                push_tight_list_text(&mut blocks, &mut inline_events);
                blocks.push(parse_table(events));
            }
            MarkdownEvent::Start(Tag::HtmlBlock) => {
                push_tight_list_text(&mut blocks, &mut inline_events);
                skip_until(events, TagEnd::HtmlBlock);
            }
            MarkdownEvent::Rule => {
                push_tight_list_text(&mut blocks, &mut inline_events);
                blocks.push(MarkdownBlock::ThematicBreak);
            }
            event => inline_events.push(event),
        }
    }

    blocks
}

fn push_tight_list_text(
    blocks: &mut Vec<MarkdownBlock>,
    inline_events: &mut Vec<MarkdownEvent<'_>>,
) {
    if inline_events.is_empty() {
        return;
    }

    let mut events = std::mem::take(inline_events);
    events.push(MarkdownEvent::End(TagEnd::Item));
    let mut events = events.into_iter().peekable();
    let content = parse_inlines(&mut events, InlineEnd::Item, 0);
    if !content.is_empty() {
        blocks.push(MarkdownBlock::Paragraph { content });
    }
}

fn push_flattened_block(events: &mut MarkdownEvents<'_>, blocks: &mut Vec<MarkdownBlock>) {
    let content = flatten_block(events);
    if !content.is_empty() {
        blocks.push(MarkdownBlock::Paragraph { content });
    }
}

fn flatten_block(events: &mut MarkdownEvents<'_>) -> Vec<MarkdownInline> {
    let mut content = Vec::new();
    let mut nesting = 1;

    while let Some(event) = events.next() {
        match event {
            MarkdownEvent::Start(Tag::BlockQuote(_)) | MarkdownEvent::Start(Tag::List(_)) => {
                nesting += 1;
            }
            MarkdownEvent::End(TagEnd::BlockQuote(_)) | MarkdownEvent::End(TagEnd::List(_)) => {
                nesting -= 1;
                if nesting == 0 {
                    break;
                }
            }
            event => push_plain_inline(&mut content, event),
        }
    }

    content
}

fn parse_code_block(events: &mut MarkdownEvents<'_>, kind: CodeBlockKind<'_>) -> MarkdownBlock {
    let language = match kind {
        CodeBlockKind::Indented => None,
        CodeBlockKind::Fenced(info) => info
            .split_whitespace()
            .next()
            .filter(|language| !language.is_empty())
            .map(str::to_owned),
    };
    let mut content = String::new();

    while let Some(event) = events.next() {
        match event {
            MarkdownEvent::Text(text) | MarkdownEvent::Code(text) => content.push_str(&text),
            MarkdownEvent::End(TagEnd::CodeBlock) => break,
            _ => {}
        }
    }

    MarkdownBlock::CodeBlock { language, content }
}

fn parse_table(events: &mut MarkdownEvents<'_>) -> MarkdownBlock {
    let mut header = Vec::new();
    let mut rows = Vec::new();

    while let Some(event) = events.next() {
        match event {
            MarkdownEvent::Start(Tag::TableHead) => {
                header = parse_table_header(events);
            }
            MarkdownEvent::Start(Tag::TableRow) => {
                rows.push(parse_table_row(events));
            }
            MarkdownEvent::End(TagEnd::Table) => break,
            _ => {}
        }
    }

    MarkdownBlock::Table { header, rows }
}

fn parse_table_header(events: &mut MarkdownEvents<'_>) -> Vec<Vec<MarkdownInline>> {
    let mut cells = Vec::new();

    while let Some(event) = events.next() {
        match event {
            MarkdownEvent::Start(Tag::TableCell) => {
                cells.push(parse_inlines(events, InlineEnd::TableCell, 0));
            }
            MarkdownEvent::End(TagEnd::TableHead) => break,
            _ => {}
        }
    }

    cells
}

fn parse_table_row(events: &mut MarkdownEvents<'_>) -> Vec<Vec<MarkdownInline>> {
    let mut cells = Vec::new();

    while let Some(event) = events.next() {
        match event {
            MarkdownEvent::Start(Tag::TableCell) => {
                cells.push(parse_inlines(events, InlineEnd::TableCell, 0));
            }
            MarkdownEvent::End(TagEnd::TableRow) => break,
            _ => {}
        }
    }

    cells
}

fn parse_inlines(
    events: &mut MarkdownEvents<'_>,
    end: InlineEnd,
    nesting: usize,
) -> Vec<MarkdownInline> {
    let mut content = Vec::new();

    while let Some(event) = events.next() {
        if let MarkdownEvent::End(tag_end) = &event {
            if matches_inline_end(tag_end, end) {
                break;
            }
        }

        match event {
            MarkdownEvent::Text(text) => content.push(MarkdownInline::Text {
                content: text.into_string(),
            }),
            MarkdownEvent::Code(text) => content.push(MarkdownInline::Code {
                content: text.into_string(),
            }),
            MarkdownEvent::SoftBreak | MarkdownEvent::HardBreak => {
                content.push(MarkdownInline::Break);
            }
            MarkdownEvent::TaskListMarker(checked) => content.push(MarkdownInline::Text {
                content: if checked {
                    "☑ ".into()
                } else {
                    "☐ ".into()
                },
            }),
            MarkdownEvent::FootnoteReference(label) => content.push(MarkdownInline::Text {
                content: format!("[{label}]"),
            }),
            MarkdownEvent::InlineMath(value) | MarkdownEvent::DisplayMath(value) => {
                content.push(MarkdownInline::Code {
                    content: value.into_string(),
                });
            }
            MarkdownEvent::Start(Tag::Emphasis) => {
                if nesting < MARKDOWN_MAX_NESTING {
                    content.push(MarkdownInline::Emphasis {
                        content: parse_inlines(events, InlineEnd::Emphasis, nesting + 1),
                    });
                } else {
                    content.extend(flatten_inlines(events, InlineEnd::Emphasis));
                }
            }
            MarkdownEvent::Start(Tag::Strong) => {
                if nesting < MARKDOWN_MAX_NESTING {
                    content.push(MarkdownInline::Strong {
                        content: parse_inlines(events, InlineEnd::Strong, nesting + 1),
                    });
                } else {
                    content.extend(flatten_inlines(events, InlineEnd::Strong));
                }
            }
            MarkdownEvent::Start(Tag::Strikethrough) => {
                if nesting < MARKDOWN_MAX_NESTING {
                    content.push(MarkdownInline::Strikethrough {
                        content: parse_inlines(events, InlineEnd::Strikethrough, nesting + 1),
                    });
                } else {
                    content.extend(flatten_inlines(events, InlineEnd::Strikethrough));
                }
            }
            MarkdownEvent::Start(Tag::Link { dest_url, .. }) => {
                let link_content = if nesting < MARKDOWN_MAX_NESTING {
                    parse_inlines(events, InlineEnd::Link, nesting + 1)
                } else {
                    flatten_inlines(events, InlineEnd::Link)
                };
                if nesting < MARKDOWN_MAX_NESTING {
                    if let Some(href) = safe_link_href(&dest_url) {
                        content.push(MarkdownInline::Link {
                            href,
                            content: link_content,
                        });
                    } else {
                        content.extend(link_content);
                    }
                } else {
                    content.extend(link_content);
                }
            }
            MarkdownEvent::Start(Tag::Image { .. }) => {
                if nesting < MARKDOWN_MAX_NESTING {
                    content.extend(parse_inlines(events, InlineEnd::Image, nesting + 1));
                } else {
                    content.extend(flatten_inlines(events, InlineEnd::Image));
                }
            }
            _ => {}
        }
    }

    content
}

fn flatten_inlines(events: &mut MarkdownEvents<'_>, expected: InlineEnd) -> Vec<MarkdownInline> {
    let mut content = Vec::new();
    let mut nesting = 1;

    while let Some(event) = events.next() {
        match &event {
            MarkdownEvent::Start(tag) if starts_inline_end(tag, expected) => nesting += 1,
            MarkdownEvent::End(tag_end) if matches_inline_end(tag_end, expected) => {
                nesting -= 1;
                if nesting == 0 {
                    break;
                }
            }
            _ => push_plain_inline(&mut content, event),
        }
    }

    content
}

fn starts_inline_end(tag: &Tag<'_>, expected: InlineEnd) -> bool {
    matches!(
        (tag, expected),
        (Tag::Emphasis, InlineEnd::Emphasis)
            | (Tag::Strong, InlineEnd::Strong)
            | (Tag::Strikethrough, InlineEnd::Strikethrough)
            | (Tag::Link { .. }, InlineEnd::Link)
            | (Tag::Image { .. }, InlineEnd::Image)
    )
}

fn push_plain_inline(content: &mut Vec<MarkdownInline>, event: MarkdownEvent<'_>) {
    match event {
        MarkdownEvent::Text(text) => content.push(MarkdownInline::Text {
            content: text.into_string(),
        }),
        MarkdownEvent::Code(text) => content.push(MarkdownInline::Code {
            content: text.into_string(),
        }),
        MarkdownEvent::SoftBreak | MarkdownEvent::HardBreak => content.push(MarkdownInline::Break),
        MarkdownEvent::TaskListMarker(checked) => content.push(MarkdownInline::Text {
            content: if checked {
                "☑ ".into()
            } else {
                "☐ ".into()
            },
        }),
        MarkdownEvent::FootnoteReference(label) => content.push(MarkdownInline::Text {
            content: format!("[{label}]"),
        }),
        MarkdownEvent::InlineMath(value) | MarkdownEvent::DisplayMath(value) => {
            content.push(MarkdownInline::Code {
                content: value.into_string(),
            });
        }
        _ => {}
    }
}

fn matches_inline_end(tag_end: &TagEnd, expected: InlineEnd) -> bool {
    matches!(
        (tag_end, expected),
        (TagEnd::Paragraph, InlineEnd::Paragraph)
            | (TagEnd::Heading(_), InlineEnd::Heading)
            | (TagEnd::TableCell, InlineEnd::TableCell)
            | (TagEnd::Item, InlineEnd::Item)
            | (TagEnd::Emphasis, InlineEnd::Emphasis)
            | (TagEnd::Strong, InlineEnd::Strong)
            | (TagEnd::Strikethrough, InlineEnd::Strikethrough)
            | (TagEnd::Link, InlineEnd::Link)
            | (TagEnd::Image, InlineEnd::Image)
    )
}

fn safe_link_href(href: &str) -> Option<String> {
    let normalized = href.trim();
    (normalized.starts_with('#')
        || normalized.starts_with("https://")
        || normalized.starts_with("http://")
        || normalized.starts_with("mailto:"))
    .then(|| normalized.to_string())
}

fn skip_until(events: &mut MarkdownEvents<'_>, expected: TagEnd) {
    while let Some(event) = events.next() {
        if matches!(event, MarkdownEvent::End(tag_end) if tag_end == expected) {
            break;
        }
    }
}

fn bounded_docx_xml(reader: &mut impl Read, declared_size: u64) -> Result<Vec<u8>, AppError> {
    if declared_size > DOCX_XML_MAX_BYTES as u64 {
        return Err(docx_xml_limit_error());
    }

    let mut xml = Vec::new();
    xml.try_reserve_exact(declared_size as usize).map_err(|e| {
        AppError::Internal(format!("DOCX extraction could not allocate buffer: {e}"))
    })?;
    if !read_bounded_bytes(reader, &mut xml, DOCX_XML_MAX_BYTES, "DOCX extraction")? {
        return Err(docx_xml_limit_error());
    }

    Ok(xml)
}

fn docx_xml_limit_error() -> AppError {
    AppError::InvalidInput(format!(
        "DOCX document XML exceeds the {DOCX_XML_MAX_BYTES}-byte extraction limit"
    ))
}

fn extracted_text_limit_error() -> AppError {
    AppError::InvalidInput(format!(
        "Extracted document text exceeds the {EXTRACTED_TEXT_MAX_BYTES}-byte extraction limit"
    ))
}

fn append_extracted_text(text: &mut String, suffix: &str) -> Result<(), AppError> {
    let Some(total_bytes) = text.len().checked_add(suffix.len()) else {
        return Err(extracted_text_limit_error());
    };
    if total_bytes > EXTRACTED_TEXT_MAX_BYTES {
        return Err(extracted_text_limit_error());
    }
    text.push_str(suffix);
    Ok(())
}

fn ensure_extracted_pages_within_limit(pages: &[(i32, String)]) -> Result<(), AppError> {
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

fn pages_from_page_texts(
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

fn nonpaginated_page(text: String, format: &str) -> Result<String, AppError> {
    let text = text.trim();
    if text.is_empty() {
        return Err(AppError::Io(format!("No text extracted from {format}")));
    }
    Ok(text.to_string())
}

#[cfg(test)]
mod tests {
    use super::{
        BoundedPdfText, DOCX_XML_MAX_BYTES, EXTRACTED_TEXT_MAX_BYTES, InlineEnd,
        MARKDOWN_MAX_NESTING, PDF_MAX_PAGES, PDF_MAX_RETAINED_TEXT_BYTES, PDF_MAX_SOURCE_BYTES,
        TEXT_PREVIEW_MAX_BYTES, append_extracted_text, bounded_docx_xml, bounded_utf8_text_reader,
        document_preview, docx_body_text, ensure_extracted_pages_within_limit,
        ensure_pdf_page_count_within_limit, ensure_pdf_source_within_limit, extract_pages,
        markdown_blocks, markdown_preview_blocks, nonpaginated_page, pages_from_page_texts,
        parse_inlines, pdf_page_text_limit,
    };
    use domain::{DocumentPreview, MarkdownBlock, MarkdownInline};
    use pulldown_cmark::{Event as MarkdownEvent, Tag, TagEnd};
    use std::{
        fs::File,
        io::{Cursor, Write},
    };
    use zip::{ZipWriter, write::SimpleFileOptions};

    #[test]
    fn preserves_physical_page_numbers_when_pdf_pages_are_blank() {
        let pages = pages_from_page_texts(
            vec!["first".into(), String::new(), "third".into()],
            3,
            "PDF",
        )
        .expect("text should produce pages");

        assert_eq!(pages, vec![(1, "first".into()), (3, "third".into())]);
    }

    #[test]
    fn rejects_documents_with_no_extractable_text() {
        assert!(pages_from_page_texts(vec![String::new(), "  ".into()], 2, "PDF").is_err());
        assert!(nonpaginated_page("  ".into(), "text document").is_err());
    }

    #[test]
    fn bounds_plain_text_and_accumulated_extracted_text_before_growth() {
        let invalid_utf8 = bounded_utf8_text_reader(&mut Cursor::new(vec![0xFF]), 1)
            .expect_err("malformed text must return a typed error");
        assert!(matches!(invalid_utf8, domain::AppError::InvalidInput(_)));

        let oversized = bounded_utf8_text_reader(
            &mut Cursor::new(vec![b'x'; EXTRACTED_TEXT_MAX_BYTES + 1]),
            0,
        )
        .expect_err("plain text beyond the extraction cap must be rejected");
        assert!(matches!(oversized, domain::AppError::InvalidInput(_)));

        let mut text = "x".repeat(EXTRACTED_TEXT_MAX_BYTES - 1);
        append_extracted_text(&mut text, "x").expect("the exact cap is permitted");
        ensure_extracted_pages_within_limit(&[(1, text.clone())])
            .expect("the exact aggregate cap is permitted");
        assert!(append_extracted_text(&mut text, "x").is_err());
        assert!(
            ensure_extracted_pages_within_limit(&[
                (1, "x".repeat(EXTRACTED_TEXT_MAX_BYTES)),
                (2, "x".to_string()),
            ])
            .is_err()
        );
    }

    #[test]
    fn rejects_partial_pdf_extraction() {
        let error = pages_from_page_texts(vec!["first".into()], 2, "PDF")
            .expect_err("a missing physical page must fail extraction");

        assert!(matches!(error, domain::AppError::Io(_)));
    }
    #[test]
    fn bounds_pdf_input_and_retained_text_before_pages_are_kept() {
        assert!(ensure_pdf_source_within_limit(PDF_MAX_SOURCE_BYTES).is_ok());
        assert!(ensure_pdf_source_within_limit(PDF_MAX_SOURCE_BYTES + 1).is_err());
        assert!(ensure_pdf_page_count_within_limit(PDF_MAX_PAGES).is_ok());
        assert!(ensure_pdf_page_count_within_limit(PDF_MAX_PAGES + 1).is_err());

        assert_eq!(pdf_page_text_limit(0).unwrap(), 4 * 1024 * 1024);
        assert_eq!(
            pdf_page_text_limit(PDF_MAX_RETAINED_TEXT_BYTES - 1).unwrap(),
            1
        );
        assert!(pdf_page_text_limit(PDF_MAX_RETAINED_TEXT_BYTES).is_err());

        let mut output = BoundedPdfText::new(4);
        output.write_all(b"safe").unwrap();
        assert!(output.write_all(b"!").is_err());
        assert!(output.limit_reached);
        assert_eq!(output.into_text(), "safe");
    }

    #[test]
    fn resolves_docx_elements_by_wordprocessingml_namespace_and_local_name() {
        let xml = br#"<doc:document xmlns:doc="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:other="urn:other"><doc:body><doc:p><doc:r><doc:t>First &amp; second</doc:t></doc:r></doc:p><doc:p><doc:r><doc:t>Third</doc:t><doc:tab/><doc:t>column</doc:t></doc:r></doc:p><other:t>ignored</other:t></doc:body></doc:document>"#;

        assert_eq!(
            docx_body_text(xml, false).unwrap(),
            "First & second\nThird\tcolumn\n"
        );
    }

    #[test]
    fn preserves_cdata_inside_docx_text_elements() {
        let xml = br#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:r><w:t><![CDATA[legal <CDATA> & text]]></w:t></w:r></w:p></w:body></w:document>"#;

        assert_eq!(
            docx_body_text(xml, false).unwrap(),
            "legal <CDATA> & text\n"
        );
    }

    #[test]
    fn decodes_utf16_docx_xml_before_parsing() {
        let xml = r#"<?xml version="1.0" encoding="UTF-16"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:r><w:t>UTF-16 text</w:t></w:r></w:p></w:body></w:document>"#;
        let mut utf16 = vec![0xFF, 0xFE];
        utf16.extend(xml.encode_utf16().flat_map(u16::to_le_bytes));

        assert_eq!(docx_body_text(&utf16, false).unwrap(), "UTF-16 text\n");
    }

    #[test]
    fn preserves_accumulated_text_when_a_bounded_docx_ends_mid_markup() {
        let mid_tag = br#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:r><w:t>First paragraph</w:t></w:r></w:p><w:p><w:r><w:"#;
        assert_eq!(docx_body_text(mid_tag, true).unwrap(), "First paragraph\n");
        assert!(docx_body_text(mid_tag, false).is_err());

        let mid_entity = br#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:r><w:t>First paragraph</w:t></w:r></w:p><w:p><w:r><w:t>Second &amp"#;
        assert_eq!(
            docx_body_text(mid_entity, true).unwrap(),
            "First paragraph\nSecond "
        );
        assert!(docx_body_text(mid_entity, false).is_err());
    }

    #[test]
    fn rejects_docx_xml_beyond_declared_or_actual_extraction_limit() {
        let declared_error = bounded_docx_xml(
            &mut Cursor::new(Vec::<u8>::new()),
            DOCX_XML_MAX_BYTES as u64 + 1,
        )
        .expect_err("an oversized declared XML entry must be rejected before allocation");
        assert!(matches!(declared_error, domain::AppError::InvalidInput(_)));

        let actual_error = bounded_docx_xml(&mut Cursor::new(vec![0; DOCX_XML_MAX_BYTES + 1]), 0)
            .expect_err("an XML entry that expands past its declared size must be rejected");
        assert!(matches!(actual_error, domain::AppError::InvalidInput(_)));
    }

    #[test]
    fn extracts_text_from_a_docx_archive() {
        let unique = format!(
            "{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let path = std::env::temp_dir().join(format!("ragtag-docx-{unique}.docx"));
        {
            let file = File::create(&path).unwrap();
            let mut archive = ZipWriter::new(file);
            archive
                .start_file("word/document.xml", SimpleFileOptions::default())
                .unwrap();
            archive
                .write_all(
                    br#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:r><w:t>DOCX text</w:t></w:r></w:p></w:body></w:document>"#,
                )
                .unwrap();
            archive.finish().unwrap();
        }
        let pages = extract_pages(&path, "docx", |_, _| {}).unwrap();
        let preview = document_preview(&path, "docx", "manual.docx").unwrap();
        std::fs::remove_file(path).unwrap();

        assert_eq!(pages.total_pages, 1);
        assert_eq!(pages.pages, vec![(1, "DOCX text".into())]);
        assert_eq!(
            preview,
            DocumentPreview::PlainText {
                content: "DOCX text\n".into(),
                truncated: false,
            }
        );
    }

    #[test]
    fn previews_a_truncated_docx_with_accumulated_text() {
        let unique = format!(
            "{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let path = std::env::temp_dir().join(format!("ragtag-truncated-docx-{unique}.docx"));
        let mut xml = br#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:r><w:t>First paragraph</w:t></w:r></w:p><!--"#
            .to_vec();
        xml.extend(std::iter::repeat_n(
            b'x',
            TEXT_PREVIEW_MAX_BYTES - xml.len() + 1,
        ));
        xml.extend(
            br#"--><w:p><w:r><w:t>Unreached paragraph</w:t></w:r></w:p></w:body></w:document>"#,
        );

        {
            let file = File::create(&path).unwrap();
            let mut archive = ZipWriter::new(file);
            archive
                .start_file("word/document.xml", SimpleFileOptions::default())
                .unwrap();
            archive.write_all(&xml).unwrap();
            archive.finish().unwrap();
        }
        let preview = document_preview(&path, "docx", "manual.docx").unwrap();
        std::fs::remove_file(path).unwrap();

        assert_eq!(
            preview,
            DocumentPreview::PlainText {
                content: "First paragraph\n".into(),
                truncated: true,
            }
        );
    }

    #[test]
    fn reads_a_bounded_utf8_text_preview() {
        let unique = format!(
            "{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let path = std::env::temp_dir().join(format!("ragtag-preview-{unique}.txt"));
        std::fs::write(
            &path,
            format!("{}éafter", "a".repeat(TEXT_PREVIEW_MAX_BYTES - 1)),
        )
        .unwrap();

        let preview = document_preview(&path, "text", "notes.txt").unwrap();
        std::fs::remove_file(path).unwrap();

        assert_eq!(
            preview,
            DocumentPreview::PlainText {
                content: "a".repeat(TEXT_PREVIEW_MAX_BYTES - 1),
                truncated: true,
            }
        );
    }

    #[test]
    fn bounds_deeply_nested_markdown_preview_conversion() {
        let markdown = format!("{}deep", "> ".repeat(MARKDOWN_MAX_NESTING + 1));
        let blocks = markdown_blocks(&markdown);
        assert!(
            max_block_nesting(&blocks) <= MARKDOWN_MAX_NESTING,
            "block nesting must be capped"
        );
        assert!(block_text(&blocks).contains("deep"));

        let mut events = Vec::new();
        for _ in 0..=MARKDOWN_MAX_NESTING {
            events.push(MarkdownEvent::Start(Tag::Emphasis));
        }
        events.push(MarkdownEvent::Text("deep".into()));
        for _ in 0..=MARKDOWN_MAX_NESTING {
            events.push(MarkdownEvent::End(TagEnd::Emphasis));
        }
        events.push(MarkdownEvent::End(TagEnd::Paragraph));
        let mut events = events.into_iter().peekable();
        let inlines = parse_inlines(&mut events, InlineEnd::Paragraph, 0);

        assert!(
            max_inline_nesting(&inlines) <= MARKDOWN_MAX_NESTING,
            "inline nesting must be capped"
        );
        assert!(inline_text(&inlines).contains("deep"));
    }

    #[test]
    fn preserves_direct_text_in_tight_list_items() {
        let blocks = markdown_blocks("- first item\n- second **item**\n");
        let [MarkdownBlock::List { items, .. }] = blocks.as_slice() else {
            panic!("a Markdown list must produce a typed list block");
        };

        assert!(matches!(
            items[0].as_slice(),
            [MarkdownBlock::Paragraph { content }]
                if content == &vec![MarkdownInline::Text { content: "first item".into() }]
        ));
        assert!(matches!(
            items[1].as_slice(),
            [MarkdownBlock::Paragraph { content }]
                if matches!(
                    content.as_slice(),
                    [
                        MarkdownInline::Text { content: prefix },
                        MarkdownInline::Strong { content: strong_content }
                    ] if prefix == "second "
                        && strong_content == &vec![MarkdownInline::Text { content: "item".into() }]
                )
        ));
    }

    fn max_block_nesting(blocks: &[MarkdownBlock]) -> usize {
        blocks
            .iter()
            .map(|block| match block {
                MarkdownBlock::BlockQuote { blocks } => 1 + max_block_nesting(blocks),
                MarkdownBlock::List { items, .. } => items
                    .iter()
                    .map(|item| 1 + max_block_nesting(item))
                    .max()
                    .unwrap_or(1),
                _ => 0,
            })
            .max()
            .unwrap_or_default()
    }

    fn block_text(blocks: &[MarkdownBlock]) -> String {
        blocks
            .iter()
            .map(|block| match block {
                MarkdownBlock::Heading { content, .. } | MarkdownBlock::Paragraph { content } => {
                    inline_text(content)
                }
                MarkdownBlock::BlockQuote { blocks } => block_text(blocks),
                MarkdownBlock::List { items, .. } => {
                    items.iter().map(|item| block_text(item)).collect()
                }
                MarkdownBlock::CodeBlock { content, .. } => content.clone(),
                MarkdownBlock::Table { header, rows } => header
                    .iter()
                    .chain(rows.iter().flatten())
                    .map(|cell| inline_text(cell))
                    .collect(),
                MarkdownBlock::ThematicBreak => String::new(),
            })
            .collect()
    }

    fn max_inline_nesting(inlines: &[MarkdownInline]) -> usize {
        inlines
            .iter()
            .map(|inline| match inline {
                MarkdownInline::Emphasis { content }
                | MarkdownInline::Strong { content }
                | MarkdownInline::Strikethrough { content }
                | MarkdownInline::Link { content, .. } => 1 + max_inline_nesting(content),
                _ => 0,
            })
            .max()
            .unwrap_or_default()
    }

    fn inline_text(inlines: &[MarkdownInline]) -> String {
        inlines
            .iter()
            .map(|inline| match inline {
                MarkdownInline::Text { content } | MarkdownInline::Code { content } => {
                    content.clone()
                }
                MarkdownInline::Emphasis { content }
                | MarkdownInline::Strong { content }
                | MarkdownInline::Strikethrough { content }
                | MarkdownInline::Link { content, .. } => inline_text(content),
                MarkdownInline::Break => "\n".into(),
            })
            .collect()
    }

    #[test]
    fn omits_canonical_leading_yaml_front_matter_from_markdown_previews() {
        let blocks = markdown_preview_blocks(
            "---\n\
             title: Private metadata\n\
             depends: [overview.md]\n\
             ---\n\
             # Visible document\n",
        );

        assert!(!block_text(&blocks).contains("Private metadata"));
        assert!(matches!(
            blocks.as_slice(),
            [MarkdownBlock::Heading { level: 1, content }]
                if content == &vec![MarkdownInline::Text {
                    content: "Visible document".into()
                }]
        ));
    }

    #[test]
    fn preserves_yaml_shaped_blocks_after_visible_markdown() {
        let blocks = markdown_preview_blocks(
            "# Visible document\n\
             \n\
             ---\n\
             title: Public metadata\n\
             ---\n",
        );

        assert!(block_text(&blocks).contains("Public metadata"));
        assert!(
            blocks
                .iter()
                .any(|block| matches!(block, MarkdownBlock::ThematicBreak))
        );
    }

    #[test]
    fn preserves_incomplete_leading_yaml_front_matter() {
        let blocks = markdown_preview_blocks(
            "---\n\
             title: Still visible\n\
             depends: [overview.md]\n",
        );

        assert!(block_text(&blocks).contains("Still visible"));
    }

    #[test]
    fn preserves_empty_leading_metadata_delimiters_as_thematic_rules() {
        let blocks = markdown_preview_blocks("---\n---\n");

        assert!(matches!(
            blocks.as_slice(),
            [MarkdownBlock::ThematicBreak, MarkdownBlock::ThematicBreak]
        ));
    }

    #[test]
    fn omits_bom_prefixed_leading_yaml_front_matter_from_markdown_previews() {
        let blocks = markdown_preview_blocks(
            "\u{feff}---\n\
             title: Private metadata\n\
             ...\n\
             # Visible document\n",
        );

        assert!(!block_text(&blocks).contains("Private metadata"));
        assert!(matches!(
            blocks.as_slice(),
            [MarkdownBlock::Heading { level: 1, content }]
                if content == &vec![MarkdownInline::Text {
                    content: "Visible document".into()
                }]
        ));
    }

    #[test]
    fn omits_bom_from_markdown_previews_without_front_matter() {
        let blocks = markdown_preview_blocks("\u{feff}# Visible document\n");

        assert!(matches!(
            blocks.as_slice(),
            [MarkdownBlock::Heading { level: 1, content }]
                if content == &vec![MarkdownInline::Text {
                    content: "Visible document".into()
                }]
        ));
    }

    #[test]
    fn parses_markdown_into_safe_typed_blocks() {
        let unique = format!(
            "{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let path = std::env::temp_dir().join(format!("ragtag-markdown-{unique}.md"));
        std::fs::write(
            &path,
            "# Plan\n\nA **safe** [link](https://example.test) and [unsafe](javascript:alert).\n\n<script>alert('ignored')</script>\n\n```rust\nlet value = 1;\n```\n",
        )
        .unwrap();

        let preview = document_preview(&path, "text", "plan.md").unwrap();
        std::fs::remove_file(path).unwrap();

        let DocumentPreview::Markdown { blocks, truncated } = preview else {
            panic!("Markdown sources must produce typed Markdown blocks");
        };
        assert!(!truncated);
        assert!(blocks.iter().any(|block| matches!(
            block,
            MarkdownBlock::Heading { level: 1, content }
                if content == &vec![MarkdownInline::Text { content: "Plan".into() }]
        )));
        assert!(blocks.iter().any(|block| matches!(
            block,
            MarkdownBlock::CodeBlock { language: Some(language), content }
                if language == "rust" && content == "let value = 1;\n"
        )));
        assert!(blocks.iter().any(|block| matches!(
            block,
            MarkdownBlock::Paragraph { content }
                if content.iter().any(|inline| matches!(
                    inline,
                    MarkdownInline::Link { href, .. } if href == "https://example.test"
                ))
                    && !content.iter().any(|inline| matches!(
                        inline,
                        MarkdownInline::Link { href, .. } if href.starts_with("javascript:")
                    ))
        )));
        assert!(!blocks.iter().any(|block| matches!(
            block,
            MarkdownBlock::Paragraph { content }
                if content.iter().any(|inline| matches!(
                    inline,
                    MarkdownInline::Text { content } if content.contains("alert")
                ))
        )));
    }

    #[test]
    fn previews_nonmarkdown_utf8_sources_as_plain_text() {
        let unique = format!(
            "{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let path = std::env::temp_dir().join(format!("ragtag-config-{unique}.toml"));
        std::fs::write(&path, "[preview]\nenabled = true\n").unwrap();

        let preview = document_preview(&path, "text", "settings.toml").unwrap();
        std::fs::remove_file(path).unwrap();

        assert_eq!(
            preview,
            DocumentPreview::PlainText {
                content: "[preview]\nenabled = true\n".into(),
                truncated: false,
            }
        );
    }

    #[test]
    fn strips_bom_from_utf8_text_extraction_and_preview() {
        let unique = format!(
            "{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let path = std::env::temp_dir().join(format!("ragtag-bom-{unique}.txt"));
        let bom_content = "\u{feff}Hello, world!";
        std::fs::write(&path, bom_content).unwrap();

        let extracted = extract_pages(&path, "text", |_, _| {}).unwrap();
        let preview = document_preview(&path, "text", "test.txt").unwrap();
        std::fs::remove_file(path).unwrap();

        assert_eq!(extracted.pages[0].1, "Hello, world!");
        assert_eq!(
            preview,
            DocumentPreview::PlainText {
                content: "Hello, world!".into(),
                truncated: false,
            }
        );
    }

    #[test]
    fn empty_docx_paragraphs_preserve_newlines() {
        let xml = br#"<?xml version="1.0" encoding="UTF-8"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:body>
    <w:p><w:r><w:t>First line</w:t></w:r></w:p>
    <w:p/>
    <w:p><w:r><w:t>Second line</w:t></w:r></w:p>
  </w:body>
</w:document>"#;
        let text = docx_body_text(xml, false).unwrap();
        assert_eq!(text, "First line\n\nSecond line\n");
    }
}
