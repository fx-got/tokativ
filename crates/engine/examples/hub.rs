//! Searches Hugging Face the way the settings window does and lists the first result's files.
//!
//! `cargo run -p tokativ-engine --example hub -- whisper`, or `-- get owner/repo/file.gguf` to
//! download one.

use tokativ_engine::models;

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let query = args.next().unwrap_or_else(|| "parakeet".into());
    if query == "get" {
        let id = args
            .next()
            .and_then(|id| models::ModelId::parse(&id))
            .ok_or_else(|| anyhow::anyhow!("get needs owner/repo/file.gguf"))?;
        let cancel = std::sync::atomic::AtomicBool::new(false);
        models::download(&id, &|_, _| {}, &cancel)?;
        println!("installed {} at {}", id.name(), id.path().display());
        return Ok(());
    }
    let found = models::search(&query)?;
    for repository in &found {
        println!(
            "{:<55} {:<20} {:>9} downloads",
            repository.id,
            repository.architecture.as_deref().unwrap_or("?"),
            repository.downloads
        );
    }
    if let Some(first) = found.first() {
        println!("\nfiles in {}:", first.id);
        for file in models::repository_files(&first.id)? {
            println!("  {:<45} {:>6} MB", file.name, file.size / 1_000_000);
        }
    }
    Ok(())
}
