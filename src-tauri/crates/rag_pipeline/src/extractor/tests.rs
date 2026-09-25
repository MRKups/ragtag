use super::{
    document_preview,
    docx::{DOCX_XML_MAX_BYTES, bounded_docx_xml, docx_body_text, normalized_docx_xml},
    extract_pages,
    markdown::{
        InlineEnd, MARKDOWN_MAX_NESTING, markdown_blocks, markdown_preview_blocks, parse_inlines,
    },
    pdf::{
        BoundedPdfText, PDF_MAX_PAGES, PDF_MAX_RETAINED_TEXT_BYTES, PDF_MAX_SOURCE_BYTES,
        ensure_pdf_page_count_within_limit, ensure_pdf_source_within_limit, pdf_page_text_limit,
    },
    text::bounded_utf8_text_reader,
    util::{
        EXTRACTED_TEXT_MAX_BYTES, TEXT_PREVIEW_MAX_BYTES, append_extracted_text,
        ensure_extracted_pages_within_limit, nonpaginated_page, pages_from_page_texts,
    },
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
    assert_eq!(invalid_utf8.code, domain::ErrorCode::InvalidInput);

    let oversized = bounded_utf8_text_reader(
        &mut Cursor::new(vec![b'x'; EXTRACTED_TEXT_MAX_BYTES + 1]),
        0,
    )
    .expect_err("plain text beyond the extraction cap must be rejected");
    assert_eq!(oversized.code, domain::ErrorCode::InvalidInput);

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

    assert_eq!(error.code, domain::ErrorCode::InvalidInput);
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
fn preserves_accumulated_text_when_docx_preview_ends_with_truncated_multibyte_utf8() {
    let mut truncated = Vec::from(
        br#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:r><w:t>Price: "#
            .as_slice(),
    );
    truncated.extend_from_slice(&[0xE2, 0x82]);
    assert_eq!(
        docx_body_text(&truncated, true).unwrap(),
        "Price: \u{FFFD}"
    );
    assert!(docx_body_text(&truncated, false).is_err());
}

#[test]
fn rejects_docx_xml_beyond_declared_or_actual_extraction_limit() {
    let declared_error = bounded_docx_xml(
        &mut Cursor::new(Vec::<u8>::new()),
        DOCX_XML_MAX_BYTES as u64 + 1,
    )
    .expect_err("an oversized declared XML entry must be rejected before allocation");
    assert_eq!(declared_error.code, domain::ErrorCode::InvalidInput);

    let actual_error = bounded_docx_xml(&mut Cursor::new(vec![0; DOCX_XML_MAX_BYTES + 1]), 0)
        .expect_err("an XML entry that expands past its declared size must be rejected");
    assert_eq!(actual_error.code, domain::ErrorCode::InvalidInput);
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

#[test]
fn bounded_pdf_text_accepts_split_multibyte_utf8_chunks() {
    use std::io::Write;

    let mut bounded = BoundedPdfText::new(1024);
    // Write partial multibyte UTF-8 for '€' (0xE2, 0x82, 0xAC) across two write calls:
    bounded.write_all(&[0xE2, 0x82]).unwrap();
    bounded.write_all(&[0xAC]).unwrap();

    // Write partial multibyte UTF-8 for 'é' (0xC3, 0xA9) across two write calls:
    bounded.write_all(b" Caf").unwrap();
    bounded.write_all(&[0xC3]).unwrap();
    bounded.write_all(&[0xA9]).unwrap();

    let text = bounded.into_text();
    assert_eq!(text, "€ Café");
}

#[test]
fn decodes_utf16_docx_xml_with_truncated_surrogate_pair() {
    let xml = r#"<?xml version="1.0" encoding="UTF-16"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:r><w:t>Surrogate: "#;
    let mut utf16 = vec![0xFF, 0xFE];
    utf16.extend(xml.encode_utf16().flat_map(u16::to_le_bytes));
    // Append high surrogate 0xD834 without matching low surrogate (simulating stream cut):
    utf16.extend(0xD834u16.to_le_bytes());

    let normalized = normalized_docx_xml(&utf16).unwrap();
    let text = String::from_utf8_lossy(&normalized);
    assert!(text.contains("Surrogate: \u{FFFD}"));
}


