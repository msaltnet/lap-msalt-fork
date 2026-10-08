import test from 'node:test';
import assert from 'node:assert/strict';

import {
  confirmCaptionBatchStart,
  eligibleCaptionFiles,
  isCaptionStale,
  runCaptionBatch,
} from '../src/common/captionBatch.js';

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

test('confirmCaptionBatchStart asks before a large batch and honours cancellation', async () => {
  let requestedCount = 0;
  const confirmed = await confirmCaptionBatchStart(
    Array.from({ length: 1001 }),
    async (count) => {
      requestedCount = count;
      return false;
    },
  );

  assert.equal(confirmed, false);
  assert.equal(requestedCount, 1001);
});

test('confirmCaptionBatchStart starts small batches without a prompt', async () => {
  let promptCalls = 0;
  const confirmed = await confirmCaptionBatchStart(
    Array.from({ length: 1000 }),
    async () => {
      promptCalls += 1;
      return false;
    },
  );

  assert.equal(confirmed, true);
  assert.equal(promptCalls, 0);
});

test('isCaptionStale detects captions made before the file changed', () => {
  assert.equal(
    isCaptionStale({ sourceModifiedAt: 100 }, { modified_at: 100 }),
    false,
  );
  assert.equal(
    isCaptionStale({ sourceModifiedAt: 100 }, { modified_at: 200 }),
    true,
  );
  assert.equal(isCaptionStale({ sourceModifiedAt: null }, { modified_at: 200 }), false);
  assert.equal(isCaptionStale(null, { modified_at: 200 }), false);
  assert.equal(isCaptionStale(undefined, { modified_at: 200 }), false);
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
