/**
 * Table rendering utilities
 */

import { CONFIG } from './config.js';
import { escapeHtml } from './utils.js';
import { renderChartContainer } from './chartRenderer.js';

/**
 * Group headers into multiple groups based on estimated width
 * Each group will be rendered as a separate table card
 * @param {string[]} headers - Array of header names
 * @returns {string[][]} Array of header groups
 */
function groupHeadersByWidth(headers) {
  const groups = [];
  let currentGroup = [];
  let currentWidth = 0;

  for (const header of headers) {
    const estimatedWidth = header.length * CONFIG.CHAR_WIDTH + CONFIG.BASE_PADDING;
    const safeWidth = Math.min(estimatedWidth, CONFIG.CARD_WIDTH_LIMIT);

    if (currentGroup.length > 0 && currentWidth + safeWidth > CONFIG.CARD_WIDTH_LIMIT) {
      // Start a new group
      groups.push(currentGroup);
      currentGroup = [header];
      currentWidth = safeWidth;
    } else {
      currentGroup.push(header);
      currentWidth += safeWidth;
    }
  }

  if (currentGroup.length > 0) {
    groups.push(currentGroup);
  }

  return groups;
}

/**
 * Render a single table card for a group of headers
 * @param {string[]} subHeaders - Headers for this table
 * @param {Object[]} rows - Data rows
 * @returns {string} HTML string for the table card
 */
function renderTableCard(subHeaders, rows) {
  let html = '<div class="table-card"><table><thead><tr>';

  // Render headers
  for (const header of subHeaders) {
    html += `<th>${escapeHtml(header)}</th>`;
  }

  html += '</tr></thead><tbody>';

  // Render rows
  for (const row of rows) {
    html += '<tr>';
    for (const header of subHeaders) {
      const value = row[header] ?? '';
      html += `<td>${escapeHtml(value)}</td>`;
    }
    html += '</tr>';
  }

  html += '</tbody></table></div>';
  return html;
}

/**
 * Render a dataset block with all its tables
 * @param {Object} dataset - Dataset object with headers, rows, and name
 * @param {number} index - Dataset index for removal button
 * @returns {string} HTML string for the dataset block
 */
function renderDatasetBlock(dataset, index) {
  const { headers, rows, name } = dataset;

  if (!headers || headers.length === 0) {
    return '';
  }

  // Split headers into groups for multiple tables
  const headerGroups = groupHeadersByWidth(headers);

  let html = `
    <div class="dataset-block">
      <div class="dataset-header">
        <div>
          <div class="dataset-title">${escapeHtml(name || 'JSON')}</div>
          <div class="dataset-meta">${rows.length} rows · ${headers.length} cols</div>
        </div>
        <div class="dataset-actions">
          <span class="pill"><span class="pill-dot"></span>Loaded</span>
          <button class="clear-btn dataset-remove" type="button" data-remove-idx="${index}" aria-label="Remove ${escapeHtml(name || 'JSON')}">✕</button>
        </div>
      </div>
      ${renderChartContainer(index)}
      <div class="table-wrapper">
        <div class="tables-grid">
  `;

  // Render each header group as a separate table card
  for (const subHeaders of headerGroups) {
    html += renderTableCard(subHeaders, rows);
  }

  html += '</div></div></div>';
  return html;
}

/**
 * Render all datasets
 * @param {Object[]} datasets - Array of dataset objects
 * @returns {string} HTML string for all datasets
 */
export function renderDatasets(datasets) {
  if (!datasets || datasets.length === 0) {
    return `<div class="empty-state">${CONFIG.EMPTY_STATE.NO_DATA}</div>`;
  }

  let html = '';
  for (let i = 0; i < datasets.length; i++) {
    html += renderDatasetBlock(datasets[i], i);
  }

  return html || `<div class="empty-state">${CONFIG.EMPTY_STATE.NO_HEADERS}</div>`;
}
