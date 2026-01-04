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

## API Commands

### Simulator Management

- `get_simulators()`: Get all connected simulators
- `scan_for_simulators()`: Scan for available simulators
- `reset_simulator(device_id)`: Reset a specific simulator
- `get_simulator_health(device_id)`: Get simulator health status
- `get_simulator_logs(device_id)`: Get connection logs

### Testing and Debugging

- `send_test_haptic(device_id, pattern_type)`: Send test haptic patterns
- `run_simulator_test(device_id)`: Run comprehensive test suite
- `get_simulator_metrics(device_id)`: Get performance metrics
- `start_health_monitoring(device_id, interval)`: Start health monitoring
- `stop_health_monitoring(device_id)`: Stop health monitoring

### Configuration

- `is_developer_mode_enabled()`: Check developer mode status
- `toggle_developer_mode(enabled)`: Enable/disable developer mode
- `toggle_simulator_support(enabled)`: Enable/disable simulator support
- `get_simulator_config()`: Get simulator configuration
- `update_simulator_config(config)`: Update simulator settings

## Test Haptic Patterns

The simulator supports specialized test patterns for development and validation:

### Pattern Types

1. **Connectivity Test**: Basic connectivity validation
2. **Latency Test**: Measure communication latency
3. **Intensity Test**: Test intensity range (0.1 to 1.0)
4. **Duration Test**: Test various duration patterns
5. **Complex Pattern**: Multi-step intensity/duration sequences

### Usage Example

```typescript
// Send a latency test pattern
await invoke('send_test_haptic', {
  deviceId: 'simulator-001',
  patternType: 'latency'
});

// Run comprehensive test suite
const results = await invoke('run_simulator_test', {
  deviceId: 'simulator-001'
});
```

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

```typescript
await invoke('toggle_developer_mode', { enabled: true });
```

### 2. Scan for Simulators

```typescript
const simulators = await invoke('scan_for_simulators');
console.log('Found simulators:', simulators);
```

### 3. Connect and Test

```typescript
// Get simulator status
const status = await invoke('get_simulator_health', { 
  deviceId: 'simulator-001' 
});

// Send test haptic
await invoke('send_test_haptic', {
  deviceId: 'simulator-001',
  patternType: 'connectivity'
});

// Run comprehensive tests
const testResults = await invoke('run_simulator_test', {
  deviceId: 'simulator-001'
});
```

### 4. Monitor Performance

```typescript
// Get performance metrics
const metrics = await invoke('get_simulator_metrics', {
  deviceId: 'simulator-001'
});

// Start health monitoring
await invoke('start_health_monitoring', {
  deviceId: 'simulator-001',
  intervalSeconds: 30
});
```

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

```typescript
// Get detailed logs
const logs = await invoke('get_simulator_logs', {
  deviceId: 'simulator-001'
});

// Reset simulator
await invoke('reset_simulator', {
  deviceId: 'simulator-001'
});

// Check health
const health = await invoke('get_simulator_health', {
  deviceId: 'simulator-001'
});
```

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
