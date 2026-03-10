# Haptic Harmony Ring Simulator Support

This document describes the comprehensive simulator support features in Gestura.app, designed to enhance development and testing workflows for Haptic Harmony Ring integration.

## Overview

Gestura.app provides extensive support for Haptic Harmony Ring simulators, enabling developers to test and develop ring integration features without requiring physical hardware. The simulator support includes auto-discovery, health monitoring, debugging tools, and comprehensive testing infrastructure.

## Features

### 1. Simulator Device Detection

- **Automatic Recognition**: Detects devices with "Simulator" in the name
- **Visual Indicators**: UI clearly distinguishes simulators from real rings
- **Developer Mode**: Toggle for simulator-specific features and debugging tools
- **Auto-Discovery**: Automatically discovers simulators on localhost

### 2. Enhanced Debugging Support

- **Detailed BLE Logs**: Comprehensive connection and communication logs
- **Status Reporting**: Real-time simulator health and status monitoring
- **Test Haptic Patterns**: Send specialized test patterns to simulators
- **Performance Metrics**: Latency, packet loss, and throughput monitoring

### 3. Development Workflow Integration

- **Localhost Discovery**: Auto-discovery on configurable port ranges
- **Reset/Reconnect**: Easy simulator reset and reconnection functionality
- **Configuration Options**: Simulator-specific settings and preferences
- **Health Monitoring**: Automated health checks with configurable intervals

### 4. Testing Infrastructure

- **Automated Tests**: Connect and test simulator functionality
- **Health Checks**: Continuous monitoring of simulator status
- **Performance Metrics**: Detailed performance and reliability metrics
- **Test Patterns**: Comprehensive haptic pattern testing suite

## Configuration

### Developer Settings

The simulator support is configured through the `DeveloperSettings` in the application configuration:

```json
{
  "developer": {
    "developer_mode": false,
    "enable_simulators": true,
    "auto_discover_simulators": true,
    "verbose_ble_logging": false,
    "simulator": {
      "device_name_pattern": "Haptic Harmony Ring Simulator",
      "auto_connect": true,
      "health_check_interval": 30,
      "enable_metrics": true,
      "discovery_port_range": [8080, 8090]
    }
  }
}
```

### Configuration Options

- **developer_mode**: Enable developer-specific features and UI elements
- **enable_simulators**: Master toggle for simulator support
- **auto_discover_simulators**: Automatically scan for simulators on startup
- **verbose_ble_logging**: Enable detailed BLE communication logs
- **device_name_pattern**: Pattern to match simulator device names
- **auto_connect**: Automatically connect to discovered simulators
- **health_check_interval**: Interval (seconds) for health monitoring
- **enable_metrics**: Collect and display performance metrics
- **discovery_port_range**: Port range for localhost discovery

## Canonical API / IPC Reference

This document focuses on simulator workflow, configuration, and testing
guidance.

For the exact GUI IPC contract and command inventory, use:

- `docs/IPC_CONTRACTS_GESTURA_GUI.md`

For the owning Rust types and implementation surface, use generated docs for the
relevant crates/modules rather than treating this file as an API reference.

## Test Haptic Patterns

The simulator supports specialized test patterns for development and validation:

### Pattern Types

1. **Connectivity Test**: Basic connectivity validation
2. **Latency Test**: Measure communication latency
3. **Intensity Test**: Test intensity range (0.1 to 1.0)
4. **Duration Test**: Test various duration patterns
5. **Complex Pattern**: Multi-step intensity/duration sequences

### Usage Example

Use the simulator panel or the documented GUI IPC surface to:

- send a named test haptic pattern to a simulator
- run the simulator test suite for a selected device
- inspect the returned metrics/results in the UI

For exact command names and payload shapes, use `docs/IPC_CONTRACTS_GESTURA_GUI.md`.

## UI Integration

### Simulator Panel

The Simulator Panel provides a comprehensive interface for managing and testing simulators:

- **Simulator List**: View all connected simulators with status indicators
- **Test Controls**: Send various test haptic patterns
- **Performance Metrics**: Real-time latency, packet loss, and throughput
- **Connection Logs**: Detailed BLE communication logs
- **Health Monitoring**: Continuous status monitoring

### Status Indicators

- 🟢 **Healthy**: Simulator is functioning normally
- 🟡 **Degraded**: Simulator has performance issues
- 🔴 **Offline**: Simulator is not responding
- ❌ **Error**: Simulator has encountered an error

## Development Workflow

### 1. Enable Developer Mode

Enable developer mode from the UI before attempting simulator-specific actions.

If you are wiring or debugging the frontend implementation, use
`docs/IPC_CONTRACTS_GESTURA_GUI.md` for the exact command contract.

### 2. Scan for Simulators

Run a simulator scan from the UI and confirm the discovered devices appear in the
simulator panel.

### 3. Connect and Test

For a selected simulator:

- verify current health/status
- send a connectivity or latency test pattern
- run the broader simulator test suite and inspect the results

### 4. Monitor Performance

Use the simulator panel to review performance metrics and enable periodic health
monitoring during longer development sessions.

## Simulator Implementation

### Device Identification

Simulators are identified by:
- Device name containing "Simulator"
- Identical BLE service UUIDs as real rings
- All standard BLE characteristics implemented
- Special simulator status reporting

### Technical Details

- **Service UUIDs**: Identical to real Haptic Harmony Rings
- **Device Name**: "Haptic Harmony Ring Simulator"
- **Characteristics**: All standard ring characteristics supported
- **Gestures**: Same gesture types as real hardware
- **Haptic Patterns**: Full haptic pattern support

## Troubleshooting

### Common Issues

1. **Simulator Not Found**
   - Ensure simulator is running and advertising
   - Check device name matches pattern
   - Verify BLE adapter is working

2. **Connection Failed**
   - Reset simulator using reset command
   - Check BLE permissions
   - Verify simulator is not connected to another app

3. **Test Patterns Not Working**
   - Ensure developer mode is enabled
   - Check simulator health status
   - Verify haptic service is available

### Debug Commands

For debugging, focus on three operator actions:

- inspect detailed simulator logs
- reset the simulator before retrying a test sequence
- re-check health/status after reset or reconnection attempts

Use `docs/IPC_CONTRACTS_GESTURA_GUI.md` only when you need the exact frontend ↔
Tauri command contract.

## Best Practices

1. **Enable Developer Mode**: Always enable developer mode when working with simulators
2. **Monitor Health**: Use health monitoring for long-running tests
3. **Reset Regularly**: Reset simulators between test sessions
4. **Check Logs**: Review connection logs for debugging issues
5. **Use Test Patterns**: Leverage specialized test patterns for validation

## Integration with Real Hardware

The simulator support is designed to be seamlessly compatible with real Haptic Harmony Ring hardware:

- **Same API**: Identical commands work with both simulators and real rings
- **Transparent Switching**: Switch between simulator and real hardware without code changes
- **Feature Parity**: All features available on real hardware work with simulators
- **Testing Validation**: Tests written for simulators work with real hardware

This comprehensive simulator support enables efficient development and testing of Haptic Harmony Ring integration without requiring physical hardware, while ensuring seamless transition to real devices when available.
