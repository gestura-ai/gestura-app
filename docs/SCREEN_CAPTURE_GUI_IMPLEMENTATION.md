# Screen Capture GUI Implementation Plan

## Overview
This document outlines the implementation of screenshot and screen recording as **first-class features** in the Gestura GUI, following Core-First Architecture principles.

## Status: Backend Complete ✅

### Completed (Backend)

#### 1. Tauri Commands Added (`crates/gestura-gui/src/api.rs`)

Three new Tauri commands expose core screen capture functionality to the frontend:

```rust
#[tauri::command]
pub async fn capture_screenshot(
    output_path: String,
    region: Option<(u32, u32, u32, u32)>,
    display: Option<u32>,
) -> Result<gestura_core::tools::screen::ScreenshotResult, String>

#[tauri::command]
pub async fn start_screen_recording(
    output_path: String,
    region: Option<(u32, u32, u32, u32)>,
    display: Option<u32>,
) -> Result<gestura_core::tools::screen::RecordingStartResult, String>

#[tauri::command]
pub async fn stop_screen_recording(
    recording_id: String,
) -> Result<gestura_core::tools::screen::RecordingStopResult, String>
```

**Return Types:**
- `ScreenshotResult`: path, width, height, format, timestamp, file_size_bytes
- `RecordingStartResult`: recording_id, output_path, started_at
- `RecordingStopResult`: recording_id, path, duration_secs, file_size_bytes, format

#### 2. Commands Registered (`crates/gestura-gui/src/main.rs`)

All three commands registered in the Tauri invoke handler:
- `gestura_gui::api::capture_screenshot`
- `gestura_gui::api::start_screen_recording`
- `gestura_gui::api::stop_screen_recording`

#### 3. Core Implementation (`gestura-core/src/tools/screen.rs`)

Cross-platform implementation already exists:
- **macOS**: `screencapture`, `ffmpeg` with AVFoundation
- **Linux**: `grim`/`scrot`, `wf-recorder`/`ffmpeg`
- **Windows**: PowerShell, `ffmpeg` with gdigrab

#### 4. Permission Handling

Existing permission commands work for screen recording:
- `check_permission("screen_recording")`
- `request_permission("screen_recording")`

---

## Next Steps: Frontend Implementation

### Option A: Dedicated Screen Capture Panel (Recommended)

Create a new panel similar to `ToolsPanel` and `WorkflowsPanel`:

**File**: `crates/gestura-gui/frontend/src/components/ScreenCapturePanel.tsx`

**Features**:
- Quick capture button (full screen, default settings)
- Advanced options (region selector, display selector)
- Recording controls (start/stop, status indicator, timer)
- Screenshot gallery (recent captures with thumbnails)
- Integration with agent (attach screenshot to message, describe screenshot)

**UI Layout**:
```
┌─────────────────────────────────────┐
│ Screen Capture                      │
├─────────────────────────────────────┤
│ [📸 Quick Screenshot]  [🎥 Record]  │
│                                     │
│ Advanced Options:                   │
│ ☐ Capture region                   │
│ ☐ Select display                   │
│                                     │
│ Recent Captures:                    │
│ ┌───┐ ┌───┐ ┌───┐                 │
│ │img│ │img│ │img│                 │
│ └───┘ └───┘ └───┘                 │
└─────────────────────────────────────┘
```

### Option B: Toolbar Integration

Add screenshot/recording buttons to the main toolbar/header:

**File**: `crates/gestura-gui/frontend/src/App.tsx`

**Features**:
- Quick access buttons in header
- Minimal UI (just capture/record buttons)
- Status indicator when recording
- Notifications for completed captures

### Option C: Enhanced ToolsPanel

Add interactive controls to existing ToolsPanel for screenshot/recording tools:

**File**: `crates/gestura-gui/frontend/src/components/ToolsPanel.tsx`

**Features**:
- Special rendering for screenshot/screen_record tools
- "Execute" button that opens capture dialog
- Inline controls for region/display selection

---

## Implementation Checklist

### Backend ✅
- [x] Add Tauri commands to `api.rs`
- [x] Register commands in `main.rs`
- [x] Verify core implementation exists
- [x] Verify permission handling exists

### Frontend (TODO)
- [ ] Choose UI approach (Panel/Toolbar/ToolsPanel enhancement)
- [ ] Create TypeScript types for return values
- [ ] Implement UI component
- [ ] Add screenshot preview/gallery
- [ ] Add recording status indicator
- [ ] Handle permissions (request screen recording permission)
- [ ] Add error handling and user feedback
- [ ] Integrate with agent (optional: attach screenshots to messages)
- [ ] Add keyboard shortcuts (optional)

### Testing (TODO)
- [ ] Test screenshot capture (full screen)
- [ ] Test screenshot capture (region)
- [ ] Test screenshot capture (specific display)
- [ ] Test recording start/stop
- [ ] Test permission handling
- [ ] Test cross-platform (macOS, Linux, Windows)

---

## Design Decision Needed

**Question**: Where should the screenshot/recording UI live in the GUI?

1. **Dedicated Panel** (like Tools, Workflows, Settings)
   - Pros: Full-featured, room for gallery and advanced options
   - Cons: Requires navigation to access

2. **Toolbar/Header** (quick access buttons)
   - Pros: Always visible, quick access
   - Cons: Limited space for advanced features

3. **Enhanced ToolsPanel** (interactive tool execution)
   - Pros: Logical location, reuses existing panel
   - Cons: May clutter tools list

**Recommendation**: Start with **Option A (Dedicated Panel)** for full feature set, then add **Option B (Toolbar buttons)** for quick access.

