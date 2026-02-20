/**
 * CSV parsing utilities
 */

/**
 * Split a CSV line into fields, handling quoted values
 * @param {string} line - CSV line to split
 * @returns {string[]} Array of field values
 */
function splitCSVLine(line) {
  const result = [];
  let current = '';
  let inQuotes = false;

  for (let i = 0; i < line.length; i++) {
    const char = line[i];
    if (char === '"') {
      inQuotes = !inQuotes;
    } else if (char === ',' && !inQuotes) {
      result.push(current);
      current = '';
    } else {
      current += char;
    }
  }
  result.push(current);

  // Remove surrounding quotes from each field
  return result.map(value => value.replace(/^"(.*)"$/, '$1'));
}

/**
 * Parse CSV text into headers and rows
 * @param {string} text - CSV text content
 * @returns {Object} Object with headers array and rows array
 * @throws {Error} If CSV is empty or invalid
 */
export function parseCSV(text) {
  const lines = text
    .split(/\r?\n/)
    .filter(line => line.trim().length > 0);

  if (lines.length === 0) {
    throw new Error('CSV file is empty');
  }

  const headers = splitCSVLine(lines[0]);
  const rows = [];

  for (let i = 1; i < lines.length; i++) {
    const line = lines[i];
    const values = splitCSVLine(line);

    // Skip completely empty rows
    if (values.every(v => v.trim() === '')) {
      continue;
    }

    // Create row object mapping headers to values
    const row = {};
    for (let j = 0; j < headers.length; j++) {
      row[headers[j]] = values[j] !== undefined ? values[j].trim() : '';
    }
    rows.push(row);
  }

  return { headers, rows };
}
