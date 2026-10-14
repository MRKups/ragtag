//! Synthetic retrieval baseline. Requires runtime embedding assets, never an LLM.
//! Usage: cargo run --locked -p rag_pipeline --example retrieval_eval -- <new-directory>

use arrow_array::{FixedSizeListArray, Int32Array, RecordBatch, StringArray};
use persistence::VectorDb;
use std::{path::PathBuf, sync::Arc, time::Instant};

const PASSAGES: &[&str] = &[
    "Hydraulic pump HP-42: operating pressure is 120 bar. Inspect the inlet filter before starting. The bypass relief valve opens at 150 bar.",
    "Electric motor EM-7: isolate the supply before maintenance. Torque the terminal screws to 2.5 Nm. The motor winding insulation test uses 500 volts DC.",
    "The programmable logic controller PLC-90 safety interlock stops the conveyor when the guard door opens. Reset the latch only after the door closes.",
    "Application programming interface authentication uses an Authorization bearer token. A missing or expired token produces HTTP status 401; rate limits produce status 429.",
    "Central processing unit temperature throttling begins at 95 degrees Celsius. The graphics processing unit fan curve is configured independently.",
    "Sensor calibration: zero the pressure transducer at atmospheric pressure, then apply a certified 100 bar reference. Record the span correction separately from offset.",
    "Database recovery: retain the SQLite write-ahead log after a crash. Vector indexes are derived data and can be rebuilt from source documents.",
    "Firmware flashing: select the correct board revision, verify the image checksum, and keep power connected until the bootloader reports completion.",
    "Pneumatic valve PV-11 uses a 24 volt DC solenoid. Air pressure must remain between 4 and 8 bar. Do not use hydraulic oil in the air circuit.",
    "The software development kit examples use JSON request bodies over HTTPS. Configure the endpoint base URL before creating a client.",
    "Mechanical coupling alignment: permissible radial offset is 0.05 mm. Tighten bolts diagonally in three passes and recheck the alignment after rotation.",
    "Battery storage: maintain a 40 percent state of charge for long-term storage. Keep the pack dry and disconnect the load before replacing a fuse.",
];

const QUERIES: &[(&str, usize)] = &[
    ("HP-42 relief valve opening pressure", 0),
    ("EM-7 terminal screw tightening torque", 1),
    ("PLC-90 guard door safety interlock reset", 2),
    ("API expired token HTTP status", 3),
    ("CPU temperature throttling threshold", 4),
    ("pressure transducer calibration reference", 5),
    ("SQLite recovery after crash write-ahead log", 6),
    ("firmware board revision image checksum", 7),
    ("PV-11 solenoid voltage air pressure", 8),
    ("SDK JSON HTTPS endpoint setup", 9),
    ("coupling permissible radial alignment offset", 10),
    ("battery long term storage state of charge", 11),
];

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let directory = PathBuf::from(std::env::args().nth(1).ok_or(
        "Pass a new, empty output directory; this benchmark does not use personal documents.",
    )?);
    if tokio::fs::try_exists(&directory).await? {
        return Err(
            "Output directory already exists; choose a new path to avoid modifying existing data."
                .into(),
        );
    }
    tokio::fs::create_dir_all(&directory).await?;
    let models = directory.join("models");
    let mut embedder =
        tokio::task::spawn_blocking(move || rag_pipeline::init_embedder(&models)).await??;
    let inputs: Vec<_> = PASSAGES
        .iter()
        .map(|text| format!("passage: {text}"))
        .collect();
    let embeddings = tokio::task::spawn_blocking(move || {
        let embeddings = embedder.embed(inputs, Some(32))?;
        Ok::<_, Box<dyn std::error::Error + Send + Sync>>((embedder, embeddings))
    })
    .await?
    .map_err(|error| error.to_string())?;
    let (embedder, vectors) = embeddings;
    if vectors.len() != PASSAGES.len() || vectors.iter().any(|vector| vector.len() != 768) {
        return Err(
            "The embedder did not produce exactly one 768-dimensional vector per passage.".into(),
        );
    }
    let embedder = Arc::new(std::sync::Mutex::new(embedder));
    let ids: Vec<_> = (0..PASSAGES.len())
        .map(|index| format!("synthetic-manual-{index}"))
        .collect();
    tokio::fs::create_dir_all(directory.join("documents")).await?;
    for (id, passage) in ids.iter().zip(PASSAGES) {
        tokio::fs::write(
            directory.join("documents").join(format!("{id}.txt")),
            passage,
        )
        .await?;
    }
    let db = VectorDb::init(&directory.join("vectors")).await?;
    let flat = vectors.into_iter().flatten().collect::<Vec<_>>();
    let vector = FixedSizeListArray::from_iter_primitive::<arrow_array::types::Float32Type, _, _>(
        flat.as_chunks::<768>()
            .0
            .iter()
            .map(|chunk| Some(chunk.iter().copied().map(Some))),
        768,
    );
    let batch = RecordBatch::try_new(
        VectorDb::schema(),
        vec![
            Arc::new(vector),
            Arc::new(StringArray::from(ids.clone())),
            Arc::new(Int32Array::from(vec![1; ids.len()])),
            Arc::new(Int32Array::from(vec![0; ids.len()])),
            Arc::new(StringArray::from(PASSAGES.to_vec())),
        ],
    )?;
    db.upsert_chunks([batch]).await?;
    let mut modes = vec![
        serde_json::json!({"mode":"dense","hit_at_1":0,"hit_at_3":0,"reciprocal_rank_sum":0.0,"latency_ms":[]}),
        serde_json::json!({"mode":"hybrid","hit_at_1":0,"hit_at_3":0,"reciprocal_rank_sum":0.0,"latency_ms":[]}),
        serde_json::json!({"mode":"hybrid_enhanced","hit_at_1":0,"hit_at_3":0,"reciprocal_rank_sum":0.0,"latency_ms":[]}),
    ];
    for (query, expected) in QUERIES {
        let model = Arc::clone(&embedder);
        let text = query.to_string();
        let vector =
            tokio::task::spawn_blocking(move || rag_pipeline::embed_query(&model, &text)).await??;
        let model = Arc::clone(&embedder);
        let enhanced = rag_pipeline::enhance_query(query);
        let enhanced_text = enhanced.clone();
        let enhanced_vector =
            tokio::task::spawn_blocking(move || rag_pipeline::embed_query(&model, &enhanced_text))
                .await??;
        for (index, metrics) in modes.iter_mut().enumerate() {
            let start = Instant::now();
            let result = match index {
                0 => db.vector_search(&vector, &ids, 3).await?,
                1 => db.hybrid_search(query, &vector, &ids, 3).await?,
                _ => {
                    db.hybrid_search(&enhanced, &enhanced_vector, &ids, 3)
                        .await?
                }
            };
            let rank = result
                .results
                .iter()
                .position(|hit| hit.document_id == ids[*expected]);
            if rank == Some(0) {
                metrics["hit_at_1"] = serde_json::json!(metrics["hit_at_1"].as_u64().unwrap() + 1);
            }
            if let Some(rank) = rank {
                metrics["hit_at_3"] = serde_json::json!(metrics["hit_at_3"].as_u64().unwrap() + 1);
                metrics["reciprocal_rank_sum"] = serde_json::json!(
                    metrics["reciprocal_rank_sum"].as_f64().unwrap() + 1.0 / (rank + 1) as f64
                );
            }
            metrics["latency_ms"]
                .as_array_mut()
                .unwrap()
                .push(serde_json::json!(start.elapsed().as_secs_f64() * 1000.0));
        }
    }
    let result = serde_json::json!({"corpus":"12 synthetic technical passages; not representative acceptance evidence","embedding":"EmbeddingGemma300M, 768 dimensions","queries":QUERIES.len(),"top_k":3,"modes":modes});
    let output = serde_json::to_string_pretty(&result)?;
    tokio::fs::write(directory.join("retrieval-results.json"), &output).await?;
    println!("{output}");
    Ok(())
}
