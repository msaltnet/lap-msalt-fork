export const eligibleCaptionFiles = files =>
  (Array.isArray(files) ? files : []).filter(file =>
    [1, 3].includes(Number(file?.file_type)),
  );

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
