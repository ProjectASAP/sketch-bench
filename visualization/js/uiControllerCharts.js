/**
 * UI Controller for charts view (D3.js visualizations only)
 */

import { CONFIG } from './config.js';
import { addClass, removeClass, escapeHtml, debounce } from './utils.js';
import { loadJsonFromFile } from './dataLoader.js';
import { createBarChart, destroyChart, renderChartContainer, getNumericColumns, renderColumnSelector, createMergedBarChart, getDatasetStats, categorizeColumn } from './chartRendererD3.js';

/**
 * UI Controller class for charts
 */
export class UIController {
  constructor() {
    this.datasets = [];
    this.elements = {};
    this.charts = [];
    this.selectedColumns = []; // Array of Sets, one per dataset
    this.mergedSelectedColumns = new Set(); // Set of selected columns for merged view
    this.currentView = 'merged'; // 'merged' or 'individual'
    this.displayModes = []; // Array of display modes ('time' or 'throughput'), one per dataset
    this.mergedDisplayMode = 'time'; // Display mode for merged view
    this.selectedCategory = 'all'; // Selected category for merged view: 'all', 'Frequency', 'Cardinality', 'Quantile', 'G-sum', 'Other'
  }

  /**
   * Initialize UI controller with DOM elements
   */
  init() {
    // Cache DOM elements - using chartContainer instead of tableContainer
    this.elements = {
      fileInput: document.getElementById('csvInput'),
      statusEl: document.getElementById('status'),
      chartContainer: document.getElementById('chartContainer'),
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
    window.addEventListener('resize', debounce(() => this.handleResize(), CONFIG.RESIZE_DEBOUNCE_MS));

    // View switching listeners
    const viewOptions = document.querySelectorAll('.view-option');
    viewOptions.forEach(option => {
      option.addEventListener('click', () => {
        const view = option.getAttribute('data-view');
        this.switchView(view);
      });
    });

    // Column selector event listeners (using event delegation)
    this.elements.chartContainer.addEventListener('change', (e) => {
      if (e.target.classList.contains('column-checkbox')) {
        this.handleColumnToggle(e.target);
      }
      if (e.target.classList.contains('merged-column-checkbox')) {
        this.handleMergedColumnToggle(e.target);
      }
    });

    this.elements.chartContainer.addEventListener('click', (e) => {
      // Use closest() to handle clicks on child elements
      const selectorBtn = e.target.closest('.selector-btn');
      if (selectorBtn) {
        this.handleSelectorAction(selectorBtn);
      }

      const mergedSelectorBtn = e.target.closest('.merged-selector-btn');
      if (mergedSelectorBtn) {
        this.handleMergedSelectorAction(mergedSelectorBtn);
      }

      const modeToggleBtn = e.target.closest('.mode-toggle-btn');
      if (modeToggleBtn) {
        this.handleModeToggle(modeToggleBtn);
      }

      const mergedModeToggleBtn = e.target.closest('.merged-mode-toggle-btn');
      if (mergedModeToggleBtn) {
        this.handleMergedModeToggle(mergedModeToggleBtn);
      }

      const categoryBtn = e.target.closest('.category-btn');
      if (categoryBtn) {
        this.handleCategorySelection(categoryBtn);
      }
    });
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

  async loadFile(file, append = false) {
    this.setStatus(CONFIG.STATUS_MESSAGES.LOADING(file.name));

    try {
      const dataset = await loadJsonFromFile(file);
      if (append) {
        this.datasets.push(dataset);
        // Initialize selected columns for the new dataset
        const columns = getNumericColumns(dataset);
        this.selectedColumns.push(new Set(columns));
        this.displayModes.push('time');
        this.setStatus(CONFIG.STATUS_MESSAGES.EXTRA_LOADED(file.name, dataset.rows.length));
      } else {
        this.datasets = [dataset];
        // Initialize selected columns for the dataset
        const columns = getNumericColumns(dataset);
        this.selectedColumns = [new Set(columns)];
        this.displayModes = ['time'];
        this.setStatus(CONFIG.STATUS_MESSAGES.LOADED(file.name, dataset.rows.length));
        addClass(this.elements.dropZone, 'hidden');
      }
      // Initialize merged columns
      this.initializeMergedColumns();
      this.updateSidebarState();
      this.render();
    } catch (error) {
      console.error(error);
      this.setStatus(CONFIG.STATUS_MESSAGES.ERROR);
      this.elements.chartContainer.innerHTML =
        `<div class="empty-state">${CONFIG.EMPTY_STATE.PARSE_ERROR(error.message)}</div>`;
    }
  }

  loadDatasets(datasets) {
    if (datasets.length > 0) {
      this.datasets = datasets;
      // Initialize selected columns (all selected by default)
      this.selectedColumns = datasets.map(dataset => {
        const columns = getNumericColumns(dataset);
        return new Set(columns);
      });
      // Initialize display modes (all start in 'time' mode)
      this.displayModes = datasets.map(() => 'time');
      // Initialize merged selected columns
      this.initializeMergedColumns();
      this.setStatus(CONFIG.STATUS_MESSAGES.LOADED(
        `${datasets.length} file(s)`,
        datasets.reduce((sum, ds) => sum + ds.rows.length, 0)
      ));
      addClass(this.elements.dropZone, 'hidden');
      this.updateSidebarState();
      this.render();
    }
  }

  buildMergedKey(label, source) {
    return `${label}@@${source || 'JSON'}`;
  }

  getAllMergedEntries() {
    const entries = [];
    this.datasets.forEach(dataset => {
      const source = dataset.name || 'JSON';
      const columns = getNumericColumns(dataset);
      columns.forEach(label => {
        entries.push({
          label,
          source,
          key: this.buildMergedKey(label, source)
        });
      });
    });
    return entries.sort((a, b) => {
      const labelCompare = a.label.localeCompare(b.label);
      if (labelCompare !== 0) {
        return labelCompare;
      }
      return a.source.localeCompare(b.source);
    });
  }

  initializeMergedColumns() {
    this.mergedSelectedColumns = new Set();
    this.getAllMergedEntries().forEach(entry => {
      this.mergedSelectedColumns.add(entry.key);
    });
  }

  getColumnsByCategory(category) {
    const allEntries = this.getAllMergedEntries();
    if (category === 'all') {
      return allEntries;
    }
    return allEntries.filter(entry => categorizeColumn(entry.label) === category);
  }

  getCategoryCounts() {
    const allEntries = this.getAllMergedEntries();
    const counts = {
      'all': allEntries.length,
      'Frequency': 0,
      'Cardinality': 0,
      'Quantile': 0,
      'G-sum': 0,
      'Other': 0
    };

    allEntries.forEach(entry => {
      const category = categorizeColumn(entry.label);
      counts[category]++;
    });

    return counts;
  }

  updateSidebarState() {
    const mergedOption = document.querySelector('.view-option[data-view="merged"]');

    if (this.datasets.length <= 1) {
      // Disable merged view when only one dataset
      if (mergedOption) {
        mergedOption.style.opacity = '0.5';
        mergedOption.style.cursor = 'not-allowed';
        mergedOption.style.pointerEvents = 'none';
      }
      // Switch to individual view
      if (this.currentView === 'merged') {
        this.currentView = 'individual';
        this.switchView('individual');
      }
    } else {
      // Enable merged view when multiple datasets
      if (mergedOption) {
        mergedOption.style.opacity = '1';
        mergedOption.style.cursor = 'pointer';
        mergedOption.style.pointerEvents = 'auto';
      }
    }
  }

  clearAllDatasets() {
    this.destroyAllCharts();
    this.datasets = [];
    this.selectedColumns = [];
    this.mergedSelectedColumns = new Set();
    this.displayModes = [];
    this.mergedDisplayMode = 'time';
    this.selectedCategory = 'all';
    this.elements.chartContainer.innerHTML = `<div class="empty-state">${CONFIG.EMPTY_STATE.NO_DATA}</div>`;
    this.setStatus(CONFIG.STATUS_MESSAGES.WAITING);
    removeClass(this.elements.dropZone, 'hidden');
    addClass(this.elements.addMoreSection, 'hidden');
    this.elements.fileInput.value = '';
    this.elements.addMoreInput.value = '';
    this.updateSidebarState();
  }

  switchView(view) {
    this.currentView = view;

    // Update active state in sidebar
    const viewOptions = document.querySelectorAll('.view-option');
    viewOptions.forEach(option => {
      const optionView = option.getAttribute('data-view');
      if (optionView === view) {
        addClass(option, 'active');
        option.querySelector('input[type="radio"]').checked = true;
      } else {
        removeClass(option, 'active');
        option.querySelector('input[type="radio"]').checked = false;
      }
    });

    // Re-render with new view
    this.render();
  }

  render() {
    // Destroy existing charts
    this.destroyAllCharts();

    // Render chart containers based on current view
    const html = this.renderChartContainers();
    this.elements.chartContainer.innerHTML = html;

    // Create charts based on current view
    if (this.currentView === 'merged' && this.datasets.length > 1) {
      this.createMergedCharts();
    } else if (this.currentView === 'individual') {
      this.createAllCharts();
    } else if (this.datasets.length === 1) {
      // If only one dataset, show it regardless of view mode
      this.createAllCharts();
    }

    if (this.datasets.length > 0) {
      removeClass(this.elements.addMoreSection, 'hidden');
    } else {
      addClass(this.elements.addMoreSection, 'hidden');
    }
  }

  renderModeToggle(index, mode) {
    return `
      <div class="mode-toggle">
        <button class="mode-toggle-btn ${mode === 'time' ? 'active' : ''}" data-mode="time" data-index="${index}">
          ⏱️ Time
        </button>
        <button class="mode-toggle-btn ${mode === 'throughput' ? 'active' : ''}" data-mode="throughput" data-index="${index}">
          🚀 Throughput
        </button>
      </div>
    `;
  }

  renderMergedModeToggle(mode) {
    return `
      <div class="mode-toggle">
        <button class="merged-mode-toggle-btn ${mode === 'time' ? 'active' : ''}" data-mode="time">
          ⏱️ Time
        </button>
        <button class="merged-mode-toggle-btn ${mode === 'throughput' ? 'active' : ''}" data-mode="throughput">
          🚀 Throughput
        </button>
      </div>
    `;
  }

  renderCategorySelector() {
    const counts = this.getCategoryCounts();
    const categories = [
      { value: 'all', label: 'All', icon: '📊', count: counts.all },
      { value: 'Frequency', label: 'Frequency', icon: '🔢', count: counts.Frequency },
      { value: 'Cardinality', label: 'Cardinality', icon: '🎯', count: counts.Cardinality },
      { value: 'Quantile', label: 'Quantile', icon: '📈', count: counts.Quantile },
      { value: 'G-sum', label: 'G-sum', icon: '∑', count: counts['G-sum'] },
      { value: 'Other', label: 'Other', icon: '📦', count: counts.Other }
    ];

    return `
      <div class="category-selector">
        <div class="category-header">
          <span class="category-title">Filter by Category:</span>
        </div>
        <div class="category-options">
          ${categories.map(cat => `
            <button
              class="category-btn ${this.selectedCategory === cat.value ? 'active' : ''}"
              data-category="${cat.value}"
              ${cat.count === 0 ? 'disabled' : ''}
            >
              <span class="category-icon">${cat.icon}</span>
              <span class="category-label">${cat.label}</span>
              <span class="category-count">(${cat.count})</span>
            </button>
          `).join('')}
        </div>
      </div>
    `;
  }

  renderChartContainers() {
    if (!this.datasets || this.datasets.length === 0) {
      return `<div class="empty-state">${CONFIG.EMPTY_STATE.NO_DATA}</div>`;
    }

    let html = '';

    // Render based on current view
    if (this.currentView === 'merged' && this.datasets.length > 1) {
      // Show only merged view
      html += this.renderMergedViewContainers();
    } else {
      // Show individual dataset charts
      for (let i = 0; i < this.datasets.length; i++) {
        const dataset = this.datasets[i];
        const columns = getNumericColumns(dataset);
        const selectedCols = this.selectedColumns[i] || new Set(columns);
        const mode = this.displayModes[i] || 'time';

        html += `
          <div class="dataset-block">
            <div class="dataset-header">
              <div>
                <div class="dataset-title">${escapeHtml(dataset.name || 'JSON')}</div>
                <div class="dataset-meta">${dataset.rows.length} rows · ${dataset.headers.length} cols · ${selectedCols.size} selected</div>
              </div>
              <div class="dataset-actions">
                ${this.renderModeToggle(i, mode)}
                <span class="pill"><span class="pill-dot"></span>Loaded</span>
              </div>
            </div>
            ${renderColumnSelector(i, columns, selectedCols)}
            ${renderChartContainer(i)}
          </div>
        `;
      }
    }
    return html;
  }

  renderMergedViewContainers() {
    const totalDatasets = this.datasets.length;
    const allEntries = this.getAllMergedEntries();
    const categoryEntries = this.getColumnsByCategory(this.selectedCategory);
    const selectedInCategory = categoryEntries.filter(entry =>
      this.mergedSelectedColumns.has(entry.key)
    ).length;

    let html = `
      <div class="dataset-block merged-view">
        <div class="dataset-header">
          <div>
            <div class="dataset-title">🔀 Merged View - All Datasets Combined</div>
            <div class="dataset-meta">${totalDatasets} datasets merged · ${allEntries.length} total bars · ${selectedInCategory} selected in category</div>
          </div>
          <div class="dataset-actions">
            ${this.renderMergedModeToggle(this.mergedDisplayMode)}
            <span class="pill" style="background: #1e40af;"><span class="pill-dot" style="background: #3b82f6;"></span>Merged</span>
          </div>
        </div>
        ${this.renderCategorySelector()}
        ${this.renderMergedColumnSelector(categoryEntries)}
        <div class="chart-container-d3" id="merged-chart"></div>
      </div>
    `;

    return html;
  }

  renderMergedColumnSelector(entries) {
    return `
      <div class="column-selector" id="merged-selector">
        <div class="selector-header">
          <span class="selector-title">Select Columns:</span>
          <div class="selector-actions">
            <button class="selector-btn merged-selector-btn" data-action="select-all">All</button>
            <button class="selector-btn merged-selector-btn" data-action="select-none">None</button>
          </div>
        </div>
        <div class="selector-options">
          ${entries.map(entry => `
            <label class="selector-option">
              <input
                type="checkbox"
                class="column-checkbox merged-column-checkbox"
                data-column="${escapeHtml(entry.key)}"
                ${this.mergedSelectedColumns.has(entry.key) ? 'checked' : ''}
              />
              <span class="option-label">${escapeHtml(entry.label)}</span>
              <span class="option-source">(${escapeHtml(entry.source)})</span>
            </label>
          `).join('')}
        </div>
      </div>
    `;
  }

  createMergedCharts() {
    // Initialize charts array here since this runs first
    this.charts = [];

    // Get columns in the selected category
    const categoryEntries = this.getColumnsByCategory(this.selectedCategory);
    const categoryKeys = new Set(categoryEntries.map(entry => entry.key));

    // Collect all data for selected columns in the current category
    const mergedData = [];
    this.datasets.forEach(dataset => {
      const source = dataset.name || 'JSON';
      const stats = getDatasetStats(dataset);

      Object.entries(stats).forEach(([columnName, statsObj]) => {
        // Only include selected columns that are in the current category
        const key = this.buildMergedKey(columnName, source);
        if (this.mergedSelectedColumns.has(key) && categoryKeys.has(key)) {
          mergedData.push({
            label: columnName,
            source,
            value: statsObj.median,
            rawValues: statsObj.values,
            min: statsObj.min,
            max: statsObj.max,
            mean: statsObj.mean,
            count: statsObj.count
          });
        }
      });
    });

    const categoryLabel = this.selectedCategory === 'all' ? 'All Benchmarks' : `${this.selectedCategory} Benchmarks`;
    const chart = createMergedBarChart('merged-chart', categoryLabel, mergedData, this.mergedDisplayMode);
    if (chart) {
      this.charts.push(chart);
    }
  }

  createAllCharts() {
    // Don't reinitialize if merged charts were already created
    if (!this.charts || (this.datasets.length <= 1)) {
      this.charts = [];
    }

    for (let i = 0; i < this.datasets.length; i++) {
      const containerId = `chart-${i}`;
      const selectedCols = this.selectedColumns[i];
      const mode = this.displayModes[i] || 'time';
      const chart = createBarChart(containerId, this.datasets[i], selectedCols, mode);
      if (chart) {
        this.charts.push(chart);
      }
    }
  }

  destroyAllCharts() {
    for (const chart of this.charts) {
      destroyChart(chart);
    }
    this.charts = [];
  }

  handleColumnToggle(checkbox) {
    if (!checkbox.hasAttribute('data-index')) {
      return;
    }
    const index = parseInt(checkbox.getAttribute('data-index'), 10);
    if (Number.isNaN(index) || !this.selectedColumns[index]) {
      return;
    }
    const column = checkbox.getAttribute('data-column');

    if (checkbox.checked) {
      this.selectedColumns[index].add(column);
    } else {
      this.selectedColumns[index].delete(column);
    }

    // Update the meta display
    const metaEl = this.elements.chartContainer.querySelector(
      `.dataset-block:nth-child(${index + 1}) .dataset-meta`
    );
    if (metaEl) {
      const dataset = this.datasets[index];
      metaEl.textContent = `${dataset.rows.length} rows · ${dataset.headers.length} cols · ${this.selectedColumns[index].size} selected`;
    }

    // Re-render only the affected chart
    this.renderChart(index);
  }

  handleSelectorAction(button) {
    if (!button.hasAttribute('data-index')) {
      return;
    }
    const action = button.getAttribute('data-action');
    const index = parseInt(button.getAttribute('data-index'), 10);
    if (Number.isNaN(index) || !this.datasets[index]) {
      return;
    }
    const columns = getNumericColumns(this.datasets[index]);

    if (action === 'select-all') {
      this.selectedColumns[index] = new Set(columns);
    } else if (action === 'select-none') {
      this.selectedColumns[index] = new Set();
    }

    // Re-render the entire view to update checkboxes and chart
    this.render();
  }

  handleMergedColumnToggle(checkbox) {
    const column = checkbox.getAttribute('data-column');

    if (checkbox.checked) {
      this.mergedSelectedColumns.add(column);
    } else {
      this.mergedSelectedColumns.delete(column);
    }

    // Update the meta display
    const metaEl = this.elements.chartContainer.querySelector('.merged-view .dataset-meta');
    if (metaEl) {
      const allEntries = this.getAllMergedEntries();
      const categoryEntries = this.getColumnsByCategory(this.selectedCategory);
      const selectedInCategory = categoryEntries.filter(entry =>
        this.mergedSelectedColumns.has(entry.key)
      ).length;
      metaEl.textContent = `${this.datasets.length} datasets merged · ${allEntries.length} total bars · ${selectedInCategory} selected in category`;
    }

    // Re-render the merged chart
    this.renderMergedChart();
  }

  handleMergedSelectorAction(button) {
    const action = button.getAttribute('data-action');
    const allEntries = this.getAllMergedEntries();

    if (action === 'select-all') {
      this.mergedSelectedColumns = new Set(allEntries.map(entry => entry.key));
    } else if (action === 'select-none') {
      this.mergedSelectedColumns = new Set();
    }

    // Re-render the entire merged view to update checkboxes and chart
    this.render();
  }

  renderMergedChart() {
    // Destroy existing merged chart
    if (this.charts.length > 0 && this.charts[0]) {
      destroyChart(this.charts[0]);
    }

    // Get columns in the selected category
    const categoryEntries = this.getColumnsByCategory(this.selectedCategory);
    const categoryKeys = new Set(categoryEntries.map(entry => entry.key));

    // Collect all data for selected columns in the current category
    const mergedData = [];
    this.datasets.forEach(dataset => {
      const source = dataset.name || 'JSON';
      const stats = getDatasetStats(dataset);

      Object.entries(stats).forEach(([columnName, statsObj]) => {
        // Only include selected columns that are in the current category
        const key = this.buildMergedKey(columnName, source);
        if (this.mergedSelectedColumns.has(key) && categoryKeys.has(key)) {
          mergedData.push({
            label: columnName,
            source,
            value: statsObj.median,
            rawValues: statsObj.values,
            min: statsObj.min,
            max: statsObj.max,
            mean: statsObj.mean,
            count: statsObj.count
          });
        }
      });
    });

    const categoryLabel = this.selectedCategory === 'all' ? 'All Benchmarks' : `${this.selectedCategory} Benchmarks`;
    const chart = createMergedBarChart('merged-chart', categoryLabel, mergedData, this.mergedDisplayMode);
    if (chart) {
      this.charts[0] = chart;
    }
  }

  renderChart(index) {
    const containerId = `chart-${index}`;
    const selectedCols = this.selectedColumns[index];
    const mode = this.displayModes[index] || 'time';

    // Destroy existing chart
    if (this.charts[index]) {
      destroyChart(this.charts[index]);
    }

    // Create new chart
    const chart = createBarChart(containerId, this.datasets[index], selectedCols, mode);
    this.charts[index] = chart;
  }

  handleModeToggle(button) {
    const mode = button.getAttribute('data-mode');
    const index = parseInt(button.getAttribute('data-index'));

    // Update the mode
    this.displayModes[index] = mode;

    // Update button states
    const container = button.closest('.dataset-actions');
    if (container) {
      container.querySelectorAll('.mode-toggle-btn').forEach(btn => {
        if (btn.getAttribute('data-mode') === mode) {
          btn.classList.add('active');
        } else {
          btn.classList.remove('active');
        }
      });
    }

    // Re-render only the affected chart
    this.renderChart(index);
  }

  handleMergedModeToggle(button) {
    const mode = button.getAttribute('data-mode');

    // Update the mode
    this.mergedDisplayMode = mode;

    // Update button states
    const container = button.closest('.dataset-actions');
    if (container) {
      container.querySelectorAll('.merged-mode-toggle-btn').forEach(btn => {
        if (btn.getAttribute('data-mode') === mode) {
          btn.classList.add('active');
        } else {
          btn.classList.remove('active');
        }
      });
    }

    // Re-render the merged chart
    this.renderMergedChart();
  }

  handleCategorySelection(button) {
    const category = button.getAttribute('data-category');

    // Update the selected category
    this.selectedCategory = category;

    // Update the column selection to include all columns in the new category
    const categoryEntries = this.getColumnsByCategory(category);
    this.mergedSelectedColumns = new Set(categoryEntries.map(entry => entry.key));

    // Re-render the entire merged view (to update category buttons, column checkboxes, and chart)
    this.render();
  }

  setStatus(html) {
    this.elements.statusEl.innerHTML = html;
  }
}
