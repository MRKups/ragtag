pub struct Chunk {
    pub page: i32,
    pub chunk_index: i32,
    pub content: String,
}

const CHARS_PER_TOKEN: usize = 4;

pub fn chunk_pages(
    pages: Vec<(i32, String)>,
    chunk_size_tokens: usize,
    overlap_tokens: usize,
) -> Vec<Chunk> {
    let chunk_size_chars = chunk_size_tokens * CHARS_PER_TOKEN;
    let overlap_chars = overlap_tokens * CHARS_PER_TOKEN;
    let stride = chunk_size_chars.saturating_sub(overlap_chars).max(1);

    let mut chunks = Vec::new();

    for (page, text) in &pages {
        let clean = text.trim();
        if clean.is_empty() {
            continue;
        }

        let chars: Vec<char> = clean.chars().collect();
        let mut chunk_index = 0;
        let mut start: usize = 0;

        while start < chars.len() {
            let end = (start + chunk_size_chars).min(chars.len());
            let snippet: String = chars[start..end].iter().collect();
            let content = format!("passage: {}", snippet.trim());

            chunks.push(Chunk {
                page: *page,
                chunk_index,
                content,
            });

            chunk_index += 1;
            start = start.saturating_add(stride);
        }
    }

    chunks
}
