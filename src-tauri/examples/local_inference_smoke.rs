//! Opt-in in-process Metal check using a synthetic retrieval_eval directory.
//! Usage: cargo run --locked --example local_inference_smoke -- <evaluation-dir> <new-model-cache> [offline]
//! This is not a substitute for the packaged WebView/manual acceptance checks.

use domain::Citation;
use std::{
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

fn request(prompt: &str) -> mistralrs::RequestBuilder {
    mistralrs::RequestBuilder::new()
        .add_message(mistralrs::TextMessageRole::System, "Answer only from the supplied evidence. Be concise and cite the source number. Do not output reasoning. /no_think")
        .add_message(mistralrs::TextMessageRole::User, format!("{prompt}\n/no_think"))
        .set_sampler_temperature(0.0)
        .set_sampler_max_len(256)
}

async fn complete(
    model: &mistralrs::Model,
    prompt: &str,
) -> Result<serde_json::Value, Box<dyn std::error::Error + Send + Sync>> {
    let start = Instant::now();
    let mut stream = model.stream_chat_request(request(prompt)).await?;
    let mut text = String::new();
    let mut first_token = None;
    let mut chunks = 0usize;
    let mut finished = false;
    loop {
        let response = tokio::time::timeout(Duration::from_secs(60), stream.next()).await?;
        match response {
            Some(mistralrs::Response::Chunk(chunk)) => {
                if let Some(choice) = chunk.choices.into_iter().next() {
                    if let Some(content) =
                        choice.delta.content.filter(|content| !content.is_empty())
                    {
                        first_token.get_or_insert(start.elapsed().as_secs_f64());
                        text.push_str(&content);
                        chunks += 1;
                    }
                    if choice.finish_reason.is_some() {
                        finished = true;
                        break;
                    }
                }
            }
            Some(mistralrs::Response::Done(_)) => {
                finished = true;
                break;
            }
            Some(mistralrs::Response::ModelError(error, _)) => return Err(error.into()),
            Some(mistralrs::Response::InternalError(error))
            | Some(mistralrs::Response::ValidationError(error)) => {
                return Err(error.to_string().into());
            }
            None => break,
            Some(_) => {}
        }
    }
    if !finished || text.trim().is_empty() {
        return Err(
            "The real-model stream ended without non-empty content and a completion marker.".into(),
        );
    }
    Ok(
        serde_json::json!({"text":text,"first_content_seconds":first_token,"elapsed_seconds":start.elapsed().as_secs_f64(),"content_chunks":chunks}),
    )
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let args: Vec<_> = std::env::args().collect();
    let evaluation = PathBuf::from(
        args.get(1)
            .ok_or("Pass a synthetic retrieval_eval directory and model-cache directory.")?,
    );
    let cache = PathBuf::from(
        args.get(2)
            .ok_or("Pass a model-cache directory separate from personal application data.")?,
    );
    let offline = args.get(3).is_some_and(|value| value == "offline");
    let models = evaluation.join("models");
    let embedder =
        tokio::task::spawn_blocking(move || rag_pipeline::init_embedder(&models)).await??;
    let embedder = Arc::new(std::sync::Mutex::new(embedder));
    let model = Arc::clone(&embedder);
    let query = "What is the HP-42 operating pressure and relief valve opening pressure?";
    let vector =
        tokio::task::spawn_blocking(move || rag_pipeline::embed_query(&model, query)).await??;
    let db = persistence::VectorDb::init(&evaluation.join("vectors")).await?;
    let ids: Vec<_> = (0..12)
        .map(|index| format!("synthetic-manual-{index}"))
        .collect();
    let retrieved = db.hybrid_search(query, &vector, &ids, 3).await?;
    let is_hybrid = matches!(
        retrieved.retrieval_mode,
        persistence::lancedb::RetrievalMode::Hybrid
    );
    let mut citations = Vec::new();
    for hit in retrieved.results {
        let filename = format!("{}.txt", hit.document_id);
        let source = evaluation.join("documents").join(&filename);
        let retained = tokio::fs::read_to_string(&source).await?;
        if retained != hit.content || hit.page != 1 {
            return Err(
                "Retrieved synthetic evidence does not match its retained logical source page."
                    .into(),
            );
        }
        citations.push(Citation {
            document_id: hit.document_id,
            filename,
            storage_path: source.to_string_lossy().into_owned(),
            page: 1,
            chunk_content: hit.content,
            score: hit.score,
            is_hybrid,
        });
    }
    if citations.is_empty() {
        return Err("No synthetic evidence was retrieved.".into());
    }
    let prompt = rag_pipeline::build_rag_prompt(query, &citations);
    let client = hf_hub::HFClient::builder().cache_dir(&cache).build()?;
    let filename = "Qwen3-1.7B-Q4_K_M.gguf";
    let gguf = client
        .model("unsloth", "Qwen3-1.7B-GGUF")
        .download_file()
        .filename(filename)
        .local_files_only(offline)
        .send()
        .await?;
    let parent = gguf
        .parent()
        .ok_or("The model cache has no snapshot directory.")?;
    let start = Instant::now();
    let device = mistralrs::Device::new_metal(0)?;
    let model = mistralrs::GgufModelBuilder::new(
        parent.to_string_lossy().into_owned(),
        vec![filename.to_string()],
    )
    .with_device(device)
    .with_logging()
    .with_tok_model_id("Qwen/Qwen3-1.7B".to_string())
    .build()
    .await?;
    let initialization = start.elapsed().as_secs_f64();
    let first = complete(&model, &prompt).await?;
    let (concurrent_a, concurrent_b) =
        tokio::try_join!(complete(&model, &prompt), complete(&model, &prompt))?;
    let mut cancelled = model.stream_chat_request(request(&prompt)).await?;
    tokio::time::timeout(Duration::from_secs(60), cancelled.next())
        .await?
        .ok_or("The cancellation test received no response.")?;
    drop(cancelled);
    let after_cancel = complete(&model, &prompt).await?;
    let receipt = serde_json::json!({"scope":"Real pinned engine with synthetic retrieval evidence; not packaged WebView/provider acceptance","device":"Metal device 0 (CPU fallback disabled)","model":"unsloth/Qwen3-1.7B-GGUF","file":filename,"snapshot":parent.file_name().and_then(|name| name.to_str()),"offline_gguf_resolution":offline,"model_initialization_seconds":initialization,"retrieved_citations":citations,"first":first,"concurrent_a":concurrent_a,"concurrent_b":concurrent_b,"after_receiver_drop":after_cancel});
    let output = serde_json::to_string_pretty(&receipt)?;
    tokio::fs::write(
        cache.join(if offline {
            "offline-smoke.json"
        } else {
            "model-smoke.json"
        }),
        &output,
    )
    .await?;
    println!("{output}");
    Ok(())
}
