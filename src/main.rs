use std::path::PathBuf;

use clap::Parser;

#[derive(Debug, Parser)]
#[command(about = "Identify songs using the Shazam API")]
struct Args {
    /// Path to the audio file to identify
    file: PathBuf,

    /// `RapidAPI` key for the Shazam API (can also be set via `RAPIDAPI_KEY` env var)
    #[arg(long, env = "RAPIDAPI_KEY")]
    api_key: String,
}

#[tokio::main]
async fn main() -> miette::Result<()> {
    let args = Args::parse();

    if !args.file.exists() {
        return Err(miette::miette!("File not found: {}", args.file.display()));
    }
    if !args.file.is_file() {
        return Err(miette::miette!("Not a regular file: {}", args.file.display()));
    }

    let result = shazam_lib::identify_song(&args.file, &args.api_key, 4_000).await?;
    println!("{result}");
    Ok(())
}
