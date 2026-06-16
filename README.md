# shazam-cli

[![License: GPL v3](https://img.shields.io/badge/License-GPLv3-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-2024_edition-orange.svg)](https://www.rust-lang.org)

A command-line tool that identifies songs from local audio files using the [Shazam API](https://rapidapi.com/apidojo/api/shazam) via RapidAPI. Feed it any audio file, and it tells you the track name, artist, genre, duration, and a link to the Shazam page.

## Features

- **Broad format support** — reads any audio format FFmpeg can decode (MP3, FLAC, AAC, OGG, WAV, M4A, and more)
- **Flexible sample position** — sample from the middle by default, or specify a percentage (`33%`) or an exact timestamp (`1:30`) with `--sample-at`
- **Rich output** — reports title, artist, genre(s), explicit flag, song duration, match offset, and Shazam URL
- **Mismatch detection** — warns when the identified song's duration or match offset differs noticeably from your file
- **API key via environment** — set `RAPIDAPI_KEY` environment variable instead of putting secrets on the command line
- **Debug logging** — `--debug` prints FFmpeg internals, resampler state, and HTTP details

## Requirements

- **Rust** toolchain (stable, 2024 edition) — install via [rustup](https://rustup.rs)
- **FFmpeg** libraries (4.x or 5.x) — required at compile time and runtime
  - Debian/Ubuntu: `sudo apt install libavcodec-dev libavformat-dev libavutil-dev libswresample-dev`
  - Fedora: `sudo dnf install ffmpeg-devel`
  - macOS: `brew install ffmpeg`
  - Windows: install FFmpeg and set `FFMPEG_DIR` before building
- A **RapidAPI key** with access to the [Shazam API](https://rapidapi.com/apidojo/api/shazam)

## Installation

### Build from source

```sh
git clone https://github.com/alexschrod/shazam-cli.git
cd shazam-cli
cargo build --release
```

The binary is placed at `target/release/shazam-cli`. Copy it somewhere on your `PATH`:

```sh
cp target/release/shazam-cli ~/.local/bin/
```

## Usage

```
shazam-cli [OPTIONS] --api-key <API_KEY> <FILE>
```

### Arguments

| Argument | Description                        |
| -------- | ---------------------------------- |
| `<FILE>` | Path to the audio file to identify |

### Options

| Option              | Default      | Description                                                                                                                                         |
| ------------------- | ------------ | --------------------------------------------------------------------------------------------------------------------------------------------------- |
| `--api-key <KEY>`   | _(required)_ | RapidAPI key for the Shazam API. Can also be set via the `RAPIDAPI_KEY` environment variable.                                                       |
| `--sample-at <POS>` | `50%`        | Where in the file to draw the 4-second sample. Use a percentage to center there (e.g. `33%`) or an absolute timestamp to start there (e.g. `2:00`). |
| `--debug`           | off          | Enable verbose debug logging.                                                                                                                       |
| `-h`, `--help`      |              | Print help.                                                                                                                                         |

### Examples

Identify a song, sampling from the middle (default):

```sh
shazam-cli song.mp3 --api-key YOUR_KEY
```

Use an environment variable for the key:

```sh
export RAPIDAPI_KEY=YOUR_KEY
shazam-cli song.flac
```

Sample from one-third into the track:

```sh
shazam-cli song.mp3 --api-key YOUR_KEY --sample-at 33%
```

Sample starting at the 1-minute 30-second mark:

```sh
shazam-cli song.mp3 --api-key YOUR_KEY --sample-at 1:30
```

### Example output

```
Rolling In the Deep — Los Vasquez Sounds
Genres:   Pop, Music, Classical, Modern Era
Duration: 3:47
Matched:  1:52 into song
https://www.shazam.com/track/54116254/rolling-in-the-deep
```

When the identified song's duration or match position differs noticeably from the file you provided, the output includes both values for comparison:

```
Some Track — Some Artist
Duration: 3:30 (song) vs 7:02 (file)
Matched:  0:45 in song vs 1:00 in file
```

## Configuration

The only required configuration is a RapidAPI key. Set it via the environment to avoid passing it as a flag each time:

```sh
# ~/.bashrc or ~/.zshrc
export RAPIDAPI_KEY=your_key_here
```

Obtain a key by subscribing to the [Shazam API on RapidAPI](https://rapidapi.com/apidojo/api/shazam). The free tier provides enough requests for casual personal use.

## How it works

1. **Decode** — FFmpeg opens the input file and decodes the best available audio stream.
2. **Seek** — the decoder seeks to the position computed from `--sample-at`.
3. **Resample** — audio frames are resampled to 44 100 Hz, mono, signed 16-bit PCM (the format the Shazam API requires).
4. **Encode** — the raw PCM bytes are Base64-encoded.
5. **Identify** — the encoded sample is posted to the Shazam `songs/v3/detect` endpoint.
6. **Format** — the JSON response is parsed and printed in a human-readable form.

The sample window is always 4 seconds long. For percentage-based positions the window is centered at that point in the song; for absolute positions it starts there. Both are clamped so the window never extends past the end of the file.

## Project layout

```
shazam-cli/
├── src/
│   └── main.rs          # CLI argument parsing and entry point
└── shazam-lib/
    └── src/
        ├── lib.rs        # Core logic: audio extraction, API calls, error types
        └── response.rs   # Shazam API response deserialization and formatting
```

## Contributing

Bug reports and pull requests are welcome. Please:

- Run `cargo clippy --workspace --all-targets` and resolve all warnings before submitting.
- Ensure `cargo test --workspace` passes.
- Follow [Conventional Commits](https://www.conventionalcommits.org/) for commit messages.

## License

This project is licensed under the [GNU General Public License v3.0](LICENSE).
