use domain::{MarkdownBlock, MarkdownInline};
use pulldown_cmark::{
    CodeBlockKind, Event as MarkdownEvent, MetadataBlockKind, Options, Parser, Tag, TagEnd,
};
use std::path::Path;

pub const MARKDOWN_MAX_NESTING: usize = 32;

pub type MarkdownEvents<'a> = std::iter::Peekable<std::vec::IntoIter<MarkdownEvent<'a>>>;

#[derive(Clone, Copy)]
pub enum BlockEnd {
    BlockQuote,
}

#[derive(Clone, Copy)]
pub enum InlineEnd {
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

pub fn is_markdown_filename(filename: &str) -> bool {
    Path::new(filename)
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            extension.eq_ignore_ascii_case("md") || extension.eq_ignore_ascii_case("markdown")
        })
}

pub fn markdown_preview_blocks(markdown: &str) -> Vec<MarkdownBlock> {
    markdown_blocks(without_leading_yaml_front_matter(markdown))
}

pub fn without_leading_yaml_front_matter(markdown: &str) -> &str {
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

pub fn markdown_blocks(markdown: &str) -> Vec<MarkdownBlock> {
    let options =
        Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TABLES | Options::ENABLE_TASKLISTS;
    let mut events = Parser::new_ext(markdown, options)
        .collect::<Vec<_>>()
        .into_iter()
        .peekable();
    parse_blocks(&mut events, None, 0)
}

pub fn parse_blocks(
    events: &mut MarkdownEvents<'_>,
    end: Option<BlockEnd>,
    nesting: usize,
) -> Vec<MarkdownBlock> {
    let mut blocks = Vec::new();

    while let Some(event) = events.next() {
        if let MarkdownEvent::End(tag_end) = &event
            && matches_block_end(tag_end, end)
        {
            break;
        }

        push_block(event, events, &mut blocks, nesting);
    }

    blocks
}

fn is_block_event(event: &MarkdownEvent<'_>) -> bool {
    matches!(
        event,
        MarkdownEvent::Start(
            Tag::Paragraph
                | Tag::Heading { .. }
                | Tag::BlockQuote(_)
                | Tag::List(_)
                | Tag::CodeBlock(_)
                | Tag::Table(_)
                | Tag::HtmlBlock
        ) | MarkdownEvent::Rule
    )
}

fn push_block(
    event: MarkdownEvent<'_>,
    events: &mut MarkdownEvents<'_>,
    blocks: &mut Vec<MarkdownBlock>,
    nesting: usize,
) {
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
                push_flattened_block(events, blocks);
            }
        }
        MarkdownEvent::Start(Tag::List(start)) => {
            if nesting < MARKDOWN_MAX_NESTING {
                blocks.push(parse_list(events, start, nesting + 1));
            } else {
                push_flattened_block(events, blocks);
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

pub fn matches_block_end(tag_end: &TagEnd, expected: Option<BlockEnd>) -> bool {
    matches!(
        (tag_end, expected),
        (TagEnd::BlockQuote(_), Some(BlockEnd::BlockQuote))
    )
}

pub fn parse_list(
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

pub fn parse_list_item(events: &mut MarkdownEvents<'_>, nesting: usize) -> Vec<MarkdownBlock> {
    let mut blocks = Vec::new();
    let mut inline_events = Vec::new();

    while let Some(event) = events.next() {
        match event {
            MarkdownEvent::End(TagEnd::Item) => {
                push_tight_list_text(&mut blocks, &mut inline_events);
                break;
            }
            event if is_block_event(&event) => {
                push_tight_list_text(&mut blocks, &mut inline_events);
                push_block(event, events, &mut blocks, nesting);
            }
            event => inline_events.push(event),
        }
    }

    blocks
}

pub fn push_tight_list_text(
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

pub fn push_flattened_block(events: &mut MarkdownEvents<'_>, blocks: &mut Vec<MarkdownBlock>) {
    let content = flatten_block(events);
    if !content.is_empty() {
        blocks.push(MarkdownBlock::Paragraph { content });
    }
}

pub fn flatten_block(events: &mut MarkdownEvents<'_>) -> Vec<MarkdownInline> {
    let mut content = Vec::new();
    let mut nesting = 1;

    for event in events.by_ref() {
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

pub fn parse_code_block(events: &mut MarkdownEvents<'_>, kind: CodeBlockKind<'_>) -> MarkdownBlock {
    let language = match kind {
        CodeBlockKind::Indented => None,
        CodeBlockKind::Fenced(info) => info
            .split_whitespace()
            .next()
            .filter(|language| !language.is_empty())
            .map(str::to_owned),
    };
    let mut content = String::new();

    for event in events.by_ref() {
        match event {
            MarkdownEvent::Text(text) | MarkdownEvent::Code(text) => content.push_str(&text),
            MarkdownEvent::End(TagEnd::CodeBlock) => break,
            _ => {}
        }
    }

    MarkdownBlock::CodeBlock { language, content }
}

pub fn parse_table(events: &mut MarkdownEvents<'_>) -> MarkdownBlock {
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

pub fn parse_table_header(events: &mut MarkdownEvents<'_>) -> Vec<Vec<MarkdownInline>> {
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

pub fn parse_table_row(events: &mut MarkdownEvents<'_>) -> Vec<Vec<MarkdownInline>> {
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

pub fn parse_inlines(
    events: &mut MarkdownEvents<'_>,
    end: InlineEnd,
    nesting: usize,
) -> Vec<MarkdownInline> {
    let mut content = Vec::new();

    while let Some(event) = events.next() {
        if let MarkdownEvent::End(tag_end) = &event
            && matches_inline_end(tag_end, end)
        {
            break;
        }

        match event {
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
            event => push_plain_inline(&mut content, event),
        }
    }

    content
}

pub fn flatten_inlines(
    events: &mut MarkdownEvents<'_>,
    expected: InlineEnd,
) -> Vec<MarkdownInline> {
    let mut content = Vec::new();
    let mut nesting = 1;

    for event in events.by_ref() {
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

pub fn starts_inline_end(tag: &Tag<'_>, expected: InlineEnd) -> bool {
    matches!(
        (tag, expected),
        (Tag::Emphasis, InlineEnd::Emphasis)
            | (Tag::Strong, InlineEnd::Strong)
            | (Tag::Strikethrough, InlineEnd::Strikethrough)
            | (Tag::Link { .. }, InlineEnd::Link)
            | (Tag::Image { .. }, InlineEnd::Image)
    )
}

pub fn push_plain_inline(content: &mut Vec<MarkdownInline>, event: MarkdownEvent<'_>) {
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

pub fn matches_inline_end(tag_end: &TagEnd, expected: InlineEnd) -> bool {
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

pub fn safe_link_href(href: &str) -> Option<String> {
    let normalized = href.trim();
    let lower = normalized.to_ascii_lowercase();
    (normalized.starts_with('#')
        || lower.starts_with("https://")
        || lower.starts_with("http://")
        || lower.starts_with("mailto:"))
    .then(|| normalized.to_string())
}

pub fn skip_until(events: &mut MarkdownEvents<'_>, expected: TagEnd) {
    for event in events.by_ref() {
        if matches!(event, MarkdownEvent::End(tag_end) if tag_end == expected) {
            break;
        }
    }
}
