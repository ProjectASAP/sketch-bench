/**
 * Application configuration
 */
export const CONFIG = {
  // Data directory configuration
  DATA_DIR: 'data',
  MANIFEST_FILE: 'data/index.json',

  // Common JSON filenames to try as fallback
  COMMON_JSON_FILES: [
    '../cpp/output/kll_final.jsonl',
    '../rust/output/kll_lib.jsonl'
  ],

  // Table rendering configuration
  CHAR_WIDTH: 9,              // Estimated character width in pixels
  BASE_PADDING: 40,           // Base padding per column in pixels
  CARD_WIDTH_LIMIT: 480,      // Maximum width for a table card in pixels

  // UI configuration
  RESIZE_DEBOUNCE_MS: 120,    // Debounce time for window resize events

  // Status messages
  STATUS_MESSAGES: {
    WAITING: '<span class="pill"><span class="pill-dot"></span>Waiting for JSON…</span>',
    LOADING: (filename) => `<span class="pill"><span class="pill-dot"></span>Loading <strong>${filename}</strong>…</span>`,
    LOADED: (filename, rowCount) => `<span class="pill"><span class="pill-dot"></span>Loaded <strong>${filename}</strong> · <strong>${rowCount}</strong> rows</span>`,
    EXTRA_LOADED: (filename, rowCount) => `<span class="pill"><span class="pill-dot"></span>Loaded extra <strong>${filename}</strong> · <strong>${rowCount}</strong> rows</span>`,
    REMOVED: (filename) => `<span class="pill"><span class="pill-dot"></span>Removed <strong>${filename}</strong></span>`,
    ERROR: '<span class="pill" style="border-color:#7f1d1d;color:#fecaca;"><span class="pill-dot" style="background:#ef4444;box-shadow:0 0 0 4px rgba(248,113,113,0.15);"></span>Failed to parse JSON</span>',
    LOADING_MULTIPLE: (count) => `<span class="pill"><span class="pill-dot"></span>Loading ${count} file(s) from output directories…</span>`
  },

  // Empty state messages
  EMPTY_STATE: {
    NO_DATA: 'No data loaded yet. Choose a JSON file above to see your sketch benchmark results.',
    NO_HEADERS: 'No headers found in JSON.',
    PARSE_ERROR: (message) => `Error parsing JSON: ${message}`
  },

  // Chart configuration
  CHART: {
    TYPE: 'bar',
    HEIGHT: 400,                    // Chart height in pixels
    BACKGROUND_COLOR: 'rgba(59, 130, 246, 0.7)',  // Blue with transparency
    BORDER_COLOR: 'rgba(59, 130, 246, 1)',
    BORDER_WIDTH: 1,
    FONT_COLOR: '#e5e7eb',
    GRID_COLOR: 'rgba(255, 255, 255, 0.1)',
    Y_AXIS_LABEL: 'Time (microseconds)',
    Y_AXIS_LABEL_THROUGHPUT: 'Throughput (items/sec)',
    SKIP_COLUMNS: ['run'],          // Columns to skip in chart (non-numeric identifiers)
    ITEMS_COUNT: 1000000,           // Number of items processed (1 million)
  }
};
