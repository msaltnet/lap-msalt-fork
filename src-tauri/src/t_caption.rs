use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiCaption {
    pub file_id: i64,
    pub caption: String,
    pub requested_language: String,
    pub model: String,
    pub source_modified_at: Option<i64>,
    pub generated_at: i64,
}

fn from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<AiCaption> {
    Ok(AiCaption {
        file_id: row.get(0)?,
        caption: row.get(1)?,
        requested_language: row.get(2)?,
        model: row.get(3)?,
        source_modified_at: row.get(4)?,
        generated_at: row.get(5)?,
    })
}

fn upsert_with_conn(conn: &Connection, value: &AiCaption) -> Result<(), String> {
    conn.execute(
        "INSERT INTO ai_captions (
             file_id, caption, requested_language, model, source_modified_at, generated_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(file_id) DO UPDATE SET
             caption = excluded.caption,
             requested_language = excluded.requested_language,
             model = excluded.model,
             source_modified_at = excluded.source_modified_at,
             generated_at = excluded.generated_at",
        params![
            value.file_id,
            value.caption,
            value.requested_language,
            value.model,
            value.source_modified_at,
            value.generated_at,
        ],
    )
    .map_err(|error| error.to_string())?;
    Ok(())
}

fn fetch_with_conn(conn: &Connection, file_id: i64) -> Result<Option<AiCaption>, String> {
    conn.query_row(
        "SELECT file_id, caption, requested_language, model, source_modified_at, generated_at
         FROM ai_captions WHERE file_id = ?1",
        [file_id],
        from_row,
    )
    .optional()
    .map_err(|error| error.to_string())
}

fn delete_with_conn(conn: &Connection, file_id: i64) -> Result<bool, String> {
    conn.execute("DELETE FROM ai_captions WHERE file_id = ?1", [file_id])
        .map(|changed| changed > 0)
        .map_err(|error| error.to_string())
}

pub fn fetch(file_id: i64) -> Result<Option<AiCaption>, String> {
    let conn = crate::t_sqlite::open_conn()?;
    fetch_with_conn(&conn, file_id)
}

pub fn delete(file_id: i64) -> Result<bool, String> {
    let conn = crate::t_sqlite::open_conn()?;
    delete_with_conn(&conn, file_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "PRAGMA foreign_keys = ON;
             CREATE TABLE afiles (id INTEGER PRIMARY KEY);
             CREATE TABLE ai_captions (
                 file_id INTEGER PRIMARY KEY,
                 caption TEXT NOT NULL,
                 requested_language TEXT NOT NULL,
                 model TEXT NOT NULL,
                 source_modified_at INTEGER,
                 generated_at INTEGER NOT NULL,
                 FOREIGN KEY (file_id) REFERENCES afiles(id) ON DELETE CASCADE
             );
             INSERT INTO afiles(id) VALUES (7);",
        )
        .unwrap();
        conn
    }

    fn caption(text: &str) -> AiCaption {
        AiCaption {
            file_id: 7,
            caption: text.to_string(),
            requested_language: "ko".to_string(),
            model: "gemma3:4b".to_string(),
            source_modified_at: Some(123),
            generated_at: 456,
        }
    }

    #[test]
    fn upsert_fetch_delete_and_file_cascade_are_atomic() {
        let conn = fixture();
        upsert_with_conn(&conn, &caption("첫 캡션")).unwrap();
        assert_eq!(fetch_with_conn(&conn, 7).unwrap(), Some(caption("첫 캡션")));

        upsert_with_conn(&conn, &caption("교체된 캡션")).unwrap();
        assert_eq!(
            fetch_with_conn(&conn, 7).unwrap().unwrap().caption,
            "교체된 캡션"
        );

        delete_with_conn(&conn, 7).unwrap();
        assert_eq!(fetch_with_conn(&conn, 7).unwrap(), None);

        upsert_with_conn(&conn, &caption("삭제될 캡션")).unwrap();
        conn.execute("DELETE FROM afiles WHERE id = 7", []).unwrap();
        assert_eq!(fetch_with_conn(&conn, 7).unwrap(), None);
    }
}
