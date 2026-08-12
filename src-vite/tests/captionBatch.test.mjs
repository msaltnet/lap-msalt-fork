import test from 'node:test';
import assert from 'node:assert/strict';

import { eligibleCaptionFiles, runCaptionBatch } from '../src/common/captionBatch.js';

test('eligibleCaptionFiles keeps images and RAW files only', () => {
  assert.deepEqual(
    eligibleCaptionFiles([
      { id: 1, file_type: 1 },
      { id: 2, file_type: 2 },
      { id: 3, file_type: 3 },
    ]).map(file => file.id),
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

  assert.deepEqual(result, {
    total: 3,
    current: 3,
    succeeded: 1,
    skipped: 1,
    failed: 1,
    cancelled: false,
  });
});

test('runCaptionBatch stops before the next file after cancellation', async () => {
  let cancelled = false;
  const processed = [];
  const result = await runCaptionBatch({
    files: [{ id: 1 }, { id: 2 }],
    isCancelled: () => cancelled,
    process: async (file) => {
      processed.push(file.id);
      cancelled = true;
      return { fileId: file.id };
    },
  });

  assert.deepEqual(processed, [1]);
  assert.equal(result.cancelled, true);
  assert.equal(result.current, 1);
});
