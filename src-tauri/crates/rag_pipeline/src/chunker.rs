use domain::AppError;
use tokenizers::Tokenizer;

pub struct Chunk {
    pub page: i32,
    pub chunk_index: i32,
    pub content: String,
}

const PASSAGE_PREFIX: &str = "passage: ";

/// Caps overlap so every successive window makes meaningful progress even when
/// callers bypass settings validation.
const MAX_OVERLAP_FRACTION_DENOMINATOR: usize = 2;

/// Splits page text at the embedder tokenizer's token boundaries. Every emitted
/// `content` is counted after its `passage:` prefix is added, so it fits within
/// the active embedder's truncation limit.
pub fn chunk_pages(
    pages: Vec<(i32, String)>,
    max_tokens: usize,
    overlap_tokens: usize,
    tokenizer: Tokenizer,
) -> Result<Vec<Chunk>, AppError> {
    let mut tokenizer = tokenizer;
    tokenizer
        .with_truncation(None)
        .map_err(|e| AppError::Internal(format!("Failed to disable tokenizer truncation: {e}")))?;
    let max_tokens = max_tokens.max(1);
    let mut chunks = Vec::new();

    for (page, text) in pages {
        let clean = text.trim();
        if clean.is_empty() {
            continue;
        }

        append_page_chunks(
            &mut chunks,
            page,
            clean,
            max_tokens,
            overlap_tokens,
            &tokenizer,
        )?;
    }

    Ok(chunks)
}

fn append_page_chunks(
    chunks: &mut Vec<Chunk>,
    page: i32,
    text: &str,
    max_tokens: usize,
    overlap_tokens: usize,
    tokenizer: &Tokenizer,
) -> Result<(), AppError> {
    let source_tokens = tokenizer
        .encode(text, false)
        .map_err(|e| AppError::InvalidInput(format!("Failed to tokenize PDF text: {e}")))?;
    let offsets = source_tokens
        .get_offsets()
        .iter()
        .copied()
        .filter(|(start, end)| start < end)
        .collect::<Vec<_>>();

    if offsets.is_empty() {
        push_chunk(chunks, page, text, max_tokens, tokenizer)?;
        return Ok(());
    }

    let prefix_tokens = token_count(tokenizer, PASSAGE_PREFIX)?;
    let content_token_budget = max_tokens.checked_sub(prefix_tokens).ok_or_else(|| {
        AppError::InvalidInput(format!(
            "Embedding token limit ({max_tokens}) is too small for the passage prefix"
        ))
    })?;
    if content_token_budget == 0 {
        return Err(AppError::InvalidInput(format!(
            "Embedding token limit ({max_tokens}) leaves no room for PDF text"
        )));
    }

    let overlap = overlap_tokens.min(content_token_budget / MAX_OVERLAP_FRACTION_DENOMINATOR);
    let mut start_token = 0;
    let mut chunk_index = 0;

    while start_token < offsets.len() {
        let mut end_token = (start_token + content_token_budget).min(offsets.len());
        let mut emitted = false;

        while end_token > start_token {
            let start = offsets[start_token].0;
            let end = offsets[end_token - 1].1;
            let content = &text[start..end];
            let embedding_text = format!("{PASSAGE_PREFIX}{content}");

            if token_count(tokenizer, &embedding_text)? <= max_tokens {
                chunks.push(Chunk {
                    page,
                    chunk_index,
                    content: content.to_string(),
                });
                chunk_index += 1;
                let chunk_token_count = end_token - start_token;
                start_token = end_token - overlap.min(chunk_token_count.saturating_sub(1));
                emitted = true;
                break;
            }

            end_token -= 1;
        }

        if !emitted {
            return Err(AppError::InvalidInput(format!(
                "Embedding token limit ({max_tokens}) is too small for text on page {page}"
            )));
        }
    }

    Ok(())
}

fn push_chunk(
    chunks: &mut Vec<Chunk>,
    page: i32,
    text: &str,
    max_tokens: usize,
    tokenizer: &Tokenizer,
) -> Result<(), AppError> {
    let embedding_text = format!("{PASSAGE_PREFIX}{text}");
    if token_count(tokenizer, &embedding_text)? > max_tokens {
        return Err(AppError::InvalidInput(format!(
            "Embedding token limit ({max_tokens}) is too small for text on page {page}"
        )));
    }

    chunks.push(Chunk {
        page,
        chunk_index: 0,
        content: text.to_string(),
    });
    Ok(())
}

fn token_count(tokenizer: &Tokenizer, text: &str) -> Result<usize, AppError> {
    tokenizer
        .encode(text, true)
        .map(|encoding| encoding.len())
        .map_err(|e| AppError::InvalidInput(format!("Failed to tokenize PDF text: {e}")))
}

#[cfg(test)]
mod tests {
    use super::chunk_pages;
    use domain::AppError;
    use tokenizers::Tokenizer;

    fn tokenizer() -> Tokenizer {
        Tokenizer::from_bytes(
            br#"{
                "version": "1.0",
                "truncation": null,
                "padding": null,
                "added_tokens": [],
                "normalizer": null,
                "pre_tokenizer": {"type": "Whitespace"},
                "post_processor": null,
                "decoder": null,
                "model": {
                    "type": "WordLevel",
                    "vocab": {
                        "<unk>": 0,
                        "passage": 1,
                        "one": 2,
                        "two": 3,
                        "three": 4,
                        "four": 5
                    },
                    "unk_token": "<unk>"
                }
            }"#,
        )
        .expect("test tokenizer should be valid")
    }

    #[test]
    fn keeps_each_embedding_input_within_the_embedder_token_limit() {
        let test_tokenizer = tokenizer();
        const MAX_TOKENS: usize = 8;
        assert!(
            test_tokenizer
                .encode("passage: one", true)
                .expect("test content should tokenize")
                .len()
                <= MAX_TOKENS
        );
        let text = "one two three four ".repeat(20);
        assert!(
            test_tokenizer
                .encode(format!("passage: {text}"), true)
                .expect("test content should tokenize")
                .len()
                > MAX_TOKENS
        );
        let chunks = chunk_pages(vec![(1, text)], MAX_TOKENS, 0, tokenizer())
            .expect("token-safe chunks should be produced");

        assert!(chunks.len() > 1);
        assert!(
            chunks
                .iter()
                .all(|chunk| !chunk.content.starts_with("passage: "))
        );
        assert!(chunks.iter().all(|chunk| {
            test_tokenizer
                .encode(format!("passage: {}", chunk.content), true)
                .expect("test content should tokenize")
                .len()
                <= MAX_TOKENS
        }));
    }

    #[test]
    fn rejects_a_limit_that_cannot_fit_the_passage_prefix() {
        assert!(matches!(
            chunk_pages(vec![(1, "one".into())], 1, 0, tokenizer()),
            Err(AppError::InvalidInput(_))
        ));
    }
}
