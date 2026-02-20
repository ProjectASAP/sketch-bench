# Sketch Benchmarks Visualization

A modern, modular web application for visualizing sketch benchmark results from JSON/JSONL files.

## Features

- 🚀 **Auto-loading**: Automatically loads JSON files from the output directories on startup
- 📁 **Drag & Drop**: Drop JSON/JSONL files directly onto the interface
- 📊 **Responsive Tables**: Tables automatically adapt to screen size
- 📈 **Bar Charts**: Automatic visualization of benchmark data with median values
- 🎨 **Dark Theme**: Modern dark UI with smooth interactions
- ➕ **Multi-file Support**: Load and view multiple JSON/JSONL files simultaneously

## Project Structure

```bash
visualization/
├── index.html              # Main HTML page (USE THIS)
├── webpage.legacy.html     # Legacy monolithic backup (DO NOT USE)
├── css/
│   └── styles.css          # All application styles
├── js/
│   ├── app.js              # Main application entry point
│   ├── config.js           # Configuration constants
│   ├── utils.js            # Utility functions
│   ├── jsonParser.js       # JSON/JSONL parsing logic
│   ├── dataLoader.js       # Data loading from files/URLs
│   ├── chartRenderer.js    # Chart visualization with Chart.js
│   ├── tableRenderer.js    # Table rendering logic
│   └── uiController.js     # UI event handling and state management
├── data/
│   ├── index.json          # Manifest file listing available JSON files
└── gather_data.py          # Legacy CSV collector (not used for JSON flow)
```

## Architecture

The application follows a modular architecture with clear separation of concerns:

### Modules

1. **app.js** - Application orchestration
   - Entry point for the application
   - Coordinates initialization and auto-loading

2. **config.js** - Configuration
   - Centralized configuration constants
   - UI messages and settings
   - Easy to extend and customize

3. **utils.js** - Utilities
   - HTML escaping for security
   - DOM manipulation helpers
   - Debounce function for performance

4. **jsonParser.js** - JSON Parsing
   - Supports JSON arrays and JSONL streams
   - Converts JSON text to structured records

5. **dataLoader.js** - Data Loading
   - Load JSON from URLs (auto-load feature)
   - Load JSON from File objects (drag & drop)
   - Manifest-based and fallback loading strategies

6. **chartRenderer.js** - Chart Visualization
   - Calculates median values for each numeric column
   - Creates bar charts using Chart.js
   - Automatically skips non-numeric columns (like "run")
   - Dark theme integration with customizable colors

7. **tableRenderer.js** - Table Rendering
   - Dynamic table grouping based on column width
   - Responsive table layout
   - HTML generation with proper escaping

8. **uiController.js** - UI Controller
   - Event handling (clicks, drag & drop, file inputs)
   - State management (datasets array and charts)
   - DOM updates and rendering coordination
   - Chart lifecycle management (creation and cleanup)

## Data Format

Each JSON/JSONL file contains benchmark results where each record has:

```json
{"implementation_name":"sketchlib_kll","total_nanoseconds":27792768}
```

- Records are grouped by `implementation_name` to form columns
- Runs are inferred by occurrence order
- Values are converted from nanoseconds to microseconds

## Usage

### Running Locally

The application requires a web server due to ES6 module imports and CORS restrictions:

```bash
# Navigate to repo root so output/ dirs are served
cd /path/to/sketch-bench

# Start a simple HTTP server (Python 3)
python3 -m http.server 8000

# Or using Node.js (if you have http-server installed)
npx http-server -p 8000
```

Then open <http://localhost:8000/visualization/> in your browser.

### Auto-loading Data

Update `visualization/data/index.json` with JSON/JSONL files from the output directories:

```json
{
  "files": [
    "../cpp/output/kll_final.jsonl",
    "../rust/output/kll_lib.jsonl",
    "../rust/output/hll_oxide.jsonl"
  ]
}
```

The application will automatically load these files on startup.

### Manual Upload

1. Click on the drop zone or use the file input
2. Select a JSON/JSONL file from your computer
3. The data will be displayed in responsive tables

### Adding More Files

After loading the initial data:

1. Use the "Add more files" drop zone at the bottom
2. Drop or select additional JSON/JSONL files
3. All files will be displayed in separate sections

## Extending the Application

### Adding New Features

The modular architecture makes it easy to extend:

1. **New configuration** → Edit `config.js`
2. **New utility function** → Add to `utils.js`
3. **New rendering logic** → Extend `tableRenderer.js`
4. **New UI interaction** → Add to `uiController.js`
5. **New data source** → Extend `dataLoader.js`

### Example: Adding Export Functionality

```javascript
// In uiController.js
exportToJSON() {
  const json = JSON.stringify(this.datasets, null, 2);
  const blob = new Blob([json], { type: 'application/json' });
  const url = URL.createObjectURL(blob);
  const a = document.createElement('a');
  a.href = url;
  a.download = 'benchmarks.json';
  a.click();
}
```

### Example: Custom Table Styling

```javascript
// In tableRenderer.js
function renderTableCard(subHeaders, rows) {
  // Add custom classes or attributes
  let html = '<div class="table-card custom-style"><table>';
  // ... rest of rendering
}
```

## Best Practices

1. **Separation of Concerns**: Each module has a single responsibility
2. **ES6 Modules**: Use import/export for clean dependencies
3. **Configuration**: Centralize magic numbers and strings in `config.js`
4. **Security**: Always escape HTML to prevent XSS attacks
5. **Performance**: Debounce expensive operations like resize handlers
6. **Accessibility**: Include ARIA labels and semantic HTML

## Browser Support

- Chrome/Edge (latest)
- Firefox (latest)
- Safari (latest)

Requires ES6 module support.

## Development

### Code Style

- Use JSDoc comments for functions
- Follow camelCase naming convention
- Keep functions small and focused
- Use const/let instead of var
- Prefer async/await over callbacks

### Debugging

Open browser DevTools (F12) to see:

- Console logs for auto-loading status
- Network tab for JSON file requests
- Elements tab for DOM inspection

## Migration from Legacy

**⚠️ IMPORTANT: Use `index.html` not `webpage.legacy.html`**

The old `webpage.legacy.html` (formerly `webpage.html`) is a deprecated monolithic file kept only as a backup. It contains a warning banner and should not be used.

The new `index.html` + modular structure offers:

- ✅ Better maintainability
- ✅ Easier testing
- ✅ Clearer dependencies
- ✅ Simpler debugging
- ✅ More extensible architecture

To migrate custom changes from the legacy file:

1. Identify which module the change belongs to (config, UI, parsing, rendering, etc.)
2. Update the appropriate file in the modular structure
3. Test in the browser using `index.html`
