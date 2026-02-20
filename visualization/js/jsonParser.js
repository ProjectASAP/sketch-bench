/**
 * JSON/JSONL parsing utilities
 */

/**
 * Parse JSON or JSONL text into records
 * @param {string} text - JSON or JSONL content
 * @returns {Object[]} Array of records
 * @throws {Error} If content is empty or invalid
 */
export function parseJsonRecords(text) {
  const trimmed = text.trim();
  if (!trimmed) {
    throw new Error('JSON file is empty');
  }

  if (trimmed.startsWith('[')) {
    const data = JSON.parse(trimmed);
    if (!Array.isArray(data)) {
      throw new Error('Expected JSON array at top level');
    }
    return data;
  }

  const lines = trimmed.split(/\r?\n/).filter(line => line.trim() !== '');
  if (lines.length === 1) {
    const record = JSON.parse(lines[0]);
    return Array.isArray(record) ? record : [record];
  }

  const records = [];
  for (let i = 0; i < lines.length; i++) {
    try {
      records.push(JSON.parse(lines[i]));
    } catch (error) {
      throw new Error(`Invalid JSON on line ${i + 1}: ${error.message}`);
    }
  }

  return records;
}
