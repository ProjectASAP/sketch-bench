/**
 * UI Controller - Handles all UI interactions and events
 */

import { CONFIG } from './config.js';
import { addClass, removeClass, escapeHtml, debounce } from './utils.js';
import { loadJsonFromFile } from './dataLoader.js';
import { renderDatasets } from './tableRenderer.js';
import { createBarChart, destroyChart } from './chartRenderer.js';

/**
 * UI Controller class
 */
export class UIController {
  constructor() {
    this.datasets = [];
    this.elements = {};
    this.charts = []; // Store chart instances
  }

  /**
   * Initialize UI controller with DOM elements
   */
  init() {
    // Cache DOM elements
    this.elements = {
      fileInput: document.getElementById('csvInput'),
      statusEl: document.getElementById('status'),
      tableContainer: document.getElementById('tableContainer'),
      dropZone: document.getElementById('dropZone'),
      addMoreSection: document.getElementById('addMoreSection'),
      addMoreZone: document.getElementById('addMoreZone'),
      addMoreInput: document.getElementById('addMoreInput'),
      clearBtn: document.getElementById('clearBtn')
    };

    this.attachEventListeners();
  }

  /**
   * Attach all event listeners
   */
  attachEventListeners() {
    // File input change
    this.elements.fileInput.addEventListener('change', (e) => this.handleFileInputChange(e));

    // Main drop zone
    this.setupDropZone(this.elements.dropZone, this.elements.fileInput, false);

    // Add more drop zone
    this.setupDropZone(this.elements.addMoreZone, this.elements.addMoreInput, true);

    // Add more file input
    this.elements.addMoreInput.addEventListener('change', (e) => this.handleAddMoreInputChange(e));

    // Clear all button
    this.elements.clearBtn.addEventListener('click', () => this.clearAllDatasets());

    // Remove individual dataset
    this.elements.tableContainer.addEventListener('click', (e) => this.handleRemoveDataset(e));

    // Window resize
    window.addEventListener('resize', debounce(() => this.handleResize(), CONFIG.RESIZE_DEBOUNCE_MS));
  }

  /**
   * Setup drag and drop for a drop zone
   * @param {HTMLElement} dropZone - Drop zone element
   * @param {HTMLElement} fileInput - Associated file input element
   * @param {boolean} append - Whether to append or replace datasets
   */
  setupDropZone(dropZone, fileInput, append) {
    // Prevent default drag behaviors
    ['dragenter', 'dragover'].forEach(eventName => {
      dropZone.addEventListener(eventName, (e) => {
        e.preventDefault();
        e.stopPropagation();
        addClass(dropZone, 'dragover');
      });
    });

    ['dragleave', 'drop'].forEach(eventName => {
      dropZone.addEventListener(eventName, (e) => {
        e.preventDefault();
        e.stopPropagation();
        removeClass(dropZone, 'dragover');
      });
    });

    // Handle drop
    dropZone.addEventListener('drop', async (e) => {
      const file = e.dataTransfer?.files?.[0];
      if (file) {
        await this.loadFile(file, append);
      }
    });

    // Handle click to open file picker
    dropZone.addEventListener('click', () => fileInput.click());
  }

  /**
   * Handle file input change
   * @param {Event} event - Change event
   */
  async handleFileInputChange(event) {
    const file = event.target.files?.[0];
    if (file) {
      await this.loadFile(file, false);
    }
  }

  /**
   * Handle add more input change
   * @param {Event} event - Change event
   */
  async handleAddMoreInputChange(event) {
    const file = event.target.files?.[0];
    if (file) {
      await this.loadFile(file, true);
      this.elements.addMoreInput.value = '';
    }
  }

  /**
   * Handle window resize
   */
  handleResize() {
    if (this.datasets.length > 0) {
      this.render();
    }
  }

  /**
   * Handle remove dataset button click
   * @param {Event} event - Click event
   */
  handleRemoveDataset(event) {
    const btn = event.target.closest('[data-remove-idx]');
    if (!btn) return;

    const index = Number(btn.getAttribute('data-remove-idx'));
    if (Number.isNaN(index)) return;

    this.removeDataset(index);
  }

  /**
   * Load a JSON file
   * @param {File} file - File to load
   * @param {boolean} append - Whether to append or replace datasets
   */
  async loadFile(file, append = false) {
    this.setStatus(CONFIG.STATUS_MESSAGES.LOADING(file.name));

    try {
      const dataset = await loadJsonFromFile(file);

      if (append) {
        this.datasets.push(dataset);
        this.setStatus(CONFIG.STATUS_MESSAGES.EXTRA_LOADED(file.name, dataset.rows.length));
      } else {
        this.datasets = [dataset];
        this.setStatus(CONFIG.STATUS_MESSAGES.LOADED(file.name, dataset.rows.length));
        addClass(this.elements.dropZone, 'hidden');
      }

      this.render();
    } catch (error) {
      console.error(error);
      this.setStatus(CONFIG.STATUS_MESSAGES.ERROR);
      this.elements.tableContainer.innerHTML =
        `<div class="empty-state">${CONFIG.EMPTY_STATE.PARSE_ERROR(error.message)}</div>`;
    }
  }

  /**
   * Load datasets (typically from auto-load)
   * @param {Object[]} datasets - Array of datasets to load
   */
  loadDatasets(datasets) {
    if (datasets.length > 0) {
      this.datasets = datasets;
      this.setStatus(CONFIG.STATUS_MESSAGES.LOADED(
        `${datasets.length} file(s)`,
        datasets.reduce((sum, ds) => sum + ds.rows.length, 0)
      ));
      addClass(this.elements.dropZone, 'hidden');
      this.render();
    }
  }

  /**
   * Remove a dataset by index
   * @param {number} index - Index of dataset to remove
   */
  removeDataset(index) {
    const removed = this.datasets.splice(index, 1)[0];
    this.setStatus(CONFIG.STATUS_MESSAGES.REMOVED(escapeHtml(removed?.name || 'JSON')));

    if (this.datasets.length === 0) {
      removeClass(this.elements.dropZone, 'hidden');
      addClass(this.elements.addMoreSection, 'hidden');
      this.setStatus(CONFIG.STATUS_MESSAGES.WAITING);
    }

    this.render();
  }

  /**
   * Clear all datasets
   */
  clearAllDatasets() {
    this.destroyAllCharts();
    this.datasets = [];
    this.elements.tableContainer.innerHTML = `<div class="empty-state">${CONFIG.EMPTY_STATE.NO_DATA}</div>`;
    this.setStatus(CONFIG.STATUS_MESSAGES.WAITING);
    removeClass(this.elements.dropZone, 'hidden');
    addClass(this.elements.addMoreSection, 'hidden');
    this.elements.fileInput.value = '';
    this.elements.addMoreInput.value = '';
  }

  /**
   * Render all datasets
   */
  render() {
    // Destroy existing charts to prevent memory leaks
    this.destroyAllCharts();

    // Render HTML
    const html = renderDatasets(this.datasets);
    this.elements.tableContainer.innerHTML = html;

    // Create charts for each dataset
    this.createAllCharts();

    if (this.datasets.length > 0) {
      removeClass(this.elements.addMoreSection, 'hidden');
    } else {
      addClass(this.elements.addMoreSection, 'hidden');
    }
  }

  /**
   * Create charts for all datasets
   */
  createAllCharts() {
    this.charts = [];
    for (let i = 0; i < this.datasets.length; i++) {
      const canvasId = `chart-${i}`;
      const chart = createBarChart(canvasId, this.datasets[i]);
      if (chart) {
        this.charts.push(chart);
      }
    }
  }

  /**
   * Destroy all chart instances
   */
  destroyAllCharts() {
    for (const chart of this.charts) {
      destroyChart(chart);
    }
    this.charts = [];
  }

  /**
   * Set status message
   * @param {string} html - HTML content for status
   */
  setStatus(html) {
    this.elements.statusEl.innerHTML = html;
  }
}
