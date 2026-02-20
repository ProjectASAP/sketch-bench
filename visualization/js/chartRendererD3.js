/**
 * Chart rendering utilities using D3.js
 */

import { CONFIG } from './config.js';
import { escapeHtml } from './utils.js';

const statsCache = new WeakMap();

const INDIVIDUAL_CHART_CONFIG = {
  margin: { top: 60, right: 30, bottom: 120, left: 80 },
  minWidth: 800,
  barWidth: 60,
  height: 500
};

const MERGED_CHART_CONFIG = {
  margin: { top: 60, right: 30, bottom: 150, left: 100 },
  minWidth: 1000,
  barWidth: 40,
  height: 500
};

/**
 * Convert time (microseconds) to throughput (items/sec)
 * Formula: 1,000,000 / (time_μs / 1,000,000) = 10^12 / time_μs
 * @param {number} timeInMicroseconds - Time in microseconds
 * @returns {number} Throughput in items/sec
 */
function convertToThroughput(timeInMicroseconds) {
  if (timeInMicroseconds === 0) return 0;
  return (1e12) / timeInMicroseconds;
}

/**
 * Calculate median of an array of numbers
 * @param {number[]} values - Array of numeric values
 * @returns {number} Median value
 */
function calculateMedian(values) {
  if (values.length === 0) return 0;

  const sorted = [...values].sort((a, b) => a - b);
  const mid = Math.floor(sorted.length / 2);

  if (sorted.length % 2 === 0) {
    return (sorted[mid - 1] + sorted[mid]) / 2;
  }
  return sorted[mid];
}

/**
 * Check if a column contains numeric data
 * @param {Object[]} rows - Data rows
 * @param {string} columnName - Column name to check
 * @returns {boolean} True if column is numeric
 */
function isNumericColumn(rows, columnName) {
  if (CONFIG.CHART.SKIP_COLUMNS.includes(columnName.toLowerCase())) {
    return false;
  }

  for (let i = 0; i < Math.min(5, rows.length); i++) {
    const value = rows[i][columnName];
    if (value && value.trim() !== '') {
      const num = parseFloat(value);
      if (isNaN(num)) {
        return false;
      }
    }
  }

  return true;
}

/**
 * Calculate statistics for each column including raw data
 * @param {Object[]} rows - Data rows
 * @param {string[]} headers - Column headers
 * @returns {Object} Object with column names as keys and stats objects
 */
export function calculateColumnStats(rows, headers) {
  const stats = {};

  for (const header of headers) {
    if (!isNumericColumn(rows, header)) {
      continue;
    }

    const values = rows
      .map(row => parseFloat(row[header]))
      .filter(val => !isNaN(val));

    if (values.length > 0) {
      const sorted = [...values].sort((a, b) => a - b);
      stats[header] = {
        median: calculateMedian(values),
        values,
        min: sorted[0],
        max: sorted[sorted.length - 1],
        count: values.length,
        mean: values.reduce((a, b) => a + b, 0) / values.length
      };
    }
  }

  return stats;
}

/**
 * Return cached stats for a dataset to avoid recomputation
 * @param {Object} dataset - Dataset object with headers and rows
 * @returns {Object} Statistics keyed by column name
 */
export function getDatasetStats(dataset) {
  if (!statsCache.has(dataset)) {
    statsCache.set(dataset, calculateColumnStats(dataset.rows, dataset.headers));
  }
  return statsCache.get(dataset);
}

function createChartBase(containerId, dataLength, config) {
  const { margin, minWidth, barWidth, height } = config;
  const width = Math.max(minWidth, dataLength * barWidth) - margin.left - margin.right;
  const innerHeight = height - margin.top - margin.bottom;

  const container = d3.select(`#${containerId}`);
  container.selectAll('*').remove();

  const svg = container
    .append('svg')
    .attr('width', width + margin.left + margin.right)
    .attr('height', height)
    .append('g')
    .attr('transform', `translate(${margin.left},${margin.top})`);

  return { container, svg, width, height: innerHeight };
}

function addAxesAndLabels({ svg, x, y, width, height, margin, title, yAxisLabel, rotateXLabels, xAxisFontSize }) {
  const xAxis = svg.append('g')
    .attr('transform', `translate(0,${height})`)
    .call(d3.axisBottom(x));

  const xText = xAxis.selectAll('text')
    .style('text-anchor', rotateXLabels ? 'end' : 'middle')
    .style('fill', CONFIG.CHART.FONT_COLOR)
    .style('font-size', xAxisFontSize);

  if (rotateXLabels) {
    xText.attr('transform', 'rotate(-45)');
  }

  svg.append('g')
    .call(d3.axisLeft(y).ticks(10))
    .selectAll('text')
    .style('fill', CONFIG.CHART.FONT_COLOR);

  svg.append('text')
    .attr('transform', 'rotate(-90)')
    .attr('y', 0 - margin.left + 20)
    .attr('x', 0 - (height / 2))
    .attr('dy', '1em')
    .style('text-anchor', 'middle')
    .style('fill', CONFIG.CHART.FONT_COLOR)
    .style('font-size', '14px')
    .text(yAxisLabel);

  svg.append('text')
    .attr('x', width / 2)
    .attr('y', 0 - (margin.top / 2))
    .attr('text-anchor', 'middle')
    .style('font-size', '18px')
    .style('font-weight', 'bold')
    .style('fill', CONFIG.CHART.FONT_COLOR)
    .text(title);

  svg.selectAll('.domain, .tick line')
    .style('stroke', CONFIG.CHART.GRID_COLOR);
}

function createTooltip() {
  return d3.select('body')
    .append('div')
    .attr('class', 'd3-tooltip')
    .style('position', 'fixed')
    .style('visibility', 'hidden')
    .style('background', 'rgba(0, 0, 0, 0.95)')
    .style('color', '#fff')
    .style('padding', '12px 16px')
    .style('border-radius', '8px')
    .style('font-size', '13px')
    .style('pointer-events', 'none')
    .style('z-index', '10000')
    .style('max-width', '400px')
    .style('box-shadow', '0 4px 12px rgba(0,0,0,0.5)')
    .style('border', '1px solid rgba(255,255,255,0.1)');
}

function positionTooltip(tooltip, event) {
  const tooltipNode = tooltip.node();
  const tooltipWidth = tooltipNode.offsetWidth;
  const tooltipHeight = tooltipNode.offsetHeight;
  const viewportWidth = window.innerWidth;
  const viewportHeight = window.innerHeight;

  let left = event.clientX + 15;
  let top = event.clientY - 15;

  if (left + tooltipWidth > viewportWidth) {
    left = event.clientX - tooltipWidth - 15;
  }
  if (top + tooltipHeight > viewportHeight) {
    top = event.clientY - tooltipHeight - 15;
  }
  if (top < 0) {
    top = event.clientY + 15;
  }

  tooltip
    .style('top', top + 'px')
    .style('left', left + 'px');
}

function formatRawValues(values, formatter) {
  if (values.length <= 20) {
    return values.map(formatter).join(', ');
  }
  return `${values.slice(0, 20).map(formatter).join(', ')}... (+${values.length - 20} more)`;
}

function formatThroughputValue(val) {
  if (val >= 1e6) {
    return `${(val / 1e6).toFixed(2)}M`;
  }
  return val.toFixed(2);
}

function getYAxisLabel(mode) {
  return mode === 'throughput'
    ? CONFIG.CHART.Y_AXIS_LABEL_THROUGHPUT
    : CONFIG.CHART.Y_AXIS_LABEL;
}

function buildSingleChartTooltip(d, mode) {
  const unit = mode === 'throughput' ? 'items/sec' : 'μs';
  const rawValuesDisplay = formatRawValues(d.rawValues, val => val.toFixed(2));
  const formatNumber = mode === 'throughput' ? formatThroughputValue : (val) => val.toFixed(2);

  const tooltipContent = mode === 'throughput' ? `
    <div style="line-height: 1.6;">
      <div><strong>Median:</strong> ${formatNumber(d.value)} ${unit}</div>
      <div><strong>Mean:</strong> ${formatNumber(d.mean)} ${unit}</div>
      <div><strong>Min:</strong> ${formatNumber(d.min)} ${unit}</div>
      <div><strong>Max:</strong> ${formatNumber(d.max)} ${unit}</div>
      <div><strong>Count:</strong> ${d.count} runs</div>
    </div>
    <div style="margin-top: 8px; padding-top: 8px; border-top: 1px solid rgba(255,255,255,0.1); font-size: 11px; color: #aaa;">
      <div><strong style="color: #fff;">Time:</strong> ${d.originalMedian.toFixed(2)} μs (median)</div>
    </div>
  ` : `
    <div style="line-height: 1.6;">
      <div><strong>Median:</strong> ${d.value.toFixed(2)} ${unit}</div>
      <div><strong>Mean:</strong> ${d.mean.toFixed(2)} ${unit}</div>
      <div><strong>Min:</strong> ${d.min.toFixed(2)} ${unit}</div>
      <div><strong>Max:</strong> ${d.max.toFixed(2)} ${unit}</div>
      <div><strong>Count:</strong> ${d.count} runs</div>
    </div>
    <div style="margin-top: 8px; padding-top: 8px; border-top: 1px solid rgba(255,255,255,0.1); font-size: 11px; color: #aaa;">
      <div><strong style="color: #fff;">Throughput:</strong> ${formatNumber(convertToThroughput(d.originalMedian))} items/sec (median)</div>
    </div>
  `;

  return `
    <div style="font-weight: bold; font-size: 14px; margin-bottom: 8px; border-bottom: 1px solid rgba(255,255,255,0.2); padding-bottom: 6px;">
      ${escapeHtml(d.label)}
    </div>
    ${tooltipContent}
    <div style="margin-top: 10px; padding-top: 8px; border-top: 1px solid rgba(255,255,255,0.2); font-size: 11px; color: #aaa;">
      <strong style="color: #fff;">Raw Data (${unit}):</strong><br/>
      <div style="margin-top: 4px; max-height: 100px; overflow-y: auto; font-family: monospace; background: rgba(0,0,0,0.3); padding: 6px; border-radius: 4px;">
        ${rawValuesDisplay}
      </div>
    </div>
  `;
}

function buildMergedChartTooltip(d, mode) {
  const unit = mode === 'throughput' ? 'items/sec' : 'μs';
  const rawValuesDisplay = formatRawValues(d.rawValues, val => val.toFixed(2));
  const formatNumber = mode === 'throughput' ? formatThroughputValue : (val) => val.toFixed(2);

  const tooltipContent = mode === 'throughput' ? `
    <div style="line-height: 1.6;">
      <div><strong>Median:</strong> ${formatNumber(d.value)} ${unit}</div>
      <div><strong>Mean:</strong> ${formatNumber(d.mean)} ${unit}</div>
      <div><strong>Min:</strong> ${formatNumber(d.min)} ${unit}</div>
      <div><strong>Max:</strong> ${formatNumber(d.max)} ${unit}</div>
      <div><strong>Count:</strong> ${d.count} runs</div>
    </div>
    <div style="margin-top: 8px; padding-top: 8px; border-top: 1px solid rgba(255,255,255,0.1); font-size: 11px; color: #aaa;">
      <div><strong style="color: #fff;">Time:</strong> ${d.originalValue.toFixed(2)} μs (median)</div>
    </div>
  ` : `
    <div style="line-height: 1.6;">
      <div><strong>Median:</strong> ${d.value.toFixed(2)} ${unit}</div>
      <div><strong>Mean:</strong> ${d.mean.toFixed(2)} ${unit}</div>
      <div><strong>Min:</strong> ${d.min.toFixed(2)} ${unit}</div>
      <div><strong>Max:</strong> ${d.max.toFixed(2)} ${unit}</div>
      <div><strong>Count:</strong> ${d.count} runs</div>
    </div>
    <div style="margin-top: 8px; padding-top: 8px; border-top: 1px solid rgba(255,255,255,0.1); font-size: 11px; color: #aaa;">
      <div><strong style="color: #fff;">Throughput:</strong> ${formatNumber(convertToThroughput(d.originalValue))} items/sec (median)</div>
    </div>
  `;

  return `
    <div style="font-weight: bold; font-size: 14px; margin-bottom: 8px; border-bottom: 1px solid rgba(255,255,255,0.2); padding-bottom: 6px;">
      ${escapeHtml(d.label)}
    </div>
    <div style="margin-bottom: 8px; padding-bottom: 6px; border-bottom: 1px solid rgba(255,255,255,0.1);">
      <strong>Source:</strong> ${escapeHtml(d.source)}
    </div>
    ${tooltipContent}
    <div style="margin-top: 10px; padding-top: 8px; border-top: 1px solid rgba(255,255,255,0.2); font-size: 11px; color: #aaa;">
      <strong style="color: #fff;">Raw Data (${unit}):</strong><br/>
      <div style="margin-top: 4px; max-height: 100px; overflow-y: auto; font-family: monospace; background: rgba(0,0,0,0.3); padding: 6px; border-radius: 4px;">
        ${rawValuesDisplay}
      </div>
    </div>
  `;
}

/**
 * Create a bar chart using D3.js
 * @param {string} containerId - ID of the container element
 * @param {Object} dataset - Dataset object with headers and rows
 * @param {Set} selectedColumns - Set of selected column names to display
 * @param {string} mode - Display mode: 'time' or 'throughput'
 * @returns {Object} Chart object with cleanup method
 */
export function createBarChart(containerId, dataset, selectedColumns = null, mode = 'time') {
  const { name } = dataset;

  const stats = getDatasetStats(dataset);
  const filteredStats = selectedColumns
    ? Object.fromEntries(
        Object.entries(stats).filter(([key]) => selectedColumns.has(key))
      )
    : stats;

  const data = Object.entries(filteredStats).map(([key, statsObj]) => {
    const isThroughputMode = mode === 'throughput';
    return {
      label: key,
      value: isThroughputMode ? convertToThroughput(statsObj.median) : statsObj.median,
      rawValues: isThroughputMode ? statsObj.values.map(convertToThroughput) : statsObj.values,
      min: isThroughputMode ? convertToThroughput(statsObj.max) : statsObj.min,
      max: isThroughputMode ? convertToThroughput(statsObj.min) : statsObj.max,
      mean: isThroughputMode ? convertToThroughput(statsObj.mean) : statsObj.mean,
      count: statsObj.count,
      originalValues: statsObj.values,
      originalMedian: statsObj.median,
      originalMin: statsObj.min,
      originalMax: statsObj.max,
      originalMean: statsObj.mean
    };
  });

  if (data.length === 0) {
    console.warn(`No numeric data found for chart: ${name}`);
    return null;
  }

  const base = createChartBase(containerId, data.length, INDIVIDUAL_CHART_CONFIG);
  const { svg, width, height, container } = base;

  const x = d3.scaleBand()
    .domain(data.map(d => d.label))
    .range([0, width])
    .padding(0.2);

  const y = d3.scaleLinear()
    .domain([0, d3.max(data, d => d.value) * 1.1])
    .nice()
    .range([height, 0]);

  const titleSuffix = mode === 'throughput' ? 'Median Throughput' : 'Median Times';
  addAxesAndLabels({
    svg,
    x,
    y,
    width,
    height,
    margin: INDIVIDUAL_CHART_CONFIG.margin,
    title: `${name} - ${titleSuffix}`,
    yAxisLabel: getYAxisLabel(mode),
    rotateXLabels: true,
    xAxisFontSize: '11px'
  });

  const bars = svg.selectAll('.bar')
    .data(data)
    .enter()
    .append('rect')
    .attr('class', 'bar')
    .attr('x', d => x(d.label))
    .attr('y', d => y(d.value))
    .attr('width', x.bandwidth())
    .attr('height', d => height - y(d.value))
    .attr('fill', CONFIG.CHART.BACKGROUND_COLOR)
    .attr('stroke', CONFIG.CHART.BORDER_COLOR)
    .attr('stroke-width', CONFIG.CHART.BORDER_WIDTH);

  const tooltip = createTooltip();

  bars
    .on('mouseover', function(event, d) {
      d3.select(this).attr('fill', CONFIG.CHART.BORDER_COLOR);
      tooltip
        .style('visibility', 'visible')
        .html(buildSingleChartTooltip(d, mode));
    })
    .on('mousemove', (event) => {
      positionTooltip(tooltip, event);
    })
    .on('mouseout', function() {
      d3.select(this).attr('fill', CONFIG.CHART.BACKGROUND_COLOR);
      tooltip.style('visibility', 'hidden');
    });

  return {
    destroy: () => {
      container.selectAll('*').remove();
      tooltip.remove();
    }
  };
}

/**
 * Get all numeric column names from a dataset
 * @param {Object} dataset - Dataset object with headers and rows
 * @returns {string[]} Array of numeric column names
 */
export function getNumericColumns(dataset) {
  const stats = getDatasetStats(dataset);
  return Object.keys(stats);
}

/**
 * Render column selector checkboxes
 * @param {number} index - Dataset index
 * @param {string[]} columns - Array of column names
 * @param {Set} selectedColumns - Set of currently selected columns
 * @returns {string} HTML string for column selector
 */
export function renderColumnSelector(index, columns, selectedColumns) {
  return `
    <div class="column-selector" id="selector-${index}">
      <div class="selector-header">
        <span class="selector-title">Select Columns:</span>
        <div class="selector-actions">
          <button class="selector-btn" data-action="select-all" data-index="${index}">All</button>
          <button class="selector-btn" data-action="select-none" data-index="${index}">None</button>
        </div>
      </div>
      <div class="selector-options">
        ${columns.map(col => `
          <label class="selector-option">
            <input
              type="checkbox"
              class="column-checkbox"
              data-index="${index}"
              data-column="${escapeHtml(col)}"
              ${selectedColumns.has(col) ? 'checked' : ''}
            />
            <span class="option-label">${escapeHtml(col)}</span>
          </label>
        `).join('')}
      </div>
    </div>
  `;
}

/**
 * Render a chart container HTML
 * @param {number} index - Dataset index
 * @returns {string} HTML string for chart container
 */
export function renderChartContainer(index) {
  const containerId = `chart-${index}`;

  return `
    <div class="chart-container-d3" id="${containerId}">
    </div>
  `;
}

/**
 * Destroy a chart instance to prevent memory leaks
 * @param {Object} chart - Chart object with destroy method
 */
export function destroyChart(chart) {
  if (chart && chart.destroy) {
    chart.destroy();
  }
}

/**
 * Categorize a column name based on keywords
 * @param {string} columnName - Column name to categorize
 * @returns {string} Category name
 */
export function categorizeColumn(columnName) {
  const lower = columnName.toLowerCase();

  if (lower.includes('count')) {
    return 'Frequency';
  }
  if (lower.includes('hll') || lower.includes('hyperloglog')) {
    return 'Cardinality';
  }
  if (lower.includes('kll')) {
    return 'Quantile';
  }
  if (lower.includes('univmon')) {
    return 'G-sum';
  }
  return 'Other';
}

/**
 * Merge data from multiple datasets and categorize by column type
 * @param {Object[]} datasets - Array of dataset objects
 * @returns {Object} Categorized data by graph type
 */
export function mergeAndCategorizeData(datasets) {
  const categories = {
    'Frequency': [],
    'Cardinality': [],
    'Quantile': [],
    'G-sum': [],
    'Other': []
  };

  datasets.forEach(dataset => {
    const { name } = dataset;
    const stats = getDatasetStats(dataset);

    Object.entries(stats).forEach(([columnName, statsObj]) => {
      const category = categorizeColumn(columnName);
      categories[category].push({
        label: columnName,
        source: name,
        value: statsObj.median,
        rawValues: statsObj.values,
        min: statsObj.min,
        max: statsObj.max,
        mean: statsObj.mean,
        count: statsObj.count
      });
    });
  });

  return categories;
}

/**
 * Create a merged bar chart for a specific category
 * @param {string} containerId - ID of the container element
 * @param {string} categoryName - Name of the category
 * @param {Object[]} data - Data for the category
 * @param {string} mode - Display mode: 'time' or 'throughput'
 * @returns {Object} Chart object with cleanup method
 */
export function createMergedBarChart(containerId, categoryName, data, mode = 'time') {
  if (data.length === 0) {
    const container = d3.select(`#${containerId}`);
    container.html('<div class="empty-state">No data for this category</div>');
    return null;
  }

  const isThroughputMode = mode === 'throughput';
  const labelCounts = new Map();
  data.forEach(d => {
    labelCounts.set(d.label, (labelCounts.get(d.label) || 0) + 1);
  });

  const usedLabels = new Set();
  const processedData = data.map(d => ({
    ...d,
    xLabel: (() => {
      const base = labelCounts.get(d.label) > 1 ? `${d.label} (${d.source})` : d.label;
      if (!usedLabels.has(base)) {
        usedLabels.add(base);
        return base;
      }
      let suffix = 2;
      let candidate = `${base} #${suffix}`;
      while (usedLabels.has(candidate)) {
        suffix += 1;
        candidate = `${base} #${suffix}`;
      }
      usedLabels.add(candidate);
      return candidate;
    })(),
    originalValue: d.value,
    originalMin: d.min,
    originalMax: d.max,
    originalMean: d.mean,
    originalRawValues: d.rawValues,
    value: isThroughputMode ? convertToThroughput(d.value) : d.value,
    min: isThroughputMode ? convertToThroughput(d.max) : d.min,
    max: isThroughputMode ? convertToThroughput(d.min) : d.max,
    mean: isThroughputMode ? convertToThroughput(d.mean) : d.mean,
    rawValues: isThroughputMode ? d.rawValues.map(convertToThroughput) : d.rawValues
  }));

  const base = createChartBase(containerId, processedData.length, MERGED_CHART_CONFIG);
  const { svg, width, height, container } = base;

  const x = d3.scaleBand()
    .domain(processedData.map(d => d.xLabel))
    .range([0, width])
    .padding(0.2);

  const y = d3.scaleLinear()
    .domain([0, d3.max(processedData, d => d.value) * 1.1])
    .nice()
    .range([height, 0]);

  const titleSuffix = mode === 'throughput' ? 'Median Throughput' : 'Median Times';
  addAxesAndLabels({
    svg,
    x,
    y,
    width,
    height,
    margin: MERGED_CHART_CONFIG.margin,
    title: `${categoryName} Benchmarks - ${titleSuffix}`,
    yAxisLabel: getYAxisLabel(mode),
    rotateXLabels: true,
    xAxisFontSize: '10px'
  });

  const colorScale = d3.scaleOrdinal()
    .domain([...new Set(data.map(d => d.source))])
    .range(['#3b82f6', '#10b981', '#f59e0b', '#ef4444', '#8b5cf6', '#ec4899']);

  const bars = svg.selectAll('.bar')
    .data(processedData)
    .enter()
    .append('rect')
    .attr('class', 'bar')
    .attr('x', d => x(d.xLabel))
    .attr('y', d => y(d.value))
    .attr('width', x.bandwidth())
    .attr('height', d => height - y(d.value))
    .attr('fill', d => colorScale(d.source))
    .attr('opacity', 0.8)
    .attr('stroke', CONFIG.CHART.BORDER_COLOR)
    .attr('stroke-width', CONFIG.CHART.BORDER_WIDTH);

  const tooltip = createTooltip();

  bars
    .on('mouseover', function(event, d) {
      d3.select(this)
        .attr('opacity', 1)
        .attr('stroke-width', 2);

      tooltip
        .style('visibility', 'visible')
        .html(buildMergedChartTooltip(d, mode));
    })
    .on('mousemove', (event) => {
      positionTooltip(tooltip, event);
    })
    .on('mouseout', function() {
      d3.select(this)
        .attr('opacity', 0.8)
        .attr('stroke-width', CONFIG.CHART.BORDER_WIDTH);

      tooltip.style('visibility', 'hidden');
    });

  const legend = svg.append('g')
    .attr('class', 'legend')
    .attr('transform', `translate(${width - 150}, 10)`);

  const sources = [...new Set(data.map(d => d.source))];
  sources.forEach((source, i) => {
    const legendRow = legend.append('g')
      .attr('transform', `translate(0, ${i * 20})`);

    legendRow.append('rect')
      .attr('width', 12)
      .attr('height', 12)
      .attr('fill', colorScale(source))
      .attr('opacity', 0.8);

    legendRow.append('text')
      .attr('x', 18)
      .attr('y', 10)
      .style('font-size', '11px')
      .style('fill', CONFIG.CHART.FONT_COLOR)
      .text(source.length > 20 ? `${source.substring(0, 20)}...` : source);
  });

  return {
    destroy: () => {
      container.selectAll('*').remove();
      tooltip.remove();
    }
  };
}
