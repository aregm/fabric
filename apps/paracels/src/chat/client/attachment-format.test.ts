import assert from 'node:assert/strict';
import { describe, it } from 'node:test';

import { formatByteSize, isImageMime, sanitizeFileName } from './attachment-format.ts';

describe('attachment names and sizes', () => {
  it('drops path pieces and characters that do not belong in a file name', () => {
    assert.equal(sanitizeFileName('../../plan.pdf'), 'plan.pdf');
    assert.equal(sanitizeFileName(''), 'attachment');
    assert.equal(sanitizeFileName('notes (1).txt'), 'notes (1).txt');
  });

  it('treats only image types as inline images', () => {
    assert.equal(isImageMime('image/jpeg'), true);
    assert.equal(isImageMime('IMAGE/PNG'), true);
    assert.equal(isImageMime('application/pdf'), false);
  });

  it('formats sizes in decimal units', () => {
    assert.equal(formatByteSize(471), '471 bytes');
    assert.equal(formatByteSize(471_000), '471 kilobytes');
    assert.equal(formatByteSize(10_000_000), '10 MB');
  });
});
