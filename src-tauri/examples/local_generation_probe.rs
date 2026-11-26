//! Compare eager and paged Metal generation using a cached GGUF and synthetic text.
//! Usage: cargo run --locked --example local_generation_probe -- <snapshot-dir> <app|eager|paged> [repetitions]
//! No application registry, documents, or credentials are read or modified.

use std::{path::PathBuf, time::Duration};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let args: Vec<_> = std::env::args().collect();
    let directory = PathBuf::from(
        args.get(1)
            .ok_or("Pass a cached GGUF snapshot directory.")?,
    );
    let mode = args
        .get(2)
        .ok_or("Choose app, eager, or paged attention.")?;
    if !matches!(mode.as_str(), "app" | "eager" | "paged") {
        return Err("Choose app, eager, or paged attention.".into());
    }
    let repetitions = args
        .get(3)
        .map(|value| value.parse::<usize>())
        .transpose()?
        .unwrap_or(1);
    if !(1..=256).contains(&repetitions) {
        return Err("Synthetic context repetitions must be between 1 and 256.".into());
    }
    let filename = "Qwen3-1.7B-Q4_K_M.gguf";
    if !directory.join(filename).is_file() {
        return Err("The cached default GGUF is missing from that directory.".into());
    }
    let builder =
        mistralrs::GgufModelBuilder::new(directory.to_string_lossy(), vec![filename.to_string()])
            .with_device(mistralrs::Device::new_metal(0)?)
            .with_tok_model_id("Qwen/Qwen3-1.7B".to_string())
            .with_logging();
    let builder = match mode.as_str() {
        "app" => ragtag_app::configure_local_generation(builder),
        "eager" => builder.without_paged_attn(),
        _ => builder,
    };
    let model = builder.build().await?;
    let excerpt = "The synthetic hydraulic pump HP-42 operates at 120 bar. Its relief valve opens at 150 bar. Inspect its inlet filter before starting. This passage is test data, not a real maintenance instruction.\n";
    let prompt = format!(
        "[Source 1, Page 1]\n{}\nWhat is the relief valve opening pressure? Answer in one sentence with [1].",
        excerpt.repeat(repetitions)
    );
    let request = ragtag_app::local_generation_request(
        "Answer only from the supplied source. Do not output reasoning.".into(),
        prompt,
    )
    .set_sampler_temperature(0.0)
    .set_sampler_max_len(64);
    let started = std::time::Instant::now();
    let result = tokio::time::timeout(Duration::from_secs(30), async {
        let mut stream = model.stream_chat_request(request).await?;
        let mut answer = String::new();
        let mut first_content = None;
        while let Some(response) = stream.next().await {
            match response {
                mistralrs::Response::Chunk(chunk) => {
                    if let Some(choice) = chunk.choices.into_iter().next() {
                        if let Some(content) = choice.delta.content.filter(|text| !text.is_empty())
                        {
                            first_content.get_or_insert(started.elapsed());
                            answer.push_str(&content);
                        }
                        if choice.finish_reason.is_some() {
                            return Ok((answer, first_content));
                        }
                    }
                }
                mistralrs::Response::Done(_) => return Ok((answer, first_content)),
                mistralrs::Response::ModelError(error, _) => return Err(error.into()),
                mistralrs::Response::InternalError(error)
                | mistralrs::Response::ValidationError(error) => return Err(error),
                _ => {}
            }
        }
        Err::<_, Box<dyn std::error::Error + Send + Sync>>(
            "The stream ended without a completion marker.".into(),
        )
    })
    .await;
    match result {
        Ok(Ok((answer, Some(first_content)))) if !answer.trim().is_empty() => {
            println!(
                "mode={mode} repetitions={repetitions} first_content={first_content:?} elapsed={:?} answer={answer}",
                started.elapsed()
            );
            Ok(())
        }
        Ok(Ok(_)) => Err("Generation completed without visible answer text.".into()),
        Ok(Err(error)) => Err(error),
        Err(_) => {
            // An engine GPU worker may remain blocked; do not wait for its teardown.
            eprintln!("mode={mode}: generation exceeded the 30-second diagnostic deadline");
            std::process::exit(2);
        }
    }
}
