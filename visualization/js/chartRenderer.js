/**
 * Chart rendering utilities using Chart.js
 */

import { CONFIG } from './config.js';

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
  } else {
    return sorted[mid];
  }
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

  // Check first few non-empty values
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
 * Calculate median values for each column
 * @param {Object[]} rows - Data rows
 * @param {string[]} headers - Column headers
 * @returns {Object} Object with column names as keys and median values
 */
function calculateColumnMedians(rows, headers) {
  const medians = {};

  for (const header of headers) {
    // Skip non-numeric columns
    if (!isNumericColumn(rows, header)) {
      continue;
    }

    // Extract numeric values for this column
    const values = rows
      .map(row => parseFloat(row[header]))
      .filter(val => !isNaN(val));

    if (values.length > 0) {
      medians[header] = calculateMedian(values);
    }
  }

  return medians;
}

/**
 * Create a bar chart for a dataset
 * @param {string} canvasId - ID of the canvas element
 * @param {Object} dataset - Dataset object with headers and rows
 * @returns {Chart} Chart.js instance
 */
export function createBarChart(canvasId, dataset) {
  const { headers, rows, name } = dataset;

  // Calculate medians for each column
  const medians = calculateColumnMedians(rows, headers);

  // Prepare data for Chart.js
  const labels = Object.keys(medians);
  const data = Object.values(medians);

  if (labels.length === 0) {
    console.warn(`No numeric data found for chart: ${name}`);
    return null;
  }

  // Get canvas context
  const canvas = document.getElementById(canvasId);
  if (!canvas) {
    console.error(`Canvas element not found: ${canvasId}`);
    return null;
  }

  const ctx = canvas.getContext('2d');

  // Create chart
  const chart = new Chart(ctx, {
    type: CONFIG.CHART.TYPE,
    data: {
      labels: labels,
      datasets: [{
        label: 'Median Time (μs)',
        data: data,
        backgroundColor: CONFIG.CHART.BACKGROUND_COLOR,
        borderColor: CONFIG.CHART.BORDER_COLOR,
        borderWidth: CONFIG.CHART.BORDER_WIDTH,
      }]
    },
    options: {
      responsive: true,
      maintainAspectRatio: false,
      plugins: {
        legend: {
          display: true,
          labels: {
            color: CONFIG.CHART.FONT_COLOR
          }
        },
        title: {
          display: true,
          text: `${name} - Median Times`,
          color: CONFIG.CHART.FONT_COLOR,
          font: {
            size: 16,
            weight: 'bold'
          }
        },
        tooltip: {
          callbacks: {
            label: function(context) {
              return `Median: ${context.parsed.y.toFixed(2)} μs`;
            }
          }
        }
      },
      scales: {
        x: {
          ticks: {
            color: CONFIG.CHART.FONT_COLOR,
            maxRotation: 45,
            minRotation: 45,
            font: {
              size: 10
            }
          },
          grid: {
            color: CONFIG.CHART.GRID_COLOR
          }
        },
        y: {
          beginAtZero: true,
          ticks: {
            color: CONFIG.CHART.FONT_COLOR
          },
          grid: {
            color: CONFIG.CHART.GRID_COLOR
          },
          title: {
            display: true,
            text: CONFIG.CHART.Y_AXIS_LABEL,
            color: CONFIG.CHART.FONT_COLOR
          }
        }
      }
    }
  });

  return chart;
}

/**
 * Render a chart container HTML
 * @param {number} index - Dataset index
 * @returns {string} HTML string for chart container
 */
export function renderChartContainer(index) {
  const canvasId = `chart-${index}`;

  return `
    <div class="chart-container">
      <canvas id="${canvasId}"></canvas>
    </div>
  `;
}

/**
 * Destroy a chart instance to prevent memory leaks
 * @param {Chart} chart - Chart.js instance to destroy
 */
export function destroyChart(chart) {
  if (chart) {
    chart.destroy();
  }
}
