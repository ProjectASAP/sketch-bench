/**
 * Main application entry point for tables view
 */

import { UIController } from './uiControllerTables.js';
import { autoLoadDataFiles } from './dataLoader.js';

/**
 * Application class for tables view
 */
class SketchBenchmarkTablesViewer {
  constructor() {
    this.uiController = new UIController();
  }

  /**
   * Initialize the application
   */
  async init() {
    // Initialize UI controller
    this.uiController.init();

    // Auto-load data files from the data directory
    await this.autoLoadData();
  }

  /**
   * Auto-load data files on startup
   */
  async autoLoadData() {
    try {
      const datasets = await autoLoadDataFiles();
      if (datasets.length > 0) {
        this.uiController.loadDatasets(datasets);
      }
    } catch (error) {
      console.error('Failed to auto-load data:', error);
    }
  }
}

/**
 * Initialize the application when DOM is ready
 */
document.addEventListener('DOMContentLoaded', () => {
  const app = new SketchBenchmarkTablesViewer();
  app.init();
});
