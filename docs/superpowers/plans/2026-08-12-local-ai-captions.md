# Local AI Captions Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add opt-in, loopback-only AI caption generation for selected images, persist captions per library, and include them in unified search.

**Architecture:** A new Rust `t_caption` module validates local OpenAI-compatible providers, prepares bounded thumbnails, generates captions, and owns caption persistence helpers. Tauri commands expose single-file operations; Vue components manage settings, File Info, and sequential selection batches. Captions live in a version-14 one-to-one SQLite table and literal search uses a correlated `EXISTS` clause.

**Tech Stack:** Rust 2024, Tauri 2, rusqlite, reqwest, serde, Vue 3, Pinia, JavaScript modules, Node built-in test runner, SQLite.

---

## File Map

- Create `src-tauri/src/t_caption.rs`: provider security, HTTP protocol, caption normalization, image payload preparation, and caption repository.
- Modify `src-tauri/Cargo.toml`: enable Reqwest JSON request serialization.
- Modify `src-tauri/src/t_migration.rs`: schema migration 14 and migration regression test.
- Modify `src-tauri/src/t_sqlite.rs`: caption-aware literal search and a bounded thumbnail-byte helper.
- Modify `src-tauri/src/t_cmds.rs`: four Tauri caption commands.
- Modify `src-tauri/src/main.rs`: register the module and commands.
- Create `src-vite/src/common/captionBatch.js`: dependency-free sequential batch state machine.
- Create `src-vite/tests/captionBatch.test.mjs`: Node tests for eligibility, skip, failure, and cancellation.
- Modify `src-vite/src/common/api.js`: typed-by-convention caption command wrappers.
- Modify `src-vite/src/stores/configStore.js`: persisted AI-caption provider settings and panel state.
- Modify `src-vite/src/views/Settings.vue`: local provider form and connection test.
- Modify `src-vite/src/components/FileInfo.vue`: caption display and single-file actions.
- Create `src-vite/src/components/CaptionBatchDialog.vue`: selection progress and cancel UI.
- Modify `src-vite/src/common/fileMenu.ts`: caption action for eligible single and multi-selection menus.
- Modify `src-vite/src/components/Content.vue`: sequential batch orchestration and dialog state.
- Modify all files in `src-vite/src/locales/*.json`: localized labels and messages.
- Modify `README.md`: local runtime setup, privacy, storage, and search behavior.

### Task 1: Caption Schema and Repository

**Files:**
- Create: `src-tauri/src/t_caption.rs`
- Modify: `src-tauri/src/t_migration.rs:9-135`
- Modify: `src-tauri/src/main.rs:16-40`

- [ ] **Step 1: Write the failing migration and repository tests**

Create `src-tauri/src/t_caption.rs` with the response type and tests written against the desired repository API:

```rust
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
        ).unwrap();
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
        assert_eq!(fetch_with_conn(&conn, 7).unwrap().unwrap().caption, "교체된 캡션");

        delete_with_conn(&conn, 7).unwrap();
        assert_eq!(fetch_with_conn(&conn, 7).unwrap(), None);

        upsert_with_conn(&conn, &caption("삭제될 캡션")).unwrap();
        conn.execute("DELETE FROM afiles WHERE id = 7", []).unwrap();
        assert_eq!(fetch_with_conn(&conn, 7).unwrap(), None);
    }
}
```

Add a migration test at the bottom of `t_migration.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrates_version_13_to_ai_captions_version_14() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "PRAGMA foreign_keys = ON;
             PRAGMA user_version = 13;
             CREATE TABLE afiles (id INTEGER PRIMARY KEY);",
        ).unwrap();

        check_and_migrate(&conn).unwrap();

        let version: i32 = conn.query_row("PRAGMA user_version", [], |row| row.get(0)).unwrap();
        let table_count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'ai_captions'",
            [],
            |row| row.get(0),
        ).unwrap();
        assert_eq!(version, 14);
        assert_eq!(table_count, 1);
    }
}
```

- [ ] **Step 2: Run the focused Rust tests and verify RED**

Run:

```bash
cd src-tauri
cargo test t_caption::tests::upsert_fetch_delete_and_file_cascade_are_atomic
cargo test t_migration::tests::migrates_version_13_to_ai_captions_version_14
```

Expected: compilation fails because `upsert_with_conn`, `fetch_with_conn`, and `delete_with_conn` do not exist, and the migration test fails because version 14 is absent.

- [ ] **Step 3: Implement migration 14 and repository helpers**

Add this migration after version 13 in `get_migrations()`:

```rust
Migration {
    version: 14,
    description: "Create local AI caption storage",
    sql: "
        CREATE TABLE IF NOT EXISTS ai_captions (
            file_id INTEGER PRIMARY KEY,
            caption TEXT NOT NULL,
            requested_language TEXT NOT NULL,
            model TEXT NOT NULL,
            source_modified_at INTEGER,
            generated_at INTEGER NOT NULL,
            FOREIGN KEY (file_id) REFERENCES afiles(id) ON DELETE CASCADE
        );
    ",
},
```

Add repository helpers to `t_caption.rs`:

```rust
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

fn fetch_with_conn(conn: &Connection, file_id: i64) -> Result<Option<AiCaption>, String> {
    conn.query_row(
        "SELECT file_id, caption, requested_language, model, source_modified_at, generated_at
         FROM ai_captions WHERE file_id = ?1",
        [file_id],
        from_row,
    ).optional().map_err(|error| error.to_string())
}

fn upsert_with_conn(conn: &Connection, value: &AiCaption) -> Result<(), String> {
    conn.execute(
        "INSERT INTO ai_captions
         (file_id, caption, requested_language, model, source_modified_at, generated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
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
    ).map(|_| ()).map_err(|error| error.to_string())
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
```

Register `mod t_caption;` in `main.rs`.

- [ ] **Step 4: Run focused tests and verify GREEN**

Run the two commands from Step 2.

Expected: both tests pass.

- [ ] **Step 5: Commit the schema slice**

```bash
git add src-tauri/src/main.rs src-tauri/src/t_caption.rs src-tauri/src/t_migration.rs
git commit -m "feat(captions): add local caption storage"
```

### Task 2: Loopback Provider Security and Response Parsing

**Files:**
- Modify: `src-tauri/src/t_caption.rs`
- Modify: `src-tauri/Cargo.toml:60`

- [ ] **Step 1: Add failing URL, request, and normalization tests**

Extend the test module:

```rust
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
        endpoint_url("http://127.0.0.1:11434/v1", "chat/completions").unwrap().as_str(),
        "http://127.0.0.1:11434/v1/chat/completions",
    );
    assert_eq!(
        endpoint_url("http://127.0.0.1:11434", "models").unwrap().as_str(),
        "http://127.0.0.1:11434/models",
    );
}

#[test]
fn normalizes_one_sentence_and_rejects_invalid_output() {
    assert_eq!(normalize_caption("  \"A dog   runs on grass.\"\n").unwrap(), "A dog runs on grass.");
    assert!(normalize_caption("   ").is_err());
    assert!(normalize_caption(&"x".repeat(MAX_CAPTION_CHARS + 1)).is_err());
}

#[test]
fn serializes_openai_vision_message() {
    let body = build_caption_request("gemma3:4b", "ko", "data:image/jpeg;base64,YWJj");
    assert_eq!(body["model"], "gemma3:4b");
    assert_eq!(body["stream"], false);
    assert_eq!(body["messages"][0]["content"][1]["image_url"]["url"], "data:image/jpeg;base64,YWJj");
}
```

- [ ] **Step 2: Run the four tests and verify RED**

Run:

```bash
cd src-tauri
cargo test t_caption::tests::accepts_only_plain_http_loopback_endpoints
cargo test t_caption::tests::preserves_versioned_base_path_when_joining_provider_routes
cargo test t_caption::tests::normalizes_one_sentence_and_rejects_invalid_output
cargo test t_caption::tests::serializes_openai_vision_message
```

Expected: compilation fails for the missing URL, normalization, and request helpers.

- [ ] **Step 3: Enable Reqwest JSON support and implement the security boundary and protocol helpers**

Update the existing Reqwest dependency so `.json(...)` is available without enabling its other default features:

```toml
reqwest = { version = "0.12.28", default-features = false, features = ["rustls-tls", "json"] }
```

Then add constants and helpers:

```rust
use reqwest::{Url, redirect::Policy};
use serde_json::{Value, json};
use std::{net::IpAddr, time::Duration};

const MAX_CAPTION_CHARS: usize = 1000;
const MAX_PROVIDER_RESPONSE_BYTES: usize = 1024 * 1024;

fn validate_base_url(raw: &str) -> Result<Url, String> {
    let url = Url::parse(raw.trim()).map_err(|_| "Invalid AI caption endpoint".to_string())?;
    if url.scheme() != "http" || !url.username().is_empty() || url.password().is_some()
        || url.query().is_some() || url.fragment().is_some()
    {
        return Err("AI caption endpoint must be a plain local HTTP URL".to_string());
    }
    let local = match url.host_str() {
        Some(host) if host.eq_ignore_ascii_case("localhost") => true,
        Some(host) => host.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback()),
        None => false,
    };
    if !local {
        return Err("AI caption endpoint must use localhost or a loopback IP".to_string());
    }
    Ok(url)
}

fn endpoint_url(base: &str, route: &str) -> Result<Url, String> {
    let mut url = validate_base_url(base)?;
    let path = format!("{}/{}", url.path().trim_end_matches('/'), route.trim_start_matches('/'));
    url.set_path(&path);
    Ok(url)
}

fn provider_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .redirect(Policy::none())
        .connect_timeout(Duration::from_secs(3))
        .timeout(Duration::from_secs(120))
        .build()
        .map_err(|error| format!("Failed to create local AI client: {error}"))
}

fn normalize_caption(raw: &str) -> Result<String, String> {
    let value = raw
        .trim()
        .trim_matches(|character| character == '\"' || character == '\'')
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
```

- [ ] **Step 4: Verify GREEN and commit**

Run all `t_caption::tests` and commit:

```bash
cargo test t_caption::tests
git add src-tauri/Cargo.toml src-tauri/src/t_caption.rs
git commit -m "feat(captions): secure local provider requests"
```

### Task 3: Provider Calls and Bounded Image Preparation

**Files:**
- Modify: `src-tauri/src/t_caption.rs`
- Modify: `src-tauri/src/t_sqlite.rs:6738-6825`

- [ ] **Step 1: Add failing loopback HTTP and image-format tests**

Add async tests using `tokio::net::TcpListener` that return a fixed OpenAI-compatible response and a redirect response:

```rust
#[tokio::test]
async fn calls_loopback_chat_completion_and_parses_caption() {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let mut request = vec![0_u8; 16 * 1024];
        let read = stream.read(&mut request).await.unwrap();
        let text = String::from_utf8_lossy(&request[..read]);
        assert!(text.starts_with("POST /v1/chat/completions HTTP/1.1"));
        let body = r#"{"choices":[{"message":{"content":"A cat sits beside a window."}}]}"#;
        stream.write_all(format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(), body
        ).as_bytes()).await.unwrap();
    });

    let result = request_caption(
        &format!("http://{address}/v1"),
        "vision-model",
        "en",
        "data:image/jpeg;base64,YWJj",
    ).await.unwrap();
    assert_eq!(result, "A cat sits beside a window.");
}

#[tokio::test]
async fn refuses_provider_redirects() {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let mut request = [0_u8; 4096];
        let _ = stream.read(&mut request).await.unwrap();
        stream.write_all(
            b"HTTP/1.1 302 Found\r\nLocation: http://example.com/\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        ).await.unwrap();
    });
    assert!(request_caption(
        &format!("http://{address}/v1"), "vision-model", "en", "data:image/jpeg;base64,YWJj"
    ).await.unwrap_err().contains("redirect"));
}

#[test]
fn detects_supported_thumbnail_mime_types() {
    assert_eq!(image_mime(&[0xFF, 0xD8, 0xFF, 0xD9]).unwrap(), "image/jpeg");
    assert_eq!(image_mime(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]).unwrap(), "image/png");
    assert!(image_mime(b"unknown").is_err());
}
```

- [ ] **Step 2: Run tests and verify RED**

Run:

```bash
cd src-tauri
cargo test t_caption::tests::calls_loopback_chat_completion_and_parses_caption
cargo test t_caption::tests::refuses_provider_redirects
cargo test t_caption::tests::detects_supported_thumbnail_mime_types
```

Expected: missing `request_caption` and `image_mime` failures.

- [ ] **Step 3: Implement bounded provider response reading**

Implement `read_limited`, `parse_provider_caption`, and `request_caption`:

```rust
async fn read_limited(mut response: reqwest::Response) -> Result<Vec<u8>, String> {
    if response.content_length().is_some_and(|size| size > MAX_PROVIDER_RESPONSE_BYTES as u64) {
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
    let content = value.pointer("/choices/0/message/content").and_then(Value::as_str)
        .ok_or_else(|| "Local AI provider returned no caption".to_string())?;
    normalize_caption(content)
}

async fn request_caption(base: &str, model: &str, language: &str, image_url: &str) -> Result<String, String> {
    let response = provider_client()?
        .post(endpoint_url(base, "chat/completions")?)
        .json(&build_caption_request(model, language, image_url))
        .send().await
        .map_err(|error| format!("Could not reach the local AI provider: {error}"))?;
    if response.status().is_redirection() {
        return Err("Local AI provider redirects are not allowed".to_string());
    }
    let status = response.status();
    let data = read_limited(response).await?;
    if !status.is_success() {
        let detail = String::from_utf8_lossy(&data).chars().take(300).collect::<String>();
        return Err(format!("Local AI provider returned {status}: {detail}"));
    }
    parse_provider_caption(&data)
}

fn image_mime(data: &[u8]) -> Result<&'static str, String> {
    if data.starts_with(&[0xFF, 0xD8, 0xFF]) { return Ok("image/jpeg"); }
    if data.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]) { return Ok("image/png"); }
    Err("Caption thumbnail is not JPEG or PNG".to_string())
}
```

- [ ] **Step 4: Add a bounded thumbnail byte API and generation service**

In `AThumb`, add:

```rust
pub fn get_or_create_caption_bytes(file: &AFile) -> Result<Vec<u8>, String> {
    let file_id = file.id.ok_or_else(|| "File ID is missing".to_string())?;
    let file_path = file.file_path.as_deref().ok_or_else(|| "File path is missing".to_string())?;
    let file_type = file.file_type.unwrap_or(0);
    let orientation = file.e_orientation.unwrap_or(1) as i32;
    let thumb = Self::get_or_create_thumb(
        file_id, file_path, file_type, orientation, 512, false, file.duration.map(|v| v as u64), None,
    )?.ok_or_else(|| "Could not create a caption thumbnail".to_string())?;
    if thumb.error_code == 2 {
        return fs::read(file_path).map_err(|error| error.to_string());
    }
    thumb.thumb_data.ok_or_else(|| "Could not read the caption thumbnail".to_string())
}
```

In `t_caption.rs`, add:

```rust
pub async fn generate(
    file_id: i64,
    endpoint: &str,
    model: &str,
    requested_language: &str,
) -> Result<AiCaption, String> {
    if model.trim().is_empty() { return Err("AI caption model is required".to_string()); }
    let file = crate::t_sqlite::AFile::get_file_info(file_id)?
        .ok_or_else(|| "File not found".to_string())?;
    if !matches!(file.file_type, Some(1) | Some(3)) {
        return Err("AI captions support images and RAW files only".to_string());
    }
    let data = crate::t_sqlite::AThumb::get_or_create_caption_bytes(&file)?;
    let mime = image_mime(&data)?;
    let image_url = format!("data:{mime};base64,{}", base64::engine::general_purpose::STANDARD.encode(data));
    let caption = request_caption(endpoint, model.trim(), requested_language, &image_url).await?;
    let value = AiCaption {
        file_id,
        caption,
        requested_language: requested_language.to_string(),
        model: model.trim().to_string(),
        source_modified_at: file.modified_at,
        generated_at: chrono::Utc::now().timestamp(),
    };
    let conn = crate::t_sqlite::open_conn()?;
    upsert_with_conn(&conn, &value)?;
    Ok(value)
}
```

Import `base64::Engine`.

- [ ] **Step 5: Verify GREEN and commit**

```bash
cargo test t_caption::tests
git add src-tauri/src/t_caption.rs src-tauri/src/t_sqlite.rs
git commit -m "feat(captions): generate captions with local VLMs"
```

### Task 4: Tauri Commands and Frontend API

**Files:**
- Modify: `src-tauri/src/t_caption.rs`
- Modify: `src-tauri/src/t_cmds.rs:2460-2500`
- Modify: `src-tauri/src/main.rs:360-408`
- Modify: `src-vite/src/common/api.js:1670-1740`

- [ ] **Step 1: Add failing model-list parsing test**

Add:

```rust
#[test]
fn finds_configured_model_in_openai_models_response() {
    let body = br#"{"data":[{"id":"gemma3:4b"},{"id":"qwen-vl"}]}"#;
    assert!(model_is_available(body, "gemma3:4b").unwrap());
    assert!(!model_is_available(body, "missing").unwrap());
}
```

Run `cargo test t_caption::tests::finds_configured_model_in_openai_models_response` and confirm missing-helper failure.

- [ ] **Step 2: Implement connection test and Tauri commands**

Add `model_is_available` and `test_provider` to `t_caption.rs`:

```rust
fn model_is_available(data: &[u8], model: &str) -> Result<bool, String> {
    let value: Value = serde_json::from_slice(data)
        .map_err(|_| "Local AI provider returned invalid model data".to_string())?;
    Ok(value["data"].as_array().is_some_and(|models| {
        models.iter().any(|item| item["id"].as_str() == Some(model))
    }))
}

pub async fn test_provider(endpoint: &str, model: &str) -> Result<(), String> {
    if model.trim().is_empty() { return Err("AI caption model is required".to_string()); }
    let response = provider_client()?.get(endpoint_url(endpoint, "models")?).send().await
        .map_err(|error| format!("Could not reach the local AI provider: {error}"))?;
    if response.status().is_redirection() { return Err("Local AI provider redirects are not allowed".to_string()); }
    let status = response.status();
    let data = read_limited(response).await?;
    if !status.is_success() { return Err(format!("Local AI provider returned {status}")); }
    if !model_is_available(&data, model.trim())? { return Err("Configured AI caption model was not found".to_string()); }
    Ok(())
}
```

Add Tauri commands:

```rust
#[tauri::command]
pub async fn test_ai_caption_provider(endpoint: String, model: String) -> Result<(), String> {
    crate::t_caption::test_provider(&endpoint, &model).await
}

#[tauri::command]
pub fn get_ai_caption(file_id: i64) -> Result<Option<crate::t_caption::AiCaption>, String> {
    crate::t_caption::fetch(file_id)
}

#[tauri::command]
pub async fn generate_ai_caption(
    file_id: i64, endpoint: String, model: String, requested_language: String,
) -> Result<crate::t_caption::AiCaption, String> {
    crate::t_caption::generate(file_id, &endpoint, &model, &requested_language).await
}

#[tauri::command]
pub fn delete_ai_caption(file_id: i64) -> Result<bool, String> {
    crate::t_caption::delete(file_id)
}
```

Register all four commands in `main.rs`.

- [ ] **Step 3: Add frontend wrappers**

Add:

```javascript
export const testAiCaptionProvider = (endpoint, model) =>
  invoke('test_ai_caption_provider', { endpoint, model });

export const getAiCaption = (fileId) =>
  invoke('get_ai_caption', { fileId });

export const generateAiCaption = (fileId, settings, requestedLanguage) =>
  invoke('generate_ai_caption', {
    fileId,
    endpoint: settings.endpoint,
    model: settings.model,
    requestedLanguage,
  });

export const deleteAiCaption = (fileId) =>
  invoke('delete_ai_caption', { fileId });
```

- [ ] **Step 4: Verify and commit**

```bash
cd src-tauri
cargo test t_caption::tests
cargo check
cd ..
git add src-tauri/src/t_caption.rs src-tauri/src/t_cmds.rs src-tauri/src/main.rs src-vite/src/common/api.js
git commit -m "feat(captions): expose local caption commands"
```

### Task 5: Persisted Settings and Provider UI

**Files:**
- Modify: `src-vite/src/stores/configStore.js:41-50,150-163`
- Modify: `src-vite/src/views/Settings.vue:334-438,664-960,1230-1435`
- Modify: `src-vite/src/locales/*.json`

- [ ] **Step 1: Add config defaults**

Add panel state and provider settings:

```javascript
infoPanel: {
  // existing fields...
  showAiCaption: true,
},

settings: {
  // existing fields...
  aiCaption: {
    enabled: false,
    endpoint: 'http://127.0.0.1:11434/v1',
    model: '',
  },
},
```

- [ ] **Step 2: Add the Search settings card**

Below the image-search card, add a card with a toggle, endpoint input, model input, privacy hint, and connection-test button. Use existing Tailwind/daisyUI patterns and bind directly to `config.settings.aiCaption`. The button handler must snapshot trimmed values, call `testAiCaptionProvider`, show localized success/error toasts, and restore no state on failure because testing is read-only.

Core script:

```typescript
const isTestingCaptionProvider = ref(false);

async function testCaptionProviderConnection() {
  if (isTestingCaptionProvider.value) return;
  isTestingCaptionProvider.value = true;
  try {
    await testAiCaptionProvider(
      String(config.settings.aiCaption.endpoint || '').trim(),
      String(config.settings.aiCaption.model || '').trim(),
    );
    toast.success(t('settings.ai_caption.connection_success'));
  } catch (error: any) {
    toast.error(error?.message || String(error));
  } finally {
    isTestingCaptionProvider.value = false;
  }
}
```

On settings mount, repair older persisted state without overwriting valid user values:

```typescript
if (!config.settings.aiCaption || typeof config.settings.aiCaption !== 'object') {
  config.settings.aiCaption = {
    enabled: false,
    endpoint: 'http://127.0.0.1:11434/v1',
    model: '',
  };
}
```

- [ ] **Step 3: Add localized strings to every locale**

Under `settings`, add the same key structure in `de`, `en`, `es`, `fr`, `ja`, `ko`, `pt`, `ru`, and `zh`:

```json
"ai_caption": {
  "title": "AI captions",
  "enable": "Enable local AI captions",
  "enable_hint": "Generate captions with a vision model running on this computer.",
  "endpoint": "Local API endpoint",
  "model": "Vision model",
  "privacy_hint": "Only localhost and loopback addresses are accepted. Lap does not install or start the model.",
  "test_connection": "Test connection",
  "testing": "Testing...",
  "connection_success": "Connected to the local AI caption model."
}
```

Use accurate native translations, not copied English, for the eight non-English files.

- [ ] **Step 4: Build-check and commit**

```bash
pnpm --dir src-vite build
git add src-vite/src/stores/configStore.js src-vite/src/views/Settings.vue src-vite/src/locales
git commit -m "feat(captions): configure local vision provider"
```

### Task 6: File Info Caption UI

**Files:**
- Modify: `src-vite/src/components/FileInfo.vue:21-411,426-890`
- Modify: `src-vite/src/locales/*.json`

- [ ] **Step 1: Add caption state and request-race protection**

Import the caption APIs and create state:

```typescript
import { deleteAiCaption, generateAiCaption, getAiCaption } from '@/common/api';

const aiCaption = ref<any>(null);
const isCaptionLoading = ref(false);
const isCaptionGenerating = ref(false);
let captionRequestSeq = 0;
const canUseAiCaption = computed(() =>
  Boolean(config.settings.aiCaption?.enabled)
  && [1, 3].includes(Number(props.fileInfo?.file_type || 0))
);
const captionIsStale = computed(() =>
  aiCaption.value?.sourceModifiedAt != null
  && Number(aiCaption.value.sourceModifiedAt) !== Number(props.fileInfo?.modified_at || 0)
);

watch(() => [props.fileInfo?.id, canUseAiCaption.value] as const, async ([fileId, enabled]) => {
  const seq = ++captionRequestSeq;
  aiCaption.value = null;
  if (!enabled || !fileId) return;
  isCaptionLoading.value = true;
  try {
    const value = await getAiCaption(Number(fileId));
    if (seq === captionRequestSeq && Number(props.fileInfo?.id) === Number(fileId)) aiCaption.value = value;
  } finally {
    if (seq === captionRequestSeq) isCaptionLoading.value = false;
  }
}, { immediate: true });
```

- [ ] **Step 2: Add generate, regenerate, delete, and event handlers**

```typescript
async function createCaption() {
  if (!props.fileInfo?.id || isCaptionGenerating.value) return;
  isCaptionGenerating.value = true;
  try {
    aiCaption.value = await generateAiCaption(
      Number(props.fileInfo.id),
      { ...config.settings.aiCaption },
      String(locale.value || 'en'),
    );
    toast.success(t('file_info.ai_caption.generated'));
  } catch (error: any) {
    toast.error(error?.message || String(error));
  } finally {
    isCaptionGenerating.value = false;
  }
}

async function removeCaption() {
  if (!props.fileInfo?.id || isCaptionGenerating.value) return;
  await deleteAiCaption(Number(props.fileInfo.id));
  aiCaption.value = null;
  toast.success(t('file_info.ai_caption.deleted'));
}
```

Listen to `ai-caption-updated` with Tauri's event API and update only when the file ID matches. Dispose the listener in `onBeforeUnmount`.

- [ ] **Step 3: Add the collapsible panel markup**

Insert an AI Caption panel between metadata and map. It must:

- Use `IconSparkles` and existing `TButton` styles.
- Show a skeleton while loading.
- Show generated text with `whitespace-pre-wrap`.
- Show requested language, model, and `formatRelativeTime(generatedAt)` metadata.
- Show a warning badge when `captionIsStale`.
- Provide Generate, Regenerate, and Delete buttons with disabled states.
- Persist collapse state through `config.infoPanel.showAiCaption`.

- [ ] **Step 4: Add localized `file_info.ai_caption` strings to every locale**

Use this key shape:

```json
"ai_caption": {
  "title": "AI Caption",
  "generate": "Generate",
  "regenerate": "Regenerate",
  "delete": "Delete",
  "generating": "Generating...",
  "empty": "No AI caption has been generated.",
  "outdated": "The source file changed after this caption was generated.",
  "generated": "AI caption generated.",
  "deleted": "AI caption deleted.",
  "metadata": "{model} · requested {language} · {time}"
}
```

- [ ] **Step 5: Build-check and commit**

```bash
pnpm --dir src-vite build
git add src-vite/src/components/FileInfo.vue src-vite/src/locales
git commit -m "feat(captions): manage captions in file info"
```

### Task 7: Sequential Selection Batch

**Files:**
- Create: `src-vite/src/common/captionBatch.js`
- Create: `src-vite/tests/captionBatch.test.mjs`
- Create: `src-vite/src/components/CaptionBatchDialog.vue`
- Modify: `src-vite/src/common/fileMenu.ts:1-334`
- Modify: `src-vite/src/components/Content.vue:390-445,640-720,980-1040,3322-3409`
- Modify: `src-vite/src/locales/*.json`

- [ ] **Step 1: Write failing Node batch tests**

Create `src-vite/tests/captionBatch.test.mjs`:

```javascript
import test from 'node:test';
import assert from 'node:assert/strict';
import { eligibleCaptionFiles, runCaptionBatch } from '../src/common/captionBatch.js';

test('eligibleCaptionFiles keeps images and RAW files only', () => {
  assert.deepEqual(
    eligibleCaptionFiles([{ id: 1, file_type: 1 }, { id: 2, file_type: 2 }, { id: 3, file_type: 3 }]).map(x => x.id),
    [1, 3],
  );
});

test('runCaptionBatch counts saved, skipped, and failed files', async () => {
  const result = await runCaptionBatch({
    files: [{ id: 1 }, { id: 2 }, { id: 3 }],
    isCancelled: () => false,
    process: async (file) => {
      if (file.id === 2) return null;
      if (file.id === 3) throw new Error('failed');
      return { fileId: file.id, caption: 'ok' };
    },
  });
  assert.deepEqual(result, { total: 3, current: 3, succeeded: 1, skipped: 1, failed: 1, cancelled: false });
});

test('runCaptionBatch stops before the next file after cancellation', async () => {
  let cancelled = false;
  const processed = [];
  const result = await runCaptionBatch({
    files: [{ id: 1 }, { id: 2 }],
    isCancelled: () => cancelled,
    process: async (file) => { processed.push(file.id); cancelled = true; return { fileId: file.id }; },
  });
  assert.deepEqual(processed, [1]);
  assert.equal(result.cancelled, true);
  assert.equal(result.current, 1);
});
```

- [ ] **Step 2: Run Node tests and verify RED**

Run `node --test src-vite/tests/captionBatch.test.mjs`.

Expected: module-not-found failure for `captionBatch.js`.

- [ ] **Step 3: Implement the pure batch runner**

Create `captionBatch.js`:

```javascript
export const eligibleCaptionFiles = (files) =>
  (Array.isArray(files) ? files : []).filter(file => [1, 3].includes(Number(file?.file_type)));

export async function runCaptionBatch({ files, isCancelled, process, onProgress = () => {} }) {
  const state = { total: files.length, current: 0, succeeded: 0, skipped: 0, failed: 0, cancelled: false };
  onProgress({ ...state });
  for (const file of files) {
    if (isCancelled()) { state.cancelled = true; break; }
    try {
      const result = await process(file);
      if (result == null) state.skipped += 1;
      else state.succeeded += 1;
    } catch {
      state.failed += 1;
    }
    state.current += 1;
    onProgress({ ...state });
  }
  if (isCancelled() && state.current < state.total) state.cancelled = true;
  return state;
}
```

Run the Node tests and confirm all three pass.

- [ ] **Step 4: Build the progress dialog**

Create `CaptionBatchDialog.vue` using `ModalDialog`. Props are `progress` and `cancelling`; events are `cancel` and `close`. While running it shows a determinate progress bar, current/total and result counts, and a Cancel button. After completion it shows Close. Cancel changes the label to “Cancelling after current photo…” and disables repeated clicks.

- [ ] **Step 5: Add menu action and Content orchestration**

In `fileMenu.ts`, import `IconSparkles` and add `generate-ai-captions`:

- Selection menu: hidden unless `config.settings.aiCaption.enabled`; disabled unless selection contains an image. Extend `options` with `selectionHasImages?: Ref<boolean>` so mixed selections remain eligible.
- Single menu: hidden unless enabled; disabled unless `file_type` is 1 or 3.

In `Content.vue`, snapshot selected eligible files and settings, show the dialog, then use `runCaptionBatch`. Before generating, call `getAiCaption(file.id)` and return `null` for existing rows. For new rows call `generateAiCaption`, then emit:

```typescript
await tauriEmit('ai-caption-updated', { fileId: Number(file.id), caption });
```

Cancellation sets a local boolean read by `isCancelled`; it does not abort the in-flight command. On finish, retain the dialog until Close and show a localized aggregate toast.

- [ ] **Step 6: Add batch locale strings, build, and commit**

Add `caption_batch` strings under `msgbox` in all locales for title, counts, cancel, cancelling, close, completed, and cancelled.

Run:

```bash
node --test src-vite/tests/captionBatch.test.mjs
pnpm --dir src-vite build
git add src-vite/src/common/captionBatch.js src-vite/tests/captionBatch.test.mjs \
  src-vite/src/components/CaptionBatchDialog.vue src-vite/src/common/fileMenu.ts \
  src-vite/src/components/Content.vue src-vite/src/locales
git commit -m "feat(captions): generate captions for selections"
```

### Task 8: Caption Search, Documentation, and Full Verification

**Files:**
- Modify: `src-tauri/src/t_sqlite.rs:3995-4008`
- Modify: `README.md:53-94,196-209`

- [ ] **Step 1: Add a failing caption-search SQL test**

Inside the existing or new `t_sqlite.rs` test module, add:

```rust
#[test]
fn literal_search_includes_ai_captions_without_a_join() {
    let params = QueryParams {
        search_file_name: "window cat".to_string(),
        search_file_type: 0,
        sort_type: 0,
        sort_order: 0,
        search_all_subfolders: String::new(),
        search_folder: String::new(),
        start_date: 0,
        end_date: 0,
        calendar_sort: 0,
        folder_sort: 0,
        category_sort: 0,
        make: String::new(),
        model: String::new(),
        lens_make: String::new(),
        lens_model: String::new(),
        location_admin1: String::new(),
        location_name: String::new(),
        is_favorite: false,
        rating: -1,
        culling_flag: -1,
        tag_id: 0,
        person_id: 0,
        gps_min_lat: None,
        gps_max_lat: None,
        gps_min_lon: None,
        gps_max_lon: None,
        group_by: 0,
    };
    let (joins, where_clause, values) = AFile::build_search_query_parts(&params);
    assert!(!joins.contains("ai_captions"));
    assert!(where_clause.contains("EXISTS"));
    assert!(where_clause.contains("FROM ai_captions"));
    assert_eq!(values.len(), 3);
}
```

- [ ] **Step 2: Run the search test and verify RED**

Run `cargo test t_sqlite::tests::literal_search_includes_ai_captions_without_a_join`.

Expected: assertion fails because only file name and comments are searched and two values are bound.

- [ ] **Step 3: Add correlated caption search**

Replace the literal condition with:

```rust
conditions.push(
    "(a.name LIKE ? COLLATE NOCASE
       OR a.comments LIKE ? COLLATE NOCASE
       OR EXISTS (
           SELECT 1 FROM ai_captions ac
           WHERE ac.file_id = a.id AND ac.caption LIKE ? COLLATE NOCASE
       ))".to_string(),
);
let pattern = format!("%{}%", params.search_file_name);
sql_params.push(Box::new(pattern.clone()));
sql_params.push(Box::new(pattern.clone()));
sql_params.push(Box::new(pattern));
```

Run the focused test and all Rust tests.

- [ ] **Step 4: Document setup and privacy**

Update README feature and metadata sections to state:

- AI captions are optional and generated by a separately installed local OpenAI-compatible vision runtime.
- Endpoint validation accepts loopback addresses only.
- Captions remain in Lap's local database and are included in unified search.
- Captions are not embedded in originals or sidecars.
- Example Ollama setup:

```bash
ollama pull gemma3:4b
ollama serve
```

Then configure endpoint `http://127.0.0.1:11434/v1` and model `gemma3:4b` in Settings → Search → AI captions.

- [ ] **Step 5: Run formatting and complete verification**

Run from the worktree root:

```bash
cd src-tauri
cargo fmt --check
cargo test
cargo clippy --all-targets --all-features -- -D warnings
cd ..
node --test src-vite/tests/captionBatch.test.mjs
pnpm --dir src-vite build
git diff --check
git status --short
```

Expected: every command succeeds, no whitespace errors, and only intentional changes are present.

- [ ] **Step 6: Commit search and documentation**

```bash
git add src-tauri/src/t_sqlite.rs README.md
git commit -m "feat(captions): search generated descriptions"
```

- [ ] **Step 7: Perform manual provider checks**

With a local vision model running, verify:

1. Remote and LAN endpoints are rejected.
2. Test connection distinguishes unavailable server and missing model.
3. JPEG and RAW captions generate and persist after restart.
4. Regeneration failure preserves the previous caption.
5. Changing the source file produces the stale indicator.
6. Batch generation skips existing captions and cancels between files.
7. Caption text is returned by unified search and disappears after deletion.

- [ ] **Step 8: Request code review and prepare the PR**

Use `requesting-code-review`, fix verified findings with tests, then use `verification-before-completion`. Push `feature/ai-captions` to the fork and open a PR against `julyx10/lap:main` with:

- Problem and local-only solution.
- Privacy boundary and loopback validation.
- Database migration and search behavior.
- Automated command results.
- Manual test matrix, plus Settings and File Info screenshots when the desktop GUI can be launched.
- Explicit exclusions: bundled model, remote providers, videos, and automatic whole-library captioning.
