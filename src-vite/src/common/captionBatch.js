export const CAPTION_BATCH_CONFIRM_THRESHOLD = 1000;

export const eligibleCaptionFiles = files =>
  (Array.isArray(files) ? files : []).filter(file =>
    [1, 3].includes(Number(file?.file_type)),
  );

export async function confirmCaptionBatchStart(files, confirmLargeBatch) {
  const count = Array.isArray(files) ? files.length : 0;
  if (count <= CAPTION_BATCH_CONFIRM_THRESHOLD) return true;
  return Boolean(await confirmLargeBatch(count));
}

// A caption is stale when the source file changed after the caption was
// generated. Mirrors the outdated check shown in the file info panel.
export const isCaptionStale = (caption, file) =>
  caption?.sourceModifiedAt != null
  && Number(caption.sourceModifiedAt) !== Number(file?.modified_at || 0);

export async function runCaptionBatch({
  files,
  isCancelled,
  process,
  onProgress = () => {},
}) {
  const queue = Array.isArray(files) ? files : [];
  const state = {
    total: queue.length,
    current: 0,
    succeeded: 0,
    skipped: 0,
    failed: 0,
    cancelled: false,
  };

  onProgress({ ...state });
  for (const file of queue) {
    if (isCancelled()) {
      state.cancelled = true;
      break;
    }

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

  if (isCancelled() && state.current < state.total) {
    state.cancelled = true;
  }
  return state;
}
