use rp_core::{AudioFormat, RecordPlayerError, Result, TrackId, TrackMetadata};
use rusqlite::{Connection, OptionalExtension, params};
use std::path::{Path, PathBuf};

/// Encode 8 cue points as a comma-separated string (empty field = unset).
/// Returns `None` (stored as SQL NULL) when no cue is set.
fn encode_cues(cues: &[Option<i64>; 8]) -> Option<String> {
    if cues.iter().all(|c| c.is_none()) {
        return None;
    }
    Some(
        cues.iter()
            .map(|c| c.map(|v| v.to_string()).unwrap_or_default())
            .collect::<Vec<_>>()
            .join(","),
    )
}

/// Decode the comma-separated cue string back into 8 slots.
fn decode_cues(s: Option<String>) -> [Option<i64>; 8] {
    let mut out = [None; 8];
    if let Some(s) = s {
        for (i, field) in s.split(',').take(8).enumerate() {
            out[i] = field.trim().parse::<i64>().ok();
        }
    }
    out
}

/// SQLite-backed track database
pub struct TrackDatabase {
    conn: Connection,
}

impl TrackDatabase {
    /// Open or create a database at the given path
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)
            .map_err(|e| RecordPlayerError::Database(e.to_string()))?;

        let db = Self { conn };
        db.init_schema()?;
        Ok(db)
    }

    /// Open an in-memory database (for testing)
    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()
            .map_err(|e| RecordPlayerError::Database(e.to_string()))?;

        let db = Self { conn };
        db.init_schema()?;
        Ok(db)
    }

    /// Initialize the database schema
    fn init_schema(&self) -> Result<()> {
        self.conn
            .execute_batch(
                "
                CREATE TABLE IF NOT EXISTS tracks (
                    id INTEGER PRIMARY KEY,
                    path TEXT NOT NULL UNIQUE,
                    title TEXT,
                    artist TEXT,
                    album TEXT,
                    duration_samples INTEGER NOT NULL,
                    sample_rate INTEGER NOT NULL,
                    channels INTEGER NOT NULL,
                    bits_per_sample INTEGER NOT NULL,
                    bpm REAL,
                    first_beat INTEGER,
                    loop_start INTEGER,
                    loop_beats REAL,
                    cues TEXT,
                    waveform BLOB
                );
                CREATE INDEX IF NOT EXISTS idx_tracks_path ON tracks(path);
                CREATE INDEX IF NOT EXISTS idx_tracks_artist ON tracks(artist);
                ",
            )
            .map_err(|e| RecordPlayerError::Database(e.to_string()))?;

        // Migrate older databases. Each ALTER fails (and is ignored) when the
        // column is already present.
        for column in [
            "first_beat INTEGER",
            "loop_start INTEGER",
            "loop_beats REAL",
            "cues TEXT",
            "waveform BLOB",
        ] {
            let _ = self
                .conn
                .execute(&format!("ALTER TABLE tracks ADD COLUMN {column}"), []);
        }

        Ok(())
    }

    /// Insert or update a track, returning its id. Uses `RETURNING` so the
    /// correct id comes back on both the insert and the conflict-update path
    /// (`last_insert_rowid()` is wrong for the update case).
    pub fn upsert(&self, track: &TrackMetadata) -> Result<TrackId> {
        let id: i64 = self
            .conn
            .query_row(
                "INSERT INTO tracks (path, title, artist, album, duration_samples,
                                    sample_rate, channels, bits_per_sample, bpm, first_beat,
                                    loop_start, loop_beats, cues, waveform)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
                 ON CONFLICT(path) DO UPDATE SET
                    title = excluded.title,
                    artist = excluded.artist,
                    album = excluded.album,
                    duration_samples = excluded.duration_samples,
                    sample_rate = excluded.sample_rate,
                    channels = excluded.channels,
                    bits_per_sample = excluded.bits_per_sample,
                    bpm = COALESCE(excluded.bpm, tracks.bpm),
                    first_beat = COALESCE(excluded.first_beat, tracks.first_beat),
                    loop_start = COALESCE(excluded.loop_start, tracks.loop_start),
                    loop_beats = COALESCE(excluded.loop_beats, tracks.loop_beats),
                    cues = COALESCE(excluded.cues, tracks.cues),
                    waveform = COALESCE(excluded.waveform, tracks.waveform)
                 RETURNING id",
                params![
                    track.path.to_string_lossy(),
                    track.title,
                    track.artist,
                    track.album,
                    track.duration_samples as i64,
                    track.format.sample_rate,
                    track.format.channels,
                    track.format.bits_per_sample,
                    track.bpm,
                    track.first_beat,
                    track.loop_start,
                    track.loop_beats,
                    encode_cues(&track.cues),
                    &track.waveform,
                ],
                |row| row.get(0),
            )
            .map_err(|e| RecordPlayerError::Database(e.to_string()))?;

        Ok(TrackId(id as u64))
    }

    /// Update the stored tempo (BPM) for a track.
    pub fn set_bpm(&self, id: TrackId, bpm: f64) -> Result<()> {
        self.conn
            .execute(
                "UPDATE tracks SET bpm = ?1 WHERE id = ?2",
                params![bpm, id.0 as i64],
            )
            .map_err(|e| RecordPlayerError::Database(e.to_string()))?;
        Ok(())
    }

    /// Update the stored first-beat (downbeat) position for a track, in samples
    /// (may be negative).
    pub fn set_first_beat(&self, id: TrackId, first_beat: i64) -> Result<()> {
        self.conn
            .execute(
                "UPDATE tracks SET first_beat = ?1 WHERE id = ?2",
                params![first_beat, id.0 as i64],
            )
            .map_err(|e| RecordPlayerError::Database(e.to_string()))?;
        Ok(())
    }

    /// Save a per-track loop (grid-aligned start in samples + length in beats).
    pub fn set_loop(&self, id: TrackId, loop_start: i64, loop_beats: f64) -> Result<()> {
        self.conn
            .execute(
                "UPDATE tracks SET loop_start = ?1, loop_beats = ?2 WHERE id = ?3",
                params![loop_start, loop_beats, id.0 as i64],
            )
            .map_err(|e| RecordPlayerError::Database(e.to_string()))?;
        Ok(())
    }

    /// Clear a track's saved loop.
    pub fn clear_loop(&self, id: TrackId) -> Result<()> {
        self.conn
            .execute(
                "UPDATE tracks SET loop_start = NULL, loop_beats = NULL WHERE id = ?1",
                params![id.0 as i64],
            )
            .map_err(|e| RecordPlayerError::Database(e.to_string()))?;
        Ok(())
    }

    /// Save a track's cue points (hot cues 1-8, source samples; `None` = unset).
    pub fn set_cues(&self, id: TrackId, cues: &[Option<i64>; 8]) -> Result<()> {
        self.conn
            .execute(
                "UPDATE tracks SET cues = ?1 WHERE id = ?2",
                params![encode_cues(cues), id.0 as i64],
            )
            .map_err(|e| RecordPlayerError::Database(e.to_string()))?;
        Ok(())
    }

    /// Get a track by ID
    pub fn get(&self, id: TrackId) -> Result<Option<TrackMetadata>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, path, title, artist, album, duration_samples,
                        sample_rate, channels, bits_per_sample, bpm, first_beat,
                        loop_start, loop_beats, cues, waveform
                 FROM tracks WHERE id = ?1",
            )
            .map_err(|e| RecordPlayerError::Database(e.to_string()))?;

        let result = stmt
            .query_row(params![id.0 as i64], |row| {
                Ok(TrackMetadata {
                    id: TrackId(row.get::<_, i64>(0)? as u64),
                    path: PathBuf::from(row.get::<_, String>(1)?),
                    title: row.get(2)?,
                    artist: row.get(3)?,
                    album: row.get(4)?,
                    duration_samples: row.get::<_, i64>(5)? as u64,
                    format: AudioFormat {
                        sample_rate: row.get(6)?,
                        channels: row.get(7)?,
                        bits_per_sample: row.get(8)?,
                    },
                    bpm: row.get(9)?,
                    first_beat: row.get::<_, Option<i64>>(10)?,
                    loop_start: row.get::<_, Option<i64>>(11)?,
                    loop_beats: row.get::<_, Option<f64>>(12)?,
                    cues: decode_cues(row.get::<_, Option<String>>(13)?),
                    waveform: row.get::<_, Option<Vec<u8>>>(14)?,
                })
            })
            .optional()
            .map_err(|e| RecordPlayerError::Database(e.to_string()))?;

        Ok(result)
    }

    /// Search tracks by query
    pub fn search(&self, query: &str) -> Result<Vec<TrackMetadata>> {
        let pattern = format!("%{}%", query);
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, path, title, artist, album, duration_samples,
                        sample_rate, channels, bits_per_sample, bpm, first_beat,
                        loop_start, loop_beats, cues, waveform
                 FROM tracks
                 WHERE title LIKE ?1 OR artist LIKE ?1 OR album LIKE ?1
                 ORDER BY artist, album, title
                 LIMIT 100",
            )
            .map_err(|e| RecordPlayerError::Database(e.to_string()))?;

        let tracks = stmt
            .query_map(params![pattern], |row| {
                Ok(TrackMetadata {
                    id: TrackId(row.get::<_, i64>(0)? as u64),
                    path: PathBuf::from(row.get::<_, String>(1)?),
                    title: row.get(2)?,
                    artist: row.get(3)?,
                    album: row.get(4)?,
                    duration_samples: row.get::<_, i64>(5)? as u64,
                    format: AudioFormat {
                        sample_rate: row.get(6)?,
                        channels: row.get(7)?,
                        bits_per_sample: row.get(8)?,
                    },
                    bpm: row.get(9)?,
                    first_beat: row.get::<_, Option<i64>>(10)?,
                    loop_start: row.get::<_, Option<i64>>(11)?,
                    loop_beats: row.get::<_, Option<f64>>(12)?,
                    cues: decode_cues(row.get::<_, Option<String>>(13)?),
                    waveform: row.get::<_, Option<Vec<u8>>>(14)?,
                })
            })
            .map_err(|e| RecordPlayerError::Database(e.to_string()))?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| RecordPlayerError::Database(e.to_string()))?;

        Ok(tracks)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_track() -> TrackMetadata {
        TrackMetadata {
            id: TrackId(0),
            path: PathBuf::from("/music/song.flac"),
            title: Some("Song".into()),
            artist: Some("Artist".into()),
            album: None,
            duration_samples: 44_100,
            format: AudioFormat::default(),
            bpm: None,
            first_beat: None,
            loop_start: None,
            loop_beats: None,
            cues: [None; 8],
            waveform: None,
        }
    }

    fn temp_db_path() -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("rp_persist_{}_{}.db", std::process::id(), nanos))
    }

    #[test]
    fn bpm_and_grid_persist_across_reopen() {
        let path = temp_db_path();
        let _ = std::fs::remove_file(&path);

        // Edit immediately: set BPM and grid offset, then close the database.
        let id = {
            let db = TrackDatabase::open(&path).unwrap();
            let id = db.upsert(&sample_track()).unwrap();
            db.set_bpm(id, 128.0).unwrap();
            db.set_first_beat(id, -960).unwrap();
            db.set_loop(id, 22050, 8.0).unwrap();
            let mut cues = [None; 8];
            cues[0] = Some(0);
            cues[2] = Some(88200);
            db.set_cues(id, &cues).unwrap();
            id
        };

        // Reopen (simulating an app restart): values must still be there.
        {
            let db = TrackDatabase::open(&path).unwrap();
            let track = db.get(id).unwrap().expect("track should persist");
            assert_eq!(track.bpm, Some(128.0), "BPM did not persist");
            assert_eq!(track.first_beat, Some(-960), "grid offset did not persist");
            assert_eq!(track.loop_start, Some(22050), "loop start did not persist");
            assert_eq!(track.loop_beats, Some(8.0), "loop length did not persist");
            assert_eq!(track.cues[0], Some(0), "cue 1 did not persist");
            assert_eq!(track.cues[2], Some(88200), "cue 3 did not persist");
            assert_eq!(track.cues[1], None, "unset cue should stay unset");

            // Reloading the track (bpm/first_beat = None) must not erase saved
            // values, and must return the same id.
            let reloaded_id = db.upsert(&sample_track()).unwrap();
            assert_eq!(reloaded_id, id, "reload returned a different id");
            let track = db.get(id).unwrap().unwrap();
            assert_eq!(track.bpm, Some(128.0), "reload erased BPM");
            assert_eq!(track.first_beat, Some(-960), "reload erased grid offset");
        }

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn upsert_returns_correct_id_after_other_inserts() {
        // Regression: re-upserting an existing track must return ITS id, not the
        // id of whatever was inserted most recently on the connection.
        let path = temp_db_path();
        let _ = std::fs::remove_file(&path);
        let db = TrackDatabase::open(&path).unwrap();

        let mut a = sample_track();
        a.path = PathBuf::from("/music/a.flac");
        let id_a = db.upsert(&a).unwrap();

        let mut b = sample_track();
        b.path = PathBuf::from("/music/b.flac");
        let _id_b = db.upsert(&b).unwrap();

        // Re-upsert A after B was inserted; should still resolve to A's id.
        let id_a_again = db.upsert(&a).unwrap();
        assert_eq!(id_a_again, id_a);

        drop(db);
        let _ = std::fs::remove_file(&path);
    }
}
