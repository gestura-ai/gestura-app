# Screen Recording/Screenshot Tool Integration Assessment

**Date:** 2026-02-03  
**Status:** Implementation Plan  
**Goal:** Integrate screen recording and screenshot capabilities as MCP tools available to LLMs

---

## Executive Summary

The screen recording and screenshot functionality has been **fully implemented** at the core level (`gestura-core/src/tools/screen.rs`) with cross-platform support (macOS, Linux, Windows). However, it is **NOT integrated into the agent workflow** and cannot be used by LLMs.

### Current State

✅ **Complete:**
- Core implementation in `crates/gestura-core/src/tools/screen.rs` (746 lines)
- Platform-specific screenshot and recording for macOS, Linux, Windows
- Async wrappers in `tools/mod.rs` (`screen_async` module)
- Tool definitions in `tools/registry.rs` (2 new tools: `screenshot`, `screen_record`)
- Permission checks in `gestura-gui/src/permissions.rs`

❌ **Missing:**
- Tool execution dispatcher in `pipeline/mod.rs` (no cases for screenshot/screen_record)
- JSON schemas in `tools/schemas.rs` (incomplete or missing)
- CLI commands (`gestura tools screenshot`, `gestura tools screen_record`)
- GUI image display for screenshot results
- LLM integration testing

---

## Gap Analysis

### 1. Pipeline Integration (CRITICAL)

**File:** `crates/gestura-core/src/pipeline/mod.rs`  
**Function:** `execute_tool()` at line 2457

**Current dispatcher:**
```rust
let result = match name {
    "shell" | "bash" | "execute" => self.execute_shell_tool(arguments, workspace).await,
    "file" | "read_file" | "write_file" => self.execute_file_tool(arguments, workspace).await,
    "git" => self.execute_git_tool(arguments, workspace).await,
    "web" | "web_search" => self.execute_web_tool(arguments).await,
    "code" => self.execute_code_tool(arguments, workspace).await,
    "task" | "tasks" => self.execute_task_tool(arguments, workspace).await,
    _ => ToolResult::Skipped(format!("Unknown tool: {}", name)),
};
```

**Missing:** Cases for `"screenshot"` and `"screen_record"` tools.

**Impact:** LLMs cannot execute these tools even though they're registered.

---

### 2. Tool Schemas (CRITICAL)

**File:** `crates/gestura-core/src/tools/schemas.rs`  
**Function:** `schema_for_tool()` at line 39

**Current:** File has 326 lines, schemas exist for shell, file, git, web, code, task tools.

**Missing:** Complete JSON schemas for:
- `screenshot` tool (OpenAI and Anthropic formats)
- `screen_record` tool (OpenAI and Anthropic formats)

**Impact:** LLMs don't know how to call these tools (no parameter definitions).

---

### 3. CLI Commands (HIGH PRIORITY)

**Files to create:**
- `crates/gestura-cli/src/commands/tools/screen.rs` (new file)
- Modify `crates/gestura-cli/src/commands/tools/mod.rs` to add Screen category
- Modify `crates/gestura-cli/src/main.rs` to add CLI argument parsing

**Pattern:** Follow existing tools (file.rs, git.rs, web.rs)

**Impact:** No way to test or use screen tools from CLI.

---

### 4. GUI Image Display (MEDIUM PRIORITY)

**File:** `crates/gestura-gui/frontend/src/components/ChatPanel.tsx`  
**Function:** `renderFormattedContent()` at line 817

**Current:** Uses ReactMarkdown for text/code rendering. Tool results shown as JSON text.

**Missing:** Image rendering for screenshot results.

**Impact:** Screenshots are saved but not displayed in chat UI.

---

## Implementation Plan

### Phase 1: Core Integration (Required for LLM usage)

#### Task 1.1: Add Pipeline Execution
**File:** `crates/gestura-core/src/pipeline/mod.rs`

Add new async function `execute_screen_tool()` following the pattern of `execute_file_tool()`:

```rust
async fn execute_screen_tool(&self, arguments: &str, workspace: Option<&SessionWorkspace>) -> ToolResult {
    use crate::tools::screen_async;
    
    match serde_json::from_str::<serde_json::Value>(arguments) {
        Ok(args) => {
            let operation = args.get("operation").and_then(|v| v.as_str()).unwrap_or("screenshot");
            
            match operation {
                "screenshot" => {
                    let output_path = args.get("output_path").and_then(|v| v.as_str())
                        .ok_or_else(|| "Missing 'output_path'")?;
                    
                    // Resolve path within workspace if set
                    let resolved_path = if let Some(ws) = workspace {
                        ws.resolve_path_for_write(Path::new(output_path))?
                            .to_string_lossy().to_string()
                    } else {
                        output_path.to_string()
                    };
                    
                    let region = args.get("region").and_then(parse_region);
                    let display = args.get("display").and_then(|v| v.as_u64()).map(|d| d as u32);
                    
                    match screen_async::screenshot(&resolved_path, region, display).await {
                        Ok(result) => ToolResult::Success(result),
                        Err(e) => ToolResult::Error(e.to_string()),
                    }
                }
                "start_recording" => { /* similar pattern */ }
                "stop_recording" => { /* similar pattern */ }
                _ => ToolResult::Error(format!("Unknown screen operation: {}", operation)),
            }
        }
        Err(e) => ToolResult::Error(format!("Invalid arguments: {}", e)),
    }
}
```

Then add to dispatcher (line 2471):
```rust
"screenshot" | "screen_record" | "screen" => self.execute_screen_tool(arguments, workspace).await,
```

**Estimated effort:** 2-3 hours
**Testing:** Unit test with mock arguments

---

#### Task 1.2: Add Tool Schemas
**File:** `crates/gestura-core/src/tools/schemas.rs`

Add to `schema_for_tool()` function (around line 200):

```rust
"screenshot" => (
    summary,
    serde_json::json!({
        "type": "object",
        "properties": {
            "output_path": {
                "type": "string",
                "description": "Path where screenshot will be saved (e.g., './screenshot.png')"
            },
            "region": {
                "type": "object",
                "description": "Optional region to capture (x, y, width, height)",
                "properties": {
                    "x": {"type": "integer"},
                    "y": {"type": "integer"},
                    "width": {"type": "integer"},
                    "height": {"type": "integer"}
                }
            },
            "display": {
                "type": "integer",
                "description": "Display number to capture (optional, default: primary display)"
            }
        },
        "required": ["output_path"],
        "additionalProperties": false
    }),
),
"screen_record" => (
    summary,
    serde_json::json!({
        "type": "object",
        "properties": {
            "operation": {
                "type": "string",
                "description": "Recording operation: 'start' or 'stop'",
                "enum": ["start", "stop"]
            },
            "output_path": {
                "type": "string",
                "description": "Path where recording will be saved (required for 'start')"
            },
            "recording_id": {
                "type": "string",
                "description": "Recording ID to stop (required for 'stop')"
            },
            "region": {
                "type": "object",
                "description": "Optional region to record (x, y, width, height)",
                "properties": {
                    "x": {"type": "integer"},
                    "y": {"type": "integer"},
                    "width": {"type": "integer"},
                    "height": {"type": "integer"}
                }
            },
            "display": {
                "type": "integer",
                "description": "Display number to record (optional)"
            }
        },
        "required": ["operation"],
        "additionalProperties": false
    }),
),
```

**Estimated effort:** 1 hour
**Testing:** Verify schemas are included in LLM prompts

---

### Phase 2: CLI Integration (For testing and manual use)

#### Task 2.1: Create CLI Screen Commands
**File:** `crates/gestura-cli/src/commands/tools/screen.rs` (NEW)

```rust
//! Screen capture tool
//!
//! Provides screen operations:
//! - capture: Take a screenshot
//! - record-start: Start screen recording
//! - record-stop: Stop screen recording

use super::super::Result;
use colored::Colorize;
use gestura_core::tools::screen::ScreenTools;
use std::path::PathBuf;
use std::sync::OnceLock;

static SCREEN_TOOLS: OnceLock<ScreenTools> = OnceLock::new();

fn get_screen_tools() -> &'static ScreenTools {
    SCREEN_TOOLS.get_or_init(ScreenTools::new)
}

pub enum ScreenSubcommand {
    Capture {
        path: PathBuf,
        region: Option<String>, // Format: "x,y,width,height"
        display: Option<u32>,
    },
    RecordStart {
        path: PathBuf,
        region: Option<String>,
        display: Option<u32>,
    },
    RecordStop {
        recording_id: String,
    },
}

pub fn run(cmd: ScreenSubcommand) -> Result<()> {
    match cmd {
        ScreenSubcommand::Capture { path, region, display } => {
            run_capture(&path, region.as_deref(), display)
        }
        ScreenSubcommand::RecordStart { path, region, display } => {
            run_record_start(&path, region.as_deref(), display)
        }
        ScreenSubcommand::RecordStop { recording_id } => {
            run_record_stop(&recording_id)
        }
    }
}

fn run_capture(path: &PathBuf, region: Option<&str>, display: Option<u32>) -> Result<()> {
    println!("{} screenshot to {}", "Capturing".bold(), path.display().to_string().cyan());

    let region_tuple = region.and_then(parse_region);
    let result = get_screen_tools().screenshot(path, region_tuple, display)?;

    println!("{} Screenshot saved", "✓".green());
    println!("  Path: {}", result.path.display().to_string().cyan());
    println!("  Size: {}x{}", result.width.unwrap_or(0), result.height.unwrap_or(0));
    println!("  Format: {}", result.format);
    println!("  File size: {} bytes", result.file_size_bytes);

    Ok(())
}

fn parse_region(s: &str) -> Option<(u32, u32, u32, u32)> {
    let parts: Vec<&str> = s.split(',').collect();
    if parts.len() == 4 {
        Some((
            parts[0].parse().ok()?,
            parts[1].parse().ok()?,
            parts[2].parse().ok()?,
            parts[3].parse().ok()?,
        ))
    } else {
        None
    }
}

// ... similar for record_start and record_stop
```

**Estimated effort:** 3-4 hours
**Testing:** Manual CLI testing on all platforms

---

#### Task 2.2: Update CLI Routing
**File:** `crates/gestura-cli/src/commands/tools/mod.rs`

Add to module declarations (line 16):
```rust
pub mod screen;
```

Add to `ToolsCategory` enum (line 28):
```rust
Screen(screen::ScreenSubcommand),
```

**File:** `crates/gestura-cli/src/main.rs`

Add to `ToolsAction` enum (around line 550):
```rust
/// Screen capture and recording
Screen {
    #[command(subcommand)]
    action: ScreenToolAction,
},
```

Add new enum:
```rust
#[derive(Subcommand)]
enum ScreenToolAction {
    /// Capture a screenshot
    Capture {
        /// Output path
        path: std::path::PathBuf,
        /// Region to capture (x,y,width,height)
        #[arg(short, long)]
        region: Option<String>,
        /// Display number
        #[arg(short, long)]
        display: Option<u32>,
    },
    /// Start screen recording
    RecordStart {
        /// Output path
        path: std::path::PathBuf,
        /// Region to record (x,y,width,height)
        #[arg(short, long)]
        region: Option<String>,
        /// Display number
        #[arg(short, long)]
        display: Option<u32>,
    },
    /// Stop screen recording
    RecordStop {
        /// Recording ID
        recording_id: String,
    },
}
```

**Estimated effort:** 1-2 hours
**Testing:** `gestura tools screen --help`

---

### Phase 3: GUI Integration (For visual feedback)

#### Task 3.1: Add Image Display in ChatPanel
**File:** `crates/gestura-gui/frontend/src/components/ChatPanel.tsx`

Modify `renderFormattedContent()` to detect and render images:

```typescript
const renderFormattedContent = (content: string) => {
  // Try to parse as JSON first (for tool results)
  try {
    const json = JSON.parse(content);

    // Check if it's a screenshot result
    if (json.path && json.format && (json.format === 'png' || json.format === 'jpg')) {
      return renderScreenshotResult(json);
    }
  } catch (e) {
    // Not JSON, continue with markdown rendering
  }

  return (
    <ReactMarkdown components={{ /* existing components */ }}>
      {content}
    </ReactMarkdown>
  );
};

const renderScreenshotResult = (result: any) => {
  const { path, width, height, file_size_bytes, timestamp } = result;

  return (
    <div className="screenshot-result">
      <div className="screenshot-header">
        <span className="screenshot-icon">📸</span>
        <span className="screenshot-title">Screenshot</span>
      </div>
      <img
        src={`asset://localhost/${path}`}
        alt="Screenshot"
        className="screenshot-image"
        onClick={() => window.open(`asset://localhost/${path}`)}
      />
      <div className="screenshot-info">
        <span>{width}×{height}</span>
        <span>{formatBytes(file_size_bytes)}</span>
        <span>{new Date(timestamp).toLocaleString()}</span>
      </div>
    </div>
  );
};

const formatBytes = (bytes: number): string => {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
};
```

**File:** `crates/gestura-gui/frontend/src/App.css`

Add CSS for screenshot display:

```css
.screenshot-result {
  border: 1px solid var(--border);
  border-radius: 8px;
  padding: 1rem;
  margin: 0.5rem 0;
  background: var(--surface);
}

.screenshot-header {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  margin-bottom: 0.75rem;
  font-weight: 600;
}

.screenshot-image {
  max-width: 100%;
  border-radius: 4px;
  cursor: pointer;
  transition: transform 0.2s;
}

.screenshot-image:hover {
  transform: scale(1.02);
}

.screenshot-info {
  display: flex;
  gap: 1rem;
  margin-top: 0.5rem;
  font-size: 0.85rem;
  color: var(--text-secondary);
}
```

**Estimated effort:** 2-3 hours
**Testing:** Take screenshot via LLM, verify image displays in chat

---

## Test Plan

### Test Scenario 1: Basic Screenshot via LLM

**Prompt:** "Take a screenshot and save it to /tmp/test.png"

**Expected LLM behavior:**
1. LLM receives tool schemas in system prompt
2. LLM decides to use `screenshot` tool
3. LLM emits tool call: `{"output_path": "/tmp/test.png"}`
4. Pipeline executes tool via dispatcher
5. Tool returns JSON result with path, dimensions, file size
6. LLM incorporates result in response
7. GUI displays image inline
8. CLI shows file path and metadata

**Verification:**
- [ ] File exists at `/tmp/test.png`
- [ ] JSON result contains valid metadata
- [ ] GUI shows image preview
- [ ] CLI shows colored success message

---

### Test Scenario 2: Region Screenshot

**Prompt:** "Capture the top-left 800x600 region of my screen and save to ./region.png"

**Expected tool call:**
```json
{
  "output_path": "./region.png",
  "region": {"x": 0, "y": 0, "width": 800, "height": 600}
}
```

**Verification:**
- [ ] Screenshot is exactly 800x600 pixels
- [ ] File saved in workspace directory
- [ ] Metadata shows correct dimensions

---

### Test Scenario 3: Screen Recording Start/Stop

**Prompt:** "Start recording my screen to ./demo.mp4"

**Expected tool call 1:**
```json
{
  "operation": "start",
  "output_path": "./demo.mp4"
}
```

**Expected result:**
```json
{
  "recording_id": "rec_abc123",
  "output_path": "/full/path/to/demo.mp4",
  "started_at": "2026-02-03T12:00:00Z"
}
```

**Follow-up prompt:** "Stop the recording"

**Expected tool call 2:**
```json
{
  "operation": "stop",
  "recording_id": "rec_abc123"
}
```

**Verification:**
- [ ] Recording process starts in background
- [ ] Recording ID returned and tracked
- [ ] Stop command terminates recording gracefully
- [ ] Video file is playable
- [ ] Duration matches expected time

---

### Test Scenario 4: Screenshot for Analysis

**Prompt:** "Take a screenshot of my screen and describe what you see"

**Expected flow:**
1. LLM calls screenshot tool
2. Screenshot saved to temp file
3. LLM receives file path in result
4. LLM describes the screenshot (based on file metadata, not actual image analysis)

**Note:** Actual image analysis requires vision model integration (future enhancement).

**Verification:**
- [ ] Screenshot taken successfully
- [ ] LLM acknowledges screenshot in response
- [ ] File path included in conversation

---

## Development Workflow

### Step 1: Implement Pipeline Integration
```bash
# Edit pipeline/mod.rs
code crates/gestura-core/src/pipeline/mod.rs

# Run tests
cargo test --package gestura-core execute_screen_tool

# Run clippy
cargo clippy --package gestura-core --all-features -- -D warnings
```

### Step 2: Add Tool Schemas
```bash
# Edit schemas.rs
code crates/gestura-core/src/tools/schemas.rs

# Verify schemas are generated
cargo run -p gestura-cli -- mcp tools | grep screenshot
```

### Step 3: Test with Dev LLM
```bash
# Start GUI in dev mode
cargo tauri dev

# Or use CLI chat
cargo run -p gestura-cli -- chat

# Test prompt
> "Take a screenshot and save it to /tmp/test.png"
```

### Step 4: Implement CLI Commands
```bash
# Create screen.rs
code crates/gestura-cli/src/commands/tools/screen.rs

# Test CLI
cargo run -p gestura-cli -- tools screen capture /tmp/cli-test.png
cargo run -p gestura-cli -- tools screen record-start /tmp/recording.mp4
cargo run -p gestura-cli -- tools screen record-stop <recording_id>
```

### Step 5: Add GUI Image Display
```bash
# Edit ChatPanel.tsx
code crates/gestura-gui/frontend/src/components/ChatPanel.tsx

# Edit App.css
code crates/gestura-gui/frontend/src/App.css

# Test in GUI
cargo tauri dev
```

---

## Success Criteria

### Minimum Viable Integration (Phase 1)
- [x] Core implementation complete
- [ ] Pipeline dispatcher includes screen tools
- [ ] Tool schemas defined for LLM providers
- [ ] LLM can successfully call screenshot tool
- [ ] Tool results returned as JSON

### Full Integration (Phase 2 + 3)
- [ ] CLI commands work on all platforms
- [ ] GUI displays screenshots inline
- [ ] Screen recording start/stop works
- [ ] All quality gates pass (fmt, clippy, tests)
- [ ] Documentation updated

---

## Risk Assessment

### High Risk
1. **Platform-specific failures:** Screenshot/recording commands may fail on some Linux distributions or Windows versions
   - **Mitigation:** Extensive platform testing, fallback error messages

2. **Permission issues:** Screen recording requires special permissions on macOS and some Linux setups
   - **Mitigation:** Clear error messages, permission check before execution

### Medium Risk
1. **File path resolution:** Workspace sandboxing may reject certain paths
   - **Mitigation:** Use `resolve_path_for_write()` consistently

2. **Large file sizes:** Screenshots/recordings can be large, may cause UI lag
   - **Mitigation:** Thumbnail generation, lazy loading in GUI

### Low Risk
1. **Schema validation:** LLM may send malformed arguments
   - **Mitigation:** Robust error handling in execute_screen_tool()

---

## Timeline Estimate

| Phase | Tasks | Estimated Time |
|-------|-------|----------------|
| Phase 1: Core Integration | Pipeline + Schemas | 4-5 hours |
| Phase 2: CLI Integration | CLI commands + routing | 5-6 hours |
| Phase 3: GUI Integration | Image display + CSS | 3-4 hours |
| Testing & Documentation | All platforms | 3-4 hours |
| **Total** | | **15-19 hours** |

---

## Next Steps

1. **Immediate:** Implement Phase 1 (pipeline integration + schemas)
2. **Test:** Verify LLM can call tools with `cargo tauri dev`
3. **Iterate:** Add CLI commands for manual testing
4. **Polish:** Add GUI image display
5. **Document:** Update README and DEVELOPER_GUIDE

---

## Appendix: Key Files Reference

| File | Purpose | Lines | Status |
|------|---------|-------|--------|
| `crates/gestura-core/src/tools/screen.rs` | Core implementation | 746 | ✅ Complete |
| `crates/gestura-core/src/tools/mod.rs` | Async wrappers | 84-202 | ✅ Complete |
| `crates/gestura-core/src/tools/registry.rs` | Tool definitions | 155-194 | ✅ Complete |
| `crates/gestura-core/src/pipeline/mod.rs` | Tool execution | 2457-2487 | ❌ Missing dispatcher |
| `crates/gestura-core/src/tools/schemas.rs` | JSON schemas | 39-326 | ❌ Missing schemas |
| `crates/gestura-cli/src/commands/tools/screen.rs` | CLI commands | N/A | ❌ Not created |
| `crates/gestura-gui/frontend/src/components/ChatPanel.tsx` | GUI display | 817-859 | ❌ No image support |


