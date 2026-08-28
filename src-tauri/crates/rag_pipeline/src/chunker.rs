pub struct Chunk {
    pub page: i32,
    pub chunk_index: i32,
    pub content: String,
}

/// Approximate token counts with a character window so chunking does not depend on
/// a model-specific tokenizer.
const CHARS_PER_ESTIMATED_TOKEN: usize = 4;

/// Caps overlap so every successive window makes meaningful progress even when
/// callers bypass settings validation.
const MAX_OVERLAP_FRACTION_DENOMINATOR: usize = 2;

pub fn chunk_pages(
    pages: Vec<(i32, String)>,
    chunk_size_estimated_tokens: usize,
    overlap_estimated_tokens: usize,
) -> Vec<Chunk> {
    let chunk_size_chars = chunk_size_estimated_tokens
        .max(1)
        .saturating_mul(CHARS_PER_ESTIMATED_TOKEN);
    let requested_overlap_chars = overlap_estimated_tokens
        .saturating_mul(CHARS_PER_ESTIMATED_TOKEN);
    let overlap_chars = requested_overlap_chars
        .min(chunk_size_chars / MAX_OVERLAP_FRACTION_DENOMINATOR);
    let stride = chunk_size_chars - overlap_chars;

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

#[cfg(test)]
mod tests {
    use super::chunk_pages;

    #[test]
    fn caps_pathological_overlap_at_half_the_window() {
        let chunks = chunk_pages(vec![(1, "abcdefghi".into())], 1, usize::MAX);
        let contents: Vec<_> = chunks.iter().map(|chunk| chunk.content.as_str()).collect();

        assert_eq!(
            contents,
            [
                "passage: abcd",
                "passage: cdef",
                "passage: efgh",
                "passage: ghi",
                "passage: i",
            ]
        );
    }
}
