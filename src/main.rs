use std::path::PathBuf;

use clap::Parser;

#[tokio::main]
async fn main() -> miette::Result<()> {
    let args = Args::parse();

    if args.debug {
        tracing_subscriber::fmt()
            .with_max_level(tracing::Level::DEBUG)
            .init();
    }

    if !args.file.exists() {
        return Err(miette::miette!("File not found: {}", args.file.display()));
    }
    if !args.file.is_file() {
        return Err(miette::miette!(
            "Not a regular file: {}",
            args.file.display()
        ));
    }

    tracing::debug!(path = %args.file.display(), "identifying song");

    let result = shazam_lib::identify_song(&args.file, &args.api_key, 4_000, args.sample_at).await?;
    println!("{result}");
    Ok(())
}

#[derive(Debug, Parser)]
#[command(about = "Identify songs using the Shazam API")]
struct Args {
    /// Path to the audio file to identify
    file: PathBuf,

    /// `RapidAPI` key for the Shazam API (can also be set via `RAPIDAPI_KEY` env var)
    #[arg(long, env = "RAPIDAPI_KEY")]
    api_key: String,

    /// Enable debug logging
    #[arg(long)]
    debug: bool,

    /// Where in the song to sample from.
    /// Use a percentage to center the window there (e.g. `33%`),
    /// or an absolute time to start there (e.g. `2:00`).
    /// Defaults to the middle of the song.
    #[arg(long, default_value = "50%")]
    sample_at: shazam_lib::SampleAt,
}
