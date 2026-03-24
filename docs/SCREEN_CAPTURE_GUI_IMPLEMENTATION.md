# Screen Capture GUI Implementation Plan

## Overview
This document outlines the implementation of screenshot and screen recording as **first-class features** in the Gestura GUI, following Core-First Architecture principles.

## Status

This file is an **implementation-planning document**, not the canonical source
for exact Tauri command signatures, current registration state, or Rust type
definitions.

For the current source of truth, use:

- `docs/IPC_CONTRACTS_GESTURA_GUI.md`
- `crates/gestura-gui/src/main.rs`
- `crates/gestura-gui/src/api.rs`
- generated docs/source for the owning Rust modules

## Backend Status: Complete ✅

### Completed (Backend)

#### 1. Frontend/IPC Surface Exists

Screen-capture-related GUI IPC handlers and registration points exist in the GUI
host. Use the IPC guide and current source files rather than this doc for exact
command names or signatures.

#### 2. Core Implementation Exists

Cross-platform implementation already exists:
- **macOS**: `screencapture`, `ffmpeg` with AVFoundation
- **Linux**: `grim`/`scrot`, `wf-recorder`/`ffmpeg`
- **Windows**: PowerShell, `ffmpeg` with gdigrab

#### 3. Permission Handling Exists

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

### Frontend Implementation Plan
- [ ] Choose UI approach (Panel/Toolbar/ToolsPanel enhancement)
- [ ] Create TypeScript types for return values
- [ ] Implement UI component
- [ ] Add screenshot preview/gallery
- [ ] Add recording status indicator
- [ ] Handle permissions (request screen recording permission)
- [ ] Add error handling and user feedback
- [ ] Integrate with agent (optional: attach screenshots to messages)
- [ ] Add keyboard shortcuts (optional)

### Testing Checklist
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

