use reqwest::{Url, redirect::Policy};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{net::IpAddr, time::Duration};

const MAX_CAPTION_CHARS: usize = 1000;
const MAX_PROVIDER_RESPONSE_BYTES: usize = 1024 * 1024;

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

fn validate_base_url(raw: &str) -> Result<Url, String> {
    let url = Url::parse(raw.trim()).map_err(|_| "Invalid AI caption endpoint".to_string())?;
    if url.scheme() != "http"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err("AI caption endpoint must be a plain local HTTP URL".to_string());
    }

    let local = match url.host_str() {
        Some(host) if host.eq_ignore_ascii_case("localhost") => true,
        Some(host) => host
            .trim_start_matches('[')
            .trim_end_matches(']')
            .parse::<IpAddr>()
            .is_ok_and(|address| address.is_loopback()),
        None => false,
    };
    if !local {
        return Err("AI caption endpoint must use localhost or a loopback IP".to_string());
    }
    Ok(url)
}

fn endpoint_url(base: &str, route: &str) -> Result<Url, String> {
    let mut url = validate_base_url(base)?;
    let path = format!(
        "{}/{}",
        url.path().trim_end_matches('/'),
        route.trim_start_matches('/')
    );
    url.set_path(&path);
    Ok(url)
}

fn provider_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .no_proxy()
        .redirect(Policy::none())
        .connect_timeout(Duration::from_secs(3))
        .timeout(Duration::from_secs(120))
        .build()
        .map_err(|error| format!("Failed to create local AI client: {error}"))
}

fn normalize_caption(raw: &str) -> Result<String, String> {
    let value = raw
        .trim()
        .trim_matches(|character| character == '"' || character == '\'')
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if value.is_empty() {
        return Err("The local model returned an empty caption".to_string());
    }
    if value.chars().count() > MAX_CAPTION_CHARS {
        return Err("The local model returned a caption that is too long".to_string());
    }
    Ok(value)
}

fn build_caption_request(model: &str, language: &str, image_url: &str) -> Value {
    let prompt = format!(
        "Describe only what is visibly present in this photo in exactly one concise sentence. \
         Do not speculate and do not use an introductory phrase. Respond in {language} when supported; otherwise respond in English."
    );
    json!({
        "model": model,
        "stream": false,
        "temperature": 0.1,
        "max_tokens": 160,
        "messages": [{
            "role": "user",
            "content": [
                { "type": "text", "text": prompt },
                { "type": "image_url", "image_url": { "url": image_url } }
            ]
        }]
    })
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

pub fn upsert(conn: &Connection, value: &AiCaption) -> Result<(), String> {
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

pub fn fetch(conn: &Connection, file_id: i64) -> Result<Option<AiCaption>, String> {
    conn.query_row(
        "SELECT file_id, caption, requested_language, model, source_modified_at, generated_at
         FROM ai_captions WHERE file_id = ?1",
        [file_id],
        from_row,
    )
    .optional()
    .map_err(|error| error.to_string())
}

pub fn delete(conn: &Connection, file_id: i64) -> Result<bool, String> {
    conn.execute("DELETE FROM ai_captions WHERE file_id = ?1", [file_id])
        .map(|changed| changed > 0)
        .map_err(|error| error.to_string())
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
        upsert(&conn, &caption("첫 캡션")).unwrap();
        assert_eq!(fetch(&conn, 7).unwrap(), Some(caption("첫 캡션")));

        upsert(&conn, &caption("교체된 캡션")).unwrap();
        assert_eq!(fetch(&conn, 7).unwrap().unwrap().caption, "교체된 캡션");

        delete(&conn, 7).unwrap();
        assert_eq!(fetch(&conn, 7).unwrap(), None);

        upsert(&conn, &caption("삭제될 캡션")).unwrap();
        conn.execute("DELETE FROM afiles WHERE id = 7", []).unwrap();
        assert_eq!(fetch(&conn, 7).unwrap(), None);
    }

    #[test]
    fn accepts_only_plain_http_loopback_endpoints() {
        for accepted in [
            "http://localhost:11434/v1",
            "http://127.0.0.1:1234/v1/",
            "http://[::1]:8080",
        ] {
            assert!(validate_base_url(accepted).is_ok(), "{accepted}");
        }
        for rejected in [
            "https://127.0.0.1:11434/v1",
            "http://192.168.1.20:11434/v1",
            "http://example.com/v1",
            "http://user:pass@localhost:11434/v1",
            "http://localhost:11434/v1?token=x",
            "http://localhost:11434/v1#fragment",
        ] {
            assert!(validate_base_url(rejected).is_err(), "{rejected}");
        }
    }

    #[test]
    fn preserves_versioned_base_path_when_joining_provider_routes() {
        assert_eq!(
            endpoint_url("http://127.0.0.1:11434/v1", "chat/completions")
                .unwrap()
                .as_str(),
            "http://127.0.0.1:11434/v1/chat/completions",
        );
        assert_eq!(
            endpoint_url("http://127.0.0.1:11434", "models")
                .unwrap()
                .as_str(),
            "http://127.0.0.1:11434/models",
        );
    }

    #[test]
    fn normalizes_one_sentence_and_rejects_invalid_output() {
        assert_eq!(
            normalize_caption("  \"A dog   runs on grass.\"\n").unwrap(),
            "A dog runs on grass."
        );
        assert!(normalize_caption("   ").is_err());
        assert!(normalize_caption(&"x".repeat(MAX_CAPTION_CHARS + 1)).is_err());
    }

    #[test]
    fn serializes_openai_vision_message() {
        let body = build_caption_request("gemma3:4b", "ko", "data:image/jpeg;base64,YWJj");
        assert_eq!(body["model"], "gemma3:4b");
        assert_eq!(body["stream"], false);
        assert_eq!(
            body["messages"][0]["content"][1]["image_url"]["url"],
            "data:image/jpeg;base64,YWJj"
        );
    }
}
