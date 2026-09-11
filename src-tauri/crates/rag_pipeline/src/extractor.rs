use domain::{AppError, DocumentPreview, MarkdownBlock, MarkdownInline};
use pulldown_cmark::{
    CodeBlockKind, Event as MarkdownEvent, MetadataBlockKind, Options, Parser, Tag, TagEnd,
};
use quick_xml::{Reader, escape::unescape, events::Event};
use std::{fs::File, io::Read, path::Path};

pub struct ExtractedPages {
    pub pages: Vec<(i32, String)>,
    pub total_pages: usize,
}

pub const TEXT_PREVIEW_MAX_BYTES: usize = 1024 * 1024;
const DOCX_XML_MAX_BYTES: usize = 16 * 1024 * 1024;
const MARKDOWN_MAX_NESTING: usize = 32;
pub fn extract_pages(
    source_path: &Path,
    file_type: &str,
    on_page_extracted: impl Fn(usize, usize),
) -> Result<ExtractedPages, AppError> {
    match file_type {
        "pdf" => extract_pdf_pages(source_path, on_page_extracted),
        "docx" => extract_docx_pages(source_path, on_page_extracted),
        "text" => extract_text_pages(source_path, on_page_extracted),
        _ => Err(AppError::InvalidInput(format!(
            "Unsupported document file type '{file_type}'"
        ))),
    }
}

fn extract_pdf_pages(
    pdf_path: &Path,
    on_page_extracted: impl Fn(usize, usize),
) -> Result<ExtractedPages, AppError> {
    let mut document = pdf_extract::Document::load(pdf_path)
        .map_err(|e| AppError::Io(format!("PDF page enumeration failed: {e}")))?;
    if document.is_encrypted() {
        document
            .decrypt("")
            .map_err(|e| AppError::Io(format!("PDF extraction failed: {e}")))?;
    }

    let total_pages = document.get_pages().len();
    let mut page_texts = Vec::with_capacity(total_pages);
    for page_index in 1..=total_pages {
        let page_number = u32::try_from(page_index)
            .map_err(|_| AppError::Io("PDF has more pages than this extractor supports".into()))?;
        let mut text = String::new();
        let extraction = {
            let mut output = pdf_extract::PlainTextOutput::new(&mut text);
            pdf_extract::output_doc_page(&document, &mut output, page_number)
        };
        if extraction.is_err() {
            break;
        }
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
    let text = std::fs::read_to_string(source_path)
        .map_err(|e| AppError::Io(format!("Text extraction failed: {e}")))?;
    let page = nonpaginated_page(text, "text document")?;
    on_page_extracted(1, 1);
    Ok(ExtractedPages {
        pages: vec![(1, page)],
        total_pages: 1,
    })
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
    Ok((content, truncated))
}

fn preview_bytes(reader: &mut impl Read, operation: &str) -> Result<(Vec<u8>, bool), AppError> {
    let mut bytes = Vec::with_capacity(TEXT_PREVIEW_MAX_BYTES + 1);
    reader
        .take((TEXT_PREVIEW_MAX_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|e| AppError::Io(format!("{operation} could not read source: {e}")))?;

    let truncated = bytes.len() > TEXT_PREVIEW_MAX_BYTES;
    if truncated {
        bytes.truncate(TEXT_PREVIEW_MAX_BYTES);
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
    let (xml, xml_truncated) = preview_bytes(&mut document_xml, "DOCX preview")?;
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
    let mut reader = Reader::from_reader(xml);
    reader.config_mut().trim_text(false);
    let mut buffer = Vec::new();
    let mut in_text = false;
    let mut text = String::new();

    loop {
        match reader.read_event_into(&mut buffer) {
            Ok(Event::Start(event)) => match event.name().as_ref() {
                b"w:t" | b"t" => in_text = true,
                b"w:tab" | b"tab" => text.push('\t'),
                b"w:br" | b"br" | b"w:cr" | b"cr" => text.push('\n'),
                _ => {}
            },
            Ok(Event::Empty(event)) => match event.name().as_ref() {
                b"w:tab" | b"tab" => text.push('\t'),
                b"w:br" | b"br" | b"w:cr" | b"cr" => text.push('\n'),
                _ => {}
            },
            Ok(Event::End(event)) => match event.name().as_ref() {
                b"w:t" | b"t" => in_text = false,
                b"w:p" | b"p" => text.push('\n'),
                _ => {}
            },
            Ok(Event::Text(event)) if in_text => {
                let decoded = event
                    .decode()
                    .map_err(|e| AppError::Io(format!("DOCX text decoding failed: {e}")))?;
                text.push_str(&decoded);
            }
            Ok(Event::GeneralRef(event)) if in_text => {
                let reference = event
                    .decode()
                    .map_err(|e| AppError::Io(format!("DOCX text decoding failed: {e}")))?;
                let encoded = format!("&{reference};");
                let resolved = unescape(&encoded)
                    .map_err(|e| AppError::Io(format!("DOCX text decoding failed: {e}")))?;
                text.push_str(&resolved);
            }
            Ok(Event::Eof) => break,
            Ok(_) => {}
            Err(_) if allow_truncated_eof => break,
            Err(error) => return Err(AppError::Io(format!("DOCX XML parsing failed: {error}"))),
        }
        buffer.clear();
    }

    Ok(text)
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

    let mut xml = Vec::with_capacity(declared_size as usize);
    reader
        .take((DOCX_XML_MAX_BYTES + 1) as u64)
        .read_to_end(&mut xml)
        .map_err(|e| AppError::Io(format!("DOCX extraction failed: {e}")))?;

    if xml.len() > DOCX_XML_MAX_BYTES {
        return Err(docx_xml_limit_error());
    }

    Ok(xml)
}

fn docx_xml_limit_error() -> AppError {
    AppError::InvalidInput(format!(
        "DOCX document XML exceeds the {DOCX_XML_MAX_BYTES}-byte extraction limit"
    ))
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
        DOCX_XML_MAX_BYTES, InlineEnd, MARKDOWN_MAX_NESTING, TEXT_PREVIEW_MAX_BYTES,
        bounded_docx_xml, document_preview, docx_body_text, extract_pages, markdown_blocks,
        markdown_preview_blocks, nonpaginated_page, pages_from_page_texts, parse_inlines,
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
    fn rejects_partial_pdf_extraction() {
        let error = pages_from_page_texts(vec!["first".into()], 2, "PDF")
            .expect_err("a missing physical page must fail extraction");

        assert!(matches!(error, domain::AppError::Io(_)));
    }

    #[test]
    fn extracts_docx_paragraph_text() {
        let xml = br#"<w:document xmlns:w="word"><w:body><w:p><w:r><w:t>First &amp; second</w:t></w:r></w:p><w:p><w:r><w:t>Third</w:t><w:tab/><w:t>column</w:t></w:r></w:p></w:body></w:document>"#;

        assert_eq!(
            docx_body_text(xml, false).unwrap(),
            "First & second\nThird\tcolumn\n"
        );
    }

    #[test]
    fn preserves_accumulated_text_when_a_bounded_docx_ends_mid_markup() {
        let mid_tag = br#"<w:document xmlns:w="word"><w:body><w:p><w:r><w:t>First paragraph</w:t></w:r></w:p><w:p><w:r><w:"#;
        assert_eq!(docx_body_text(mid_tag, true).unwrap(), "First paragraph\n");
        assert!(docx_body_text(mid_tag, false).is_err());

        let mid_entity = br#"<w:document xmlns:w="word"><w:body><w:p><w:r><w:t>First paragraph</w:t></w:r></w:p><w:p><w:r><w:t>Second &amp"#;
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
                    br#"<w:document xmlns:w="word"><w:body><w:p><w:r><w:t>DOCX text</w:t></w:r></w:p></w:body></w:document>"#,
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
        let mut xml = br#"<w:document xmlns:w="word"><w:body><w:p><w:r><w:t>First paragraph</w:t></w:r></w:p><!--"#
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
}
