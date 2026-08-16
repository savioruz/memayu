//! End-to-end tests for the on-device Candle embedder.
//!
//! These download the multilingual model on first run (cached under
//! `memayu_config::model_dir()`), so they are ignored by default. Run explicitly with:
//!
//! ```sh
//! cargo test -p memayu-llm-client --test local_embedding -- --ignored
//! ```

use memayu_core::EmbedderProvider;
use memayu_llm_client::local_embedder::LocalEmbedder;

fn cosine(a: &[f32], b: &[f32]) -> f32 {
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let na: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let nb: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    dot / (na * nb)
}

/// Returns the indices (0-based) of the `k` documents with the highest cosine
/// similarity to `query_vec`, from most to least similar.
fn top_k(docs: &[Vec<f32>], query_vec: &[f32], k: usize) -> Vec<usize> {
    let mut scored: Vec<(usize, f32)> = docs
        .iter()
        .enumerate()
        .map(|(i, d)| (i, cosine(d, query_vec)))
        .collect();
    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
    scored.into_iter().take(k).map(|(i, _)| i).collect()
}

#[tokio::test]
#[ignore]
async fn local_embedder_produces_normalized_384_dim_vectors() {
    let embedder = LocalEmbedder::new(memayu_llm_client::local_embedder::DEFAULT_MODEL_ID);

    let a = embedder.embed("hello world").await.expect("embed A");
    let b = embedder.embed("greetings earth").await.expect("embed B");

    // paraphrase-multilingual-MiniLM-L12-v2 outputs 384-d.
    assert_eq!(a.len(), 384, "embedding dimension");
    assert_eq!(b.len(), 384);
    // L2-normalized → unit length.
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    assert!(
        (norm_a - 1.0).abs() < 1e-3,
        "expected unit norm, got {norm_a}"
    );
}

#[tokio::test]
#[ignore]
async fn local_embedder_retrieval_is_meaningful_across_en_and_id() {
    let embedder = LocalEmbedder::new(memayu_llm_client::local_embedder::DEFAULT_MODEL_ID);

    let q = embedder.embed("how do I reset my password").await.unwrap();
    let related = embedder
        .embed("steps to change your account password")
        .await
        .unwrap();
    let unrelated = embedder
        .embed("the cat sat on the windowsill")
        .await
        .unwrap();
    let id_related = embedder
        .embed("cara mengganti kata sandi akun saya")
        .await
        .unwrap();

    let s_related = cosine(&q, &related);
    let s_unrelated = cosine(&q, &unrelated);
    let s_id = cosine(&q, &id_related);

    assert!(
        s_related > s_unrelated,
        "related EN should beat unrelated (related={s_related:.3} unrelated={s_unrelated:.3})"
    );
    assert!(
        s_id > s_unrelated,
        "cross-lingual ID should beat unrelated (id={s_id:.3} unrelated={s_unrelated:.3})"
    );
    // Sanity: semantically related pairs are reasonably strong.
    assert!(
        s_related > 0.4,
        "related similarity too low: {s_related:.3}"
    );
    assert!(s_id > 0.3, "cross-lingual similarity too low: {s_id:.3}");
}

/// The issue's retrieval-quality gate: a fixed mixed corpus (10 Bahasa
/// Indonesia + 10 English-technical memories) queried with 5 mixed queries.
/// Acceptable: the expected relevant memory (in either language) lands in the
/// top-3 for at least 4 of 5 queries (≥80% recall@3). This mirrors the
/// Indonesia-first audience, where an English-only default would fail.
#[tokio::test]
#[ignore]
async fn local_embedder_retrieval_gate_mixed_id_en() {
    let embedder = LocalEmbedder::new(memayu_llm_client::local_embedder::DEFAULT_MODEL_ID);

    // 10 EN + 10 ID documents, paired by concept (doc[i] and doc[i+10] are the
    // same concept in English and Indonesian respectively).
    let docs = vec![
        // 0 EN
        "The database connection pool size is configured in the config file",
        "Rate limiting is applied per API key to prevent abuse",
        "Vector embeddings are stored in pgvector for similarity search",
        "The server listens on localhost port 18080 by default",
        "Session cookies expire after 24 hours for security",
        "Full text search uses libSQL FTS5 for keyword matching",
        "The embedder probes the model on startup to detect dimensions",
        "Backup snapshots are written to the data directory nightly",
        "The API returns a cursor for paginating memory results",
        "Logs are rotated daily and kept for thirty days",
        // 10 ID
        "Koneksi database diatur di dalam file konfigurasi",
        "Pembatasan kecepatan diterapkan per kunci API",
        "Vektor embedding disimpan di pgvector untuk pencarian kemiripan",
        "Server mendengarkan di localhost port 18080 secara default",
        "Cookie sesi kedaluwarsa setelah 24 jam demi keamanan",
        "Pencarian teks lengkap menggunakan libSQL FTS5",
        "Embedder memeriksa model saat startup untuk mendeteksi dimensi",
        "Cadangan snapshot ditulis ke direktori data setiap malam",
        "API mengembalikan kursor untuk membagi halaman hasil memori",
        "Log diputar setiap hari dan disimpan selama tiga puluh hari",
    ];
    // Each query's relevant set is the concept's {EN, ID} pair (indices as above).
    let queries: Vec<(&str, [usize; 2])> = vec![
        ("how is the database connection pool configured?", [0, 10]),
        (
            "where are vector embeddings stored for similarity search?",
            [2, 12],
        ),
        ("bagaimana cara mengatur ukuran koneksi database?", [0, 10]),
        (
            "di mana vektor embedding disimpan untuk pencarian kemiripan?",
            [2, 12],
        ),
        (
            "apa yang terjadi pada cookie sesi setelah kedaluwarsa?",
            [4, 14],
        ),
    ];

    let mut doc_vecs = Vec::with_capacity(docs.len());
    for d in &docs {
        doc_vecs.push(embedder.embed(d).await.expect("embed doc"));
    }

    let mut hits = 0;
    for (qi, (q, relevant)) in queries.iter().enumerate() {
        let qvec = embedder.embed(q).await.expect("embed query");
        let top = top_k(&doc_vecs, &qvec, 3);
        let hit = top.iter().any(|i| relevant.contains(i));
        if hit {
            hits += 1;
        }
        println!("query {qi} {q:?} -> top-3 {top:?} relevant {relevant:?} hit={hit}");
    }

    let recall = hits as f32 / queries.len() as f32;
    println!(
        "mixed EN/ID retrieval recall@3 = {hits}/{} ({recall:.0}%)",
        queries.len()
    );
    // Gate: ≥ 4/5 (80%) recall@3 on the mixed corpus.
    assert!(
        hits >= 4,
        "recall@3 too low on mixed EN/ID corpus: {hits}/{}",
        queries.len()
    );
}
