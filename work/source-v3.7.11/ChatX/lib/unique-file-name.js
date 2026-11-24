'use strict';

function uniqueDisplayName(originalName, existingNames = []) {
  const original = String(originalName || 'file');
  const dot = original.lastIndexOf('.');
  const hasExtension = dot > 0;
  const stem = hasExtension ? original.slice(0, dot) : original;
  const extension = hasExtension ? original.slice(dot) : '';
  const used = new Set([...existingNames].map(name => String(name || '').toLocaleLowerCase()));
  let candidate = original;
  let suffix = 2;
  while (used.has(candidate.toLocaleLowerCase())) candidate = `${stem} (${suffix++})${extension}`;
  return candidate;
}

module.exports = { uniqueDisplayName };
