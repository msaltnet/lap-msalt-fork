# Local AI Captions Design

## Summary

Lap will let users generate a one-sentence description for selected images with a vision-language model running on the same computer. Captions remain local, are stored separately from comments and embedded metadata, and participate in the existing unified text search.

This is an opt-in feature. It does not bundle a generative model, start a model runtime, send media to a remote service, or automatically process an entire library. Users provide a loopback-only OpenAI-compatible endpoint and model name, then generate captions for one image or the current selection.

## Goals

- Generate a concise, factual caption for a normal image or RAW image.
- Request the current application language and allow the model to fall back to English.
- Store the generated caption and its provenance in the current library database.
- Display, regenerate, and delete a caption from File Info.
- Generate missing captions sequentially for the current selection with progress and cancellation between files.
- Include stored captions in unified literal text search without changing the existing CLIP semantic-search behavior.
- Preserve Lap's local-first privacy model and large-library performance characteristics.

## Non-goals

- Bundling or downloading a VLM in Lap.
- Connecting to remote or LAN hosts, even when the user supplies an API key.
- Automatically captioning a complete library during indexing.
- Generating captions for videos in the first version.
- Writing captions into EXIF, IPTC, XMP, or sidecar files.
- Editing generated captions manually in the first version.
- Adding caption fields to Smart Album rules or exposing full-text query syntax.

## User Experience

### Settings

The Search settings tab gains an **AI captions** card with:

- An enable toggle, disabled by default.
- An endpoint field with `http://127.0.0.1:11434/v1` as its initial value.
- A model field with no assumed default model.
- A **Test connection** action.
- A short privacy note stating that only loopback endpoints are accepted and that Lap does not install the model.

These values live in the existing persisted Pinia configuration under `settings.aiCaption`. They are passed to backend commands rather than copied into `app-config.json`. The Rust backend treats frontend values as untrusted and validates them for every request.

Testing the connection calls the provider's OpenAI-compatible models endpoint and verifies that the configured model identifier is present. This proves that the local server and selected model are available; vision capability is verified when the first caption is generated.

### Single-image captions

For image and RAW files, File Info gains a collapsible **AI Caption** section when the feature is enabled.

- Without a stored caption, it shows **Generate**.
- With a caption, it shows the text, requested language, model name, and generation time.
- If the file modification time differs from the stored source modification time, the caption is marked as potentially outdated.
- Existing captions can be regenerated or deleted.
- While generation is running, actions are disabled and an inline busy state is shown.

The section does not replace the existing EXIF Description or user Comment fields. A generation failure leaves an existing caption unchanged.

### Selection captions

The shared selection context menu gains **Generate AI captions** when AI captions are enabled and at least one selected item is an image or RAW file.

- Only image and RAW items are processed.
- Existing captions are skipped unless the single-image **Regenerate** action is used.
- Files are processed one at a time to avoid overloading a local model.
- Progress reports current, total, succeeded, skipped, and failed counts.
- Cancellation takes effect after the current provider request completes.
- A completion message summarizes the result. Individual failures do not stop the remaining selection.

The existing large-batch confirmation is used before starting a large selection. There is no all-library action.

## Architecture

### Backend module

A focused Rust module, `src-tauri/src/t_caption.rs`, owns:

- Provider endpoint validation and URL construction.
- HTTP client construction with redirects disabled and bounded timeouts.
- OpenAI-compatible request and response types.
- Thumbnail-to-data-URL preparation.
- Caption prompt construction and response normalization.
- Caption persistence operations exposed through a small service interface.

Tauri commands in `t_cmds.rs` provide four operations:

- Test a provider configuration.
- Get the stored caption for one file.
- Generate or regenerate the caption for one file.
- Delete the caption for one file.

Batch orchestration stays in the frontend so cancellation can occur between ordinary single-file commands and no new global backend job state is required.

### Provider protocol

The first version targets the shared OpenAI-compatible subset implemented by local runtimes such as Ollama and LM Studio.

Caption generation sends `POST {base_url}/chat/completions` with:

- The configured model identifier.
- A user message containing a text instruction and an `image_url` data URL.
- `stream: false`.
- A low temperature.
- A small output-token limit appropriate for one sentence.

The prompt instructs the model to:

- Describe only visible facts.
- Produce exactly one concise sentence.
- Avoid speculation and introductory phrases.
- Respond in the current application language when supported, otherwise in English.

The response parser accepts the standard `choices[0].message.content` string. It trims whitespace and wrapping quotes, collapses repeated whitespace, rejects empty output, and enforces a conservative character limit before persistence. Provider error bodies are reduced to safe, bounded messages for the UI.

### Local-only network policy

The backend accepts only plain HTTP URLs whose host is exactly `localhost` or a literal loopback IP address such as `127.0.0.1` or `::1`.

It rejects:

- Non-HTTP schemes.
- Credentials in the URL.
- Non-loopback hostnames and IP addresses.
- Query strings and fragments in the configured base URL.
- Redirects from the provider.

The base path may be empty or versioned, such as `/v1`. URL joining appends `models` or `chat/completions` without discarding that base path. No API-key setting is introduced because remote and authenticated providers are outside this feature's scope.

### Image preparation

Caption generation resolves the file through the current library database and accepts only file types `1` (image) and `3` (RAW).

It reuses Lap's thumbnail pipeline at a maximum dimension of 512 pixels:

1. Use a current cached thumbnail when available.
2. Generate a 512-pixel thumbnail through the existing format-specific pipeline when missing.
3. When the thumbnail pipeline intentionally uses an original because the image is already small, use that small source.
4. Reject the request if no decodable image representation is available.

The payload is encoded as a JPEG or PNG data URL according to the produced bytes. Full-size originals are not base64-encoded for ordinary generation, which bounds memory and request size and preserves RAW, HEIC, and other existing decoder support.

## Data Model

Schema migration version 14 creates a one-to-one table:

```sql
CREATE TABLE ai_captions (
    file_id INTEGER PRIMARY KEY,
    caption TEXT NOT NULL,
    requested_language TEXT NOT NULL,
    model TEXT NOT NULL,
    source_modified_at INTEGER,
    generated_at INTEGER NOT NULL,
    FOREIGN KEY (file_id) REFERENCES afiles(id) ON DELETE CASCADE
);
```

The table is separate from `afiles` because a caption has its own lifecycle and provenance, is optional for nearly every file, and should not widen the central file row. `file_id` provides the required search and lookup index. Deleting a file cascades to its caption.

`requested_language` records the locale requested from the provider; it is not presented as detected language. Regeneration uses an upsert and replaces all provenance fields atomically only after a valid provider response has been received.

An `AiCaption` response object uses the frontend's camel-case convention:

```text
fileId, caption, requestedLanguage, model, sourceModifiedAt, generatedAt
```

Caption data is fetched separately for File Info rather than added to every `AFile` row, avoiding extra data transfer across normal library browsing.

## Search Integration

The existing literal text condition changes from matching file name or comment to matching file name, comment, or a caption associated with the file. It uses a correlated `EXISTS` condition so the central file query keeps one row per file:

```sql
a.name LIKE ? COLLATE NOCASE
OR a.comments LIKE ? COLLATE NOCASE
OR EXISTS (
    SELECT 1
    FROM ai_captions c
    WHERE c.file_id = a.id
      AND c.caption LIKE ? COLLATE NOCASE
)
```

The current unified search still runs literal search and CLIP semantic search together, de-duplicates by file ID, and places literal results first. Generating or deleting a caption therefore affects the next search without rebuilding an index. SQLite FTS is intentionally deferred: user-triggered captions will initially be sparse, and the current `%LIKE%` search already scans file names and comments.

## State and Events

Single-image generation returns the saved `AiCaption`, allowing File Info to update without refetching the complete file record. Delete returns success and clears local caption state.

The Content component owns selection-batch state and calls the same generate command sequentially. After each item it updates progress and emits a lightweight caption-updated event containing the file ID and optional caption. File Info listens for that event so it remains consistent when the visible file is part of a batch.

Provider settings are snapshotted when a batch begins. Changing Settings does not alter an in-progress batch.

## Error Handling

Backend errors use stable categories with user-oriented messages:

- Invalid or non-local endpoint.
- Local provider unavailable or timed out.
- Configured model unavailable.
- Model rejected the image or lacks vision support.
- Provider returned an invalid or empty response.
- File is unsupported, missing, or cannot produce a thumbnail.
- Database read or write failure.

HTTP connection and total-request timeouts prevent a stopped provider from hanging the UI indefinitely. Responses and surfaced provider error text have size limits. A database upsert happens only after successful image preparation, provider completion, and caption validation. Regeneration therefore preserves the previous caption on any failure.

Batch generation counts a per-file error, continues, and reports the aggregate result. Cancellation never aborts a database transaction or leaves a partial row; it only prevents the next file from starting.

## Testing

Rust unit tests cover:

- Acceptance of `localhost`, IPv4 loopback, and IPv6 loopback base URLs.
- Rejection of remote hosts, credentials, unsupported schemes, query strings, fragments, and redirect attempts.
- Correct path joining for base URLs with and without `/v1`.
- Request serialization for text and image content.
- Parsing, normalization, empty-output rejection, and length bounds.
- Provider success and failure behavior against a loopback test server.
- Caption insert, fetch, atomic regeneration, deletion, and file-delete cascade using a temporary SQLite database.
- Migration from schema version 13 to 14.
- Literal search matches caption text without duplicating file rows.

Frontend batch bookkeeping is extracted into a small dependency-free JavaScript module and tested with Node's built-in test runner for eligible file filtering, skip/success/failure counts, and cancellation between items. TypeScript and Vue compilation validates the component integrations.

Final verification runs:

```text
cargo fmt --check
cargo test
cargo clippy --all-targets --all-features -- -D warnings
node --test src-vite/tests/captionBatch.test.mjs
pnpm --dir src-vite build
```

Manual validation covers Ollama or LM Studio connection testing, English and non-English generation, regeneration failure preservation, stale indication after a file change, RAW generation, batch skip/cancel behavior, and unified caption search.

## Documentation and Pull Request

README documentation will explain that AI captions require a separately installed local OpenAI-compatible vision runtime, remain in Lap's database, and are not written to original media. It will include one concise Ollama-compatible setup example while keeping the protocol vendor-neutral.

The pull request will remain focused on local captions, include screenshots of Settings and File Info when a runnable GUI environment is available, describe the privacy boundary, list automated and manual verification, and note that automatic library-wide processing and bundled models are intentionally excluded.
