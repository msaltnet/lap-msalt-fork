use base64::Engine;
use reqwest::{Url, redirect::Policy};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    time::Duration,
};

const MAX_CAPTION_CHARS: usize = 1000;
const MAX_PROVIDER_RESPONSE_BYTES: usize = 1024 * 1024;

pub fn literal_search_condition(search_term: &str) -> Option<(String, Vec<String>)> {
    if search_term.is_empty() {
        return None;
    }

    let condition = "(a.name LIKE (? COLLATE NOCASE) ESCAPE '\\'
        OR a.comments LIKE (? COLLATE NOCASE) ESCAPE '\\'
        OR EXISTS (
            SELECT 1 FROM ai_captions ac
            WHERE ac.file_id = a.id AND ac.caption LIKE (? COLLATE NOCASE) ESCAPE '\\'
        ))"
    .to_string();
    let pattern = format!("%{}%", escape_like_literal(search_term));
    Some((condition, vec![pattern; 3]))
}

fn escape_like_literal(term: &str) -> String {
    let mut escaped = String::with_capacity(term.len());
    for character in term.chars() {
        if matches!(character, '\\' | '%' | '_') {
            escaped.push('\\');
        }
        escaped.push(character);
    }
    escaped
}

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

fn localhost_dns_targets() -> [SocketAddr; 2] {
    [
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0),
        SocketAddr::new(IpAddr::V6(Ipv6Addr::LOCALHOST), 0),
    ]
}

fn provider_client() -> Result<reqwest::Client, String> {
    let localhost_targets = localhost_dns_targets();
    reqwest::Client::builder()
        .no_proxy()
        .redirect(Policy::none())
        .resolve_to_addrs("localhost", &localhost_targets)
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

fn language_name(code: &str) -> &'static str {
    match code.trim().to_ascii_lowercase().as_str() {
        "de" => "German",
        "es" => "Spanish",
        "fr" => "French",
        "hu" => "Hungarian",
        "it" => "Italian",
        "ja" => "Japanese",
        "ko" => "Korean",
        "nl" => "Dutch",
        "pl" => "Polish",
        "pt" => "Portuguese",
        "ru" => "Russian",
        "uk" => "Ukrainian",
        "zh-cn" | "zh" => "Chinese (Simplified)",
        "zh-tw" => "Chinese (Traditional)",
        _ => "English",
    }
}

fn build_caption_request(model: &str, language: &str, image_url: &str) -> Value {
    let prompt = format!(
        "Describe only what is visibly present in this photo in exactly one concise sentence. \
         Do not speculate and do not use an introductory phrase. Respond in {language}."
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

async fn read_limited(mut response: reqwest::Response) -> Result<Vec<u8>, String> {
    if response
        .content_length()
        .is_some_and(|size| size > MAX_PROVIDER_RESPONSE_BYTES as u64)
    {
        return Err("Local AI provider response is too large".to_string());
    }

    let mut data = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|error| error.to_string())? {
        if data.len() + chunk.len() > MAX_PROVIDER_RESPONSE_BYTES {
            return Err("Local AI provider response is too large".to_string());
        }
        data.extend_from_slice(&chunk);
    }
    Ok(data)
}

fn parse_provider_caption(data: &[u8]) -> Result<String, String> {
    let value: Value = serde_json::from_slice(data)
        .map_err(|_| "Local AI provider returned invalid JSON".to_string())?;
    let content = value
        .pointer("/choices/0/message/content")
        .and_then(Value::as_str)
        .ok_or_else(|| "Local AI provider returned no caption".to_string())?;
    normalize_caption(content)
}

async fn request_caption(
    base: &str,
    model: &str,
    language: &str,
    image_url: &str,
) -> Result<String, String> {
    let response = provider_client()?
        .post(endpoint_url(base, "chat/completions")?)
        .json(&build_caption_request(model, language, image_url))
        .send()
        .await
        .map_err(|error| format!("Could not reach the local AI provider: {error}"))?;
    if response.status().is_redirection() {
        return Err("Local AI provider redirects are not allowed".to_string());
    }

    let status = response.status();
    let data = read_limited(response).await?;
    if !status.is_success() {
        let detail = String::from_utf8_lossy(&data)
            .chars()
            .take(300)
            .collect::<String>();
        return Err(format!("Local AI provider returned {status}: {detail}"));
    }
    parse_provider_caption(&data)
}

fn image_mime(data: &[u8]) -> Result<&'static str, String> {
    if data.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return Ok("image/jpeg");
    }
    if data.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]) {
        return Ok("image/png");
    }
    Err("Caption thumbnail is not JPEG or PNG".to_string())
}

pub async fn generate(
    file_id: i64,
    source_modified_at: Option<i64>,
    endpoint: &str,
    model: &str,
    requested_language: &str,
    image_data: &[u8],
) -> Result<AiCaption, String> {
    let model = model.trim();
    if model.is_empty() {
        return Err("AI caption model is required".to_string());
    }
    let language = match requested_language.trim() {
        "" => "en",
        value => value,
    };
    let mime = image_mime(image_data)?;
    let encoded = base64::engine::general_purpose::STANDARD.encode(image_data);
    let image_url = format!("data:{mime};base64,{encoded}");
    let caption = request_caption(endpoint, model, language_name(language), &image_url).await?;

    Ok(AiCaption {
        file_id,
        caption,
        requested_language: language.to_string(),
        model: model.to_string(),
        source_modified_at,
        generated_at: chrono::Utc::now().timestamp(),
    })
}

fn model_is_available(data: &[u8], model: &str) -> Result<bool, String> {
    let value: Value = serde_json::from_slice(data)
        .map_err(|_| "Local AI provider returned invalid model data".to_string())?;
    Ok(value["data"]
        .as_array()
        .is_some_and(|models| models.iter().any(|item| item["id"].as_str() == Some(model))))
}

pub async fn test_provider(endpoint: &str, model: &str) -> Result<(), String> {
    let model = model.trim();
    if model.is_empty() {
        return Err("AI caption model is required".to_string());
    }
    let response = provider_client()?
        .get(endpoint_url(endpoint, "models")?)
        .send()
        .await
        .map_err(|error| format!("Could not reach the local AI provider: {error}"))?;
    if response.status().is_redirection() {
        return Err("Local AI provider redirects are not allowed".to_string());
    }

    let status = response.status();
    let data = read_limited(response).await?;
    if !status.is_success() {
        return Err(format!("Local AI provider returned {status}"));
    }
    if !model_is_available(&data, model)? {
        return Err("Configured AI caption model was not found".to_string());
    }
    Ok(())
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
             CREATE TABLE afiles (
                 id INTEGER PRIMARY KEY,
                 name TEXT,
                 comments TEXT
             );
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
    fn localhost_dns_targets_are_pinned_to_loopback() {
        assert!(
            localhost_dns_targets()
                .iter()
                .all(|address| address.ip().is_loopback())
        );
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
    fn maps_locale_codes_to_language_names() {
        assert_eq!(language_name("ko"), "Korean");
        assert_eq!(language_name("ja"), "Japanese");
        assert_eq!(language_name("zh-CN"), "Chinese (Simplified)");
        assert_eq!(language_name("zh-TW"), "Chinese (Traditional)");
        assert_eq!(language_name("pt"), "Portuguese");
        assert_eq!(language_name(""), "English");
        assert_eq!(language_name("xx-unknown"), "English");
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

    #[test]
    fn literal_search_treats_wildcards_as_literal_text() {
        let conn = fixture();
        for (id, name) in [
            (9, "report_100%.txt"),
            (10, "report_100X.txt"),
            (11, "notes_100_.md"),
        ] {
            conn.execute(
                "INSERT INTO afiles(id, name) VALUES (?1, ?2)",
                rusqlite::params![id, name],
            )
            .unwrap();
        }

        let search = |term: &str| -> Vec<i64> {
            let (condition, values) = literal_search_condition(term).unwrap();
            let query = format!("SELECT a.id FROM afiles a WHERE {condition} ORDER BY a.id");
            conn.prepare(&query)
                .unwrap()
                .query_map(rusqlite::params_from_iter(values), |row| row.get(0))
                .unwrap()
                .map(|row| row.unwrap())
                .collect()
        };

        // "%" and "_" match literally instead of acting as wildcards.
        assert_eq!(search("100%"), vec![9]);
        assert_eq!(search("100_"), vec![11]);
    }

    #[test]
    fn literal_search_uses_a_correlated_caption_subquery() {
        let (condition, values) = literal_search_condition("window cat").unwrap();
        assert!(condition.contains("EXISTS"));
        assert!(condition.contains("FROM ai_captions"));
        assert!(!condition.contains("JOIN ai_captions"));
        assert_eq!(values, vec!["%window cat%"; 3]);

        let conn = fixture();
        upsert(&conn, &caption("A window cat watches the street.")).unwrap();
        let query = format!("SELECT a.id FROM afiles a WHERE {condition}");
        let file_id: i64 = conn
            .query_row(&query, rusqlite::params_from_iter(values), |row| row.get(0))
            .unwrap();
        assert_eq!(file_id, 7);
    }

    #[tokio::test]
    async fn calls_loopback_chat_completion_and_parses_caption() {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            let mut request = vec![0_u8; 16 * 1024];
            let read = stream.read(&mut request).await.unwrap();
            let text = String::from_utf8_lossy(&request[..read]);
            assert!(text.starts_with("POST /v1/chat/completions HTTP/1.1"));
            let body = r#"{"choices":[{"message":{"content":"A cat sits beside a window."}}]}"#;
            stream
                .write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
        });

        let result = request_caption(
            &format!("http://{address}/v1"),
            "vision-model",
            "en",
            "data:image/jpeg;base64,YWJj",
        )
        .await
        .unwrap();
        assert_eq!(result, "A cat sits beside a window.");
    }

    #[tokio::test]
    async fn refuses_provider_redirects() {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            let mut request = [0_u8; 4096];
            let _ = stream.read(&mut request).await.unwrap();
            stream
                .write_all(
                    b"HTTP/1.1 302 Found\r\nLocation: http://example.com/\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                )
                .await
                .unwrap();
        });

        assert!(
            request_caption(
                &format!("http://{address}/v1"),
                "vision-model",
                "en",
                "data:image/jpeg;base64,YWJj",
            )
            .await
            .unwrap_err()
            .contains("redirect")
        );
    }

    #[test]
    fn detects_supported_thumbnail_mime_types() {
        assert_eq!(image_mime(&[0xFF, 0xD8, 0xFF, 0xD9]).unwrap(), "image/jpeg");
        assert_eq!(
            image_mime(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]).unwrap(),
            "image/png"
        );
        assert!(image_mime(b"unknown").is_err());
    }

    #[test]
    fn finds_configured_model_in_openai_models_response() {
        let body = br#"{"data":[{"id":"gemma3:4b"},{"id":"qwen-vl"}]}"#;
        assert!(model_is_available(body, "gemma3:4b").unwrap());
        assert!(!model_is_available(body, "missing").unwrap());
    }
}
