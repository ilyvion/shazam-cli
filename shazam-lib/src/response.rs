use std::collections::HashMap;

use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub(crate) struct ShazamResponse {
    pub(crate) results: Results,
    #[serde(default)]
    pub(crate) resources: Resources,
}

impl ShazamResponse {
    #[must_use]
    pub(crate) fn format_display(&self, context: Option<&crate::ExtractionContext>) -> String {
        let Some(first_match) = self.results.matches.first() else {
            return "No match found.".to_owned();
        };
        let Some(song) = self.resources.shazam_songs.get(&first_match.id) else {
            return "No match found.".to_owned();
        };

        let attrs = &song.attributes;
        let mut lines = vec![format!("{} \u{2014} {}", attrs.title, attrs.artist)];

        let genre_names = collect_genre_names(&self.resources, song);
        if !genre_names.is_empty() {
            lines.push(format!("Genres:   {}", genre_names.join(", ")));
        }

        if attrs.explicit {
            lines.push("[Explicit]".to_owned());
        }

        if let Some(meta) = &song.meta {
            lines.push(format_duration_line(meta, context));
            lines.push(format_match_line(meta, context));
        }

        if let Some(url) = &attrs.web_url {
            let base = url.split_once('?').map_or(url.as_str(), |(b, _)| b);
            lines.push(base.to_owned());
        }

        lines.join("\n")
    }
}

fn collect_genre_names<'res>(resources: &'res Resources, song: &ShazamSong) -> Vec<&'res str> {
    song.relationships
        .as_ref()
        .and_then(|r| r.genres.as_ref())
        .map_or_else(Vec::new, |rel| {
            rel.data
                .iter()
                .filter_map(|item| {
                    resources
                        .genres
                        .get(&item.id)
                        .map(|g| g.attributes.name.as_str())
                })
                .collect()
        })
}

fn format_duration_line(
    meta: &SongMeta,
    context: Option<&crate::ExtractionContext>,
) -> String {
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "API durations are small positive values"
    )]
    let song_secs = meta.duration as u64;
    let s_mins = song_secs / 60;
    let s_secs = song_secs % 60;

    let suffix = context
        .and_then(|ctx| {
            let file_secs = ctx.file_duration_ms / 1_000;
            (song_secs.abs_diff(file_secs) > 5).then(|| {
                let f_mins = file_secs / 60;
                let f_secs = file_secs % 60;
                format!(" (song) vs {f_mins}:{f_secs:02} (file)")
            })
        })
        .unwrap_or_default();

    format!("Duration: {s_mins}:{s_secs:02}{suffix}")
}

fn format_match_line(
    meta: &SongMeta,
    context: Option<&crate::ExtractionContext>,
) -> String {
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "API match offsets are small positive values"
    )]
    let offset_secs = meta.match_offset as u64;
    let o_mins = offset_secs / 60;
    let o_secs = offset_secs % 60;

    let suffix = context
        .and_then(|ctx| {
            let start_secs = ctx.sample_start_ms / 1_000;
            (offset_secs.abs_diff(start_secs) > 5).then(|| {
                let f_mins = start_secs / 60;
                let f_secs = start_secs % 60;
                format!(" in song vs {f_mins}:{f_secs:02} in file")
            })
        })
        .unwrap_or_else(|| " into song".to_owned());

    format!("Matched:  {o_mins}:{o_secs:02}{suffix}")
}

#[derive(Debug, Deserialize)]
pub(crate) struct Results {
    pub(crate) matches: Vec<Match>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct Match {
    pub(crate) id: String,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct Resources {
    #[serde(rename = "shazam-songs", default)]
    pub(crate) shazam_songs: HashMap<String, ShazamSong>,
    #[serde(default)]
    pub(crate) genres: HashMap<String, GenreResource>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ShazamSong {
    pub(crate) attributes: ShazamSongAttributes,
    pub(crate) meta: Option<SongMeta>,
    pub(crate) relationships: Option<ShazamSongRelationships>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ShazamSongAttributes {
    pub(crate) title: String,
    pub(crate) artist: String,
    pub(crate) explicit: bool,
    pub(crate) web_url: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SongMeta {
    pub(crate) match_offset: f64,
    pub(crate) duration: f64,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ShazamSongRelationships {
    pub(crate) genres: Option<Relationship>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct Relationship {
    pub(crate) data: Vec<RelationshipItem>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct RelationshipItem {
    pub(crate) id: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct GenreResource {
    pub(crate) attributes: GenreAttributes,
}

#[derive(Debug, Deserialize)]
pub(crate) struct GenreAttributes {
    pub(crate) name: String,
}
