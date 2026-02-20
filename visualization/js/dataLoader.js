/**
 * Data loading utilities for JSON/JSONL files
 */

import { parseJsonRecords } from './jsonParser.js';
import { CONFIG } from './config.js';

const NANOSECONDS_PER_MICROSECOND = 1000;

function formatMicroseconds(nsValue) {
  const micro = nsValue / NANOSECONDS_PER_MICROSECOND;
  if (!Number.isFinite(micro)) {
    return '';
  }
  const fixed = micro.toFixed(3);
  return fixed.replace(/\.?0+$/, '');
}

function resolveDataUrl(filename) {
  if (!filename) {
    return '';
  }
  if (filename.startsWith('/') || filename.startsWith('.') || filename.includes('/')) {
    return filename;
  }
  return `${CONFIG.DATA_DIR}/${filename}`;
}

function getDisplayName(filename) {
  if (!filename) {
    return '';
  }
  const parts = filename.split('/');
  return parts[parts.length - 1] || filename;
}

function buildDatasetFromRecords(records, name) {
  if (!Array.isArray(records) || records.length === 0) {
    throw new Error('JSON file has no records');
  }

  const implOrder = [];
  const implSet = new Set();
  const implCounts = new Map();
  const rows = [];

  records.forEach((record, index) => {
    if (!record || typeof record !== 'object') {
      throw new Error(`Record ${index + 1} is not an object`);
    }

    const impl =
      record.implementation_name ||
      record.implementation ||
      record.name;

    if (!impl) {
      throw new Error(`Record ${index + 1} missing implementation_name`);
    }

    const rawValue =
      record.total_nanoseconds ??
      record.totalNanoseconds ??
      record.total_ns ??
      record.totalNs;

    if (rawValue === undefined) {
      throw new Error(`Record ${index + 1} missing total_nanoseconds`);
    }

    const nsValue = Number(rawValue);
    if (!Number.isFinite(nsValue)) {
      throw new Error(`Record ${index + 1} has invalid total_nanoseconds`);
    }

    if (!implSet.has(impl)) {
      implSet.add(impl);
      implOrder.push(impl);
    }

    const runIndex = implCounts.get(impl) ?? 0;
    implCounts.set(impl, runIndex + 1);

    while (rows.length <= runIndex) {
      rows.push({ run: String(rows.length) });
    }

    rows[runIndex][impl] = formatMicroseconds(nsValue);
  });

  const headers = ['run', ...implOrder];

  return {
    headers,
    rows,
    name
  };
}

/**
 * Load a JSON/JSONL file from a URL
 * @param {string} url - URL to fetch JSON from
 * @param {string} filename - Display name for the file
 * @returns {Promise<Object|null>} Dataset object or null on failure
 */
export async function loadJsonFromUrl(url, filename) {
  try {
    const response = await fetch(url);
    if (!response.ok) {
      return null;
    }

    const text = await response.text();
    const records = parseJsonRecords(text);

    return buildDatasetFromRecords(records, filename);
  } catch (error) {
    console.error(`Failed to load ${url}:`, error);
    return null;
  }
}

/**
 * Load a JSON/JSONL file from a File object
 * @param {File} file - File object to load
 * @returns {Promise<Object>} Dataset object
 */
export function loadJsonFromFile(file) {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();

    reader.onload = (event) => {
      try {
        const text = event.target.result;
        const records = parseJsonRecords(text);
        resolve(buildDatasetFromRecords(records, file.name));
      } catch (error) {
        reject(error);
      }
    };

    reader.onerror = () => {
      reject(new Error('Failed to read file'));
    };

    reader.readAsText(file);
  });
}

/**
 * Auto-load JSON files from the data directory or output dirs
 * First tries to load from manifest file, then falls back to common filenames
 * @returns {Promise<Object[]>} Array of loaded datasets
 */
export async function autoLoadDataFiles() {
  try {
    // Try to fetch manifest file
    const manifestResponse = await fetch(CONFIG.MANIFEST_FILE);

    if (manifestResponse.ok) {
      const manifest = await manifestResponse.json();
      const jsonFiles = manifest.files || [];

      if (jsonFiles.length === 0) {
        console.log('No JSON files listed in manifest');
        return await tryLoadCommonFiles();
      }

      return await loadFilesInParallel(jsonFiles);
    }

    console.log(`Manifest not available (status ${manifestResponse.status}), falling back to common files`);
    // Fallback to common files when manifest fetch fails
    return await tryLoadCommonFiles();
  } catch (error) {
    console.log('Could not auto-load from manifest, trying common files:', error.message);
    return await tryLoadCommonFiles();
  }
}

/**
 * Try to load JSON files with common naming patterns
 * @returns {Promise<Object[]>} Array of loaded datasets
 */
async function tryLoadCommonFiles() {
  return await loadFilesInParallel(CONFIG.COMMON_JSON_FILES);
}

async function loadFilesInParallel(filenames) {
  if (!filenames || filenames.length === 0) {
    return [];
  }

  const loadPromises = filenames.map(filename => {
    const url = resolveDataUrl(filename);
    const displayName = getDisplayName(filename);
    return loadJsonFromUrl(url, displayName);
  });

  const results = await Promise.all(loadPromises);
  return results.filter(Boolean);
}
