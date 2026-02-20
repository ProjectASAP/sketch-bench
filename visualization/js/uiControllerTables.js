/**
 * UI Controller for tables view (no charts)
 */

import { CONFIG } from './config.js';
import { addClass, removeClass, escapeHtml, debounce } from './utils.js';
import { loadJsonFromFile } from './dataLoader.js';
import { renderDatasets } from './tableRendererOnly.js';

/**
 * UI Controller class for tables
 */
export class UIController {
  constructor() {
    this.datasets = [];
    this.elements = {};
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
    this.elements.fileInput.addEventListener('change', (e) => this.handleFileInputChange(e));
    this.setupDropZone(this.elements.dropZone, this.elements.fileInput, false);
    this.setupDropZone(this.elements.addMoreZone, this.elements.addMoreInput, true);
    this.elements.addMoreInput.addEventListener('change', (e) => this.handleAddMoreInputChange(e));
    this.elements.clearBtn.addEventListener('click', () => this.clearAllDatasets());
    this.elements.tableContainer.addEventListener('click', (e) => this.handleRemoveDataset(e));
    window.addEventListener('resize', debounce(() => this.handleResize(), CONFIG.RESIZE_DEBOUNCE_MS));
  }

  setupDropZone(dropZone, fileInput, append) {
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

    dropZone.addEventListener('drop', async (e) => {
      const file = e.dataTransfer?.files?.[0];
      if (file) await this.loadFile(file, append);
    });

    dropZone.addEventListener('click', () => fileInput.click());
  }

  async handleFileInputChange(event) {
    const file = event.target.files?.[0];
    if (file) await this.loadFile(file, false);
  }

  async handleAddMoreInputChange(event) {
    const file = event.target.files?.[0];
    if (file) {
      await this.loadFile(file, true);
      this.elements.addMoreInput.value = '';
    }
  }

  handleResize() {
    if (this.datasets.length > 0) this.render();
  }

  handleRemoveDataset(event) {
    const btn = event.target.closest('[data-remove-idx]');
    if (!btn) return;
    const index = Number(btn.getAttribute('data-remove-idx'));
    if (Number.isNaN(index)) return;
    this.removeDataset(index);
  }

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

  clearAllDatasets() {
    this.datasets = [];
    this.elements.tableContainer.innerHTML = `<div class="empty-state">${CONFIG.EMPTY_STATE.NO_DATA}</div>`;
    this.setStatus(CONFIG.STATUS_MESSAGES.WAITING);
    removeClass(this.elements.dropZone, 'hidden');
    addClass(this.elements.addMoreSection, 'hidden');
    this.elements.fileInput.value = '';
    this.elements.addMoreInput.value = '';
  }

  render() {
    const html = renderDatasets(this.datasets);
    this.elements.tableContainer.innerHTML = html;
    if (this.datasets.length > 0) {
      removeClass(this.elements.addMoreSection, 'hidden');
    } else {
      addClass(this.elements.addMoreSection, 'hidden');
    }
  }

  setStatus(html) {
    this.elements.statusEl.innerHTML = html;
  }
}
