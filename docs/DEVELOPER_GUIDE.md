# Gestura.app Developer Guide

## Table of Contents

1. [Getting Started](#getting-started)
2. [Development Environment](#development-environment)
3. [Architecture Overview](#architecture-overview)
4. [Plugin Development](#plugin-development)
5. [Scripting](#scripting)
6. [API Integration](#api-integration)
7. [Custom Gestures](#custom-gestures)
8. [Third-Party Integrations](#third-party-integrations)
9. [Testing](#testing)
10. [Deployment](#deployment)

## Getting Started

Welcome to Gestura.app development! This guide will help you build plugins, scripts, and integrations for the Gestura ecosystem.

### Prerequisites

- **Programming Experience**: Familiarity with at least one supported language
- **Development Tools**: Code editor, terminal, Git
- **Gestura.app**: Latest version installed
- **API Key**: Developer API key (get from Settings → Developer)

### Supported Languages

- **Rust**: Core application development
- **JavaScript/TypeScript**: Frontend and plugins
- **Python**: Scripting and ML integrations
- **Lua**: Lightweight scripting
- **WebAssembly**: High-performance plugins

## Development Environment

### Setting Up

1. **Clone the repository**:
```bash
git clone https://github.com/gestura-ai/gestura-app.git
cd gestura-app
```

2. **Install dependencies**:
```bash
# Rust dependencies
cargo install tauri-cli
rustup target add wasm32-unknown-unknown

# Node.js dependencies
npm install

# Python dependencies (optional)
pip install gestura-sdk
```

3. **Development server**:
```bash
# Start development server
npm run tauri dev

# Or use just/make
just dev
make dev
```

### Project Structure

```
gestura-app/
├── src/                    # React frontend
├── src-tauri/             # Rust backend
│   ├── src/
│   │   ├── lib.rs         # Main library
│   │   ├── main.rs        # Application entry
│   │   └── modules/       # Feature modules
│   └── Cargo.toml
├── plugins/               # Plugin directory
├── scripts/               # User scripts
├── docs/                  # Documentation
├── tests/                 # Test files
└── package.json
```

### Build System

#### Using Just (Recommended)

```bash
# Development
just dev                   # Start dev server
just test                  # Run all tests
just lint                  # Run linters
just format               # Format code

# Building
just build                # Debug build
just build-release        # Release build
just package              # Create packages

# Platform-specific
just build-mac            # macOS build
just build-windows        # Windows build
just build-linux          # Linux build
```

#### Using Make (Alternative)

```bash
# Development
make dev                  # Start dev server
make test                 # Run tests
make clean                # Clean build artifacts

# Building
make build                # Debug build
make release              # Release build
make package              # Create packages
```

## Architecture Overview

### Core Components

```rust
// Main application structure
pub struct GesturaApp {
    voice_processor: VoiceProcessor,
    gesture_recognizer: GestureRecognizer,
    ring_manager: RingManager,
    plugin_manager: PluginManager,
    mcp_server: McpServer,
}
```

### Event System

```rust
// Event types
#[derive(Debug, Clone)]
pub enum GesturaEvent {
    VoiceRecognized { text: String, confidence: f32 },
    GestureDetected { gesture: String, confidence: f32 },
    RingConnected { device_id: String },
    PluginLoaded { plugin_id: String },
}

// Event handler trait
pub trait EventHandler {
    fn handle_event(&mut self, event: GesturaEvent) -> Result<(), Error>;
}
```

### Plugin Interface

```rust
// Plugin trait
pub trait Plugin {
    fn initialize(&mut self, config: PluginConfig) -> Result<(), Error>;
    fn handle_command(&mut self, command: &str, args: Value) -> Result<Value, Error>;
    fn handle_event(&mut self, event: GesturaEvent) -> Result<(), Error>;
    fn shutdown(&mut self) -> Result<(), Error>;
}
```

## Plugin Development

### Creating a Plugin

1. **Create plugin directory**:
```bash
mkdir plugins/my-plugin
cd plugins/my-plugin
```

2. **Create manifest** (`plugin.json`):
```json
{
  "id": "my-plugin",
  "name": "My Awesome Plugin",
  "version": "1.0.0",
  "description": "Does awesome things",
  "author": "Your Name",
  "license": "MIT",
  "entry_point": "main.js",
  "permissions": [
    "voice_control",
    "notifications"
  ],
  "dependencies": [],
  "supported_platforms": ["linux", "macos", "windows"]
}
```

3. **Create main file** (`main.js`):
```javascript
// Plugin main file
class MyPlugin {
    constructor() {
        this.name = "My Awesome Plugin";
    }

    initialize(config) {
        console.log("Plugin initialized with config:", config);
        return Promise.resolve();
    }

    handleCommand(command, args) {
        switch (command) {
            case "hello":
                return Promise.resolve({ message: "Hello from plugin!" });
            default:
                return Promise.reject(new Error(`Unknown command: ${command}`));
        }
    }

    handleEvent(event) {
        if (event.type === "voice_recognized") {
            console.log("Voice recognized:", event.text);
        }
        return Promise.resolve();
    }

    shutdown() {
        console.log("Plugin shutting down");
        return Promise.resolve();
    }
}

// Export plugin
module.exports = MyPlugin;
```

### Plugin Permissions

Available permissions:
- `voice_control`: Access voice recognition
- `gesture_control`: Access gesture recognition
- `ring_control`: Control haptic ring
- `notifications`: Send notifications
- `filesystem`: File system access
- `network`: Network access
- `system_commands`: Execute system commands
- `clipboard`: Clipboard access
- `window_management`: Control windows

### Plugin API

```javascript
// Voice API
gestura.voice.recognize(audioData, options)
gestura.voice.synthesize(text, options)
gestura.voice.setLanguage(language)

// Gesture API
gestura.gestures.recognize(sensorData)
gestura.gestures.createCustom(name, pattern)
gestura.gestures.onDetected(callback)

// Ring API
gestura.ring.sendHaptic(pattern, intensity)
gestura.ring.getStatus()
gestura.ring.onConnected(callback)

// System API
gestura.system.notify(title, message)
gestura.system.execute(command)
gestura.system.getClipboard()
gestura.system.setClipboard(text)

// Storage API
gestura.storage.get(key)
gestura.storage.set(key, value)
gestura.storage.delete(key)
```

## Scripting

### Lua Scripting

```lua
-- @name Weather Command
-- @description Get weather information
-- @author Your Name
-- @version 1.0.0
-- @permission network
-- @trigger voice:weather

function main(args)
    local location = args.location or "current"
    local weather = http.get("https://api.weather.com/v1/current?location=" .. location)
    
    if weather.status == 200 then
        local data = json.decode(weather.body)
        local message = string.format("Weather in %s: %s, %d°F", 
            data.location, data.condition, data.temperature)
        
        gestura.notify("Weather", message)
        gestura.speak(message)
        
        return { success = true, message = message }
    else
        return { success = false, error = "Failed to get weather" }
    end
end
```

### Python Scripting

```python
# @name Smart Home Control
# @description Control smart home devices
# @author Your Name
# @version 1.0.0
# @permission network
# @trigger gesture:circle

import requests
import json

def main(context):
    """Main script entry point"""
    gesture_data = context.get('gesture_data', {})
    
    if gesture_data.get('confidence', 0) > 0.8:
        # Turn on lights
        response = requests.post(
            'https://api.smarthome.com/lights/living-room/on',
            headers={'Authorization': f'Bearer {get_token()}'}
        )
        
        if response.status_code == 200:
            gestura.ring.send_haptic('success', 0.7)
            return {'success': True, 'action': 'lights_on'}
        else:
            gestura.ring.send_haptic('error', 0.5)
            return {'success': False, 'error': 'Failed to control lights'}
    
    return {'success': False, 'error': 'Low confidence gesture'}

def get_token():
    """Get authentication token"""
    return gestura.storage.get('smarthome_token')
```

### JavaScript Scripting

```javascript
// @name Productivity Booster
// @description Automate common tasks
// @author Your Name
// @version 1.0.0
// @permission system_commands
// @trigger voice:focus mode

async function main(context) {
    const { text } = context;
    
    if (text.includes('focus mode')) {
        // Close distracting applications
        await gestura.system.execute('pkill -f "Social Media App"');
        await gestura.system.execute('pkill -f "Game"');
        
        // Start focus timer
        const focusTime = 25 * 60 * 1000; // 25 minutes
        setTimeout(async () => {
            await gestura.notify('Focus Session', 'Time for a break!');
            await gestura.ring.sendHaptic('notification', 0.6);
        }, focusTime);
        
        // Set status
        await gestura.system.setClipboard('🎯 In focus mode');
        
        return {
            success: true,
            message: 'Focus mode activated for 25 minutes'
        };
    }
    
    return { success: false, error: 'Unknown command' };
}
```

## API Integration

### Using the SDK

#### JavaScript/Node.js

```javascript
const { GesturaSDK } = require('@gestura/sdk');

const client = new GesturaSDK({
    apiKey: 'your-api-key',
    baseUrl: 'https://api.gestura.app/v1'
});

// Voice recognition
async function recognizeVoice(audioFile) {
    try {
        const result = await client.voice.recognize({
            audioData: audioFile,
            language: 'en-US',
            model: 'medium'
        });
        
        console.log('Recognized text:', result.text);
        console.log('Confidence:', result.confidence);
        return result;
    } catch (error) {
        console.error('Recognition failed:', error);
    }
}

// Gesture recognition
async function recognizeGesture(sensorData) {
    try {
        const result = await client.gestures.recognize({
            sensorData: sensorData,
            userId: 'user123'
        });
        
        console.log('Recognized gesture:', result.gesture);
        return result;
    } catch (error) {
        console.error('Gesture recognition failed:', error);
    }
}
```

#### Python

```python
from gestura_sdk import GesturaClient

client = GesturaClient(api_key='your-api-key')

# Voice recognition
def recognize_voice(audio_file):
    try:
        result = client.voice.recognize(
            audio_data=audio_file,
            language='en-US',
            model='medium'
        )
        
        print(f'Recognized text: {result.text}')
        print(f'Confidence: {result.confidence}')
        return result
    except Exception as error:
        print(f'Recognition failed: {error}')

# Custom gesture creation
def create_custom_gesture(name, training_data):
    try:
        gesture = client.gestures.create_custom(
            name=name,
            description=f'Custom gesture: {name}',
            gesture_type='motion',
            user_id='user123'
        )
        
        # Train the gesture
        for sample in training_data:
            client.gestures.add_training_sample(
                gesture_id=gesture.id,
                sensor_data=sample
            )
        
        return gesture
    except Exception as error:
        print(f'Failed to create gesture: {error}')
```

### REST API Examples

#### Voice Recognition

```bash
curl -X POST https://api.gestura.app/v1/voice/recognize \
  -H "Authorization: Bearer YOUR_API_KEY" \
  -H "Content-Type: multipart/form-data" \
  -F "audio_data=@recording.wav" \
  -F "language=en-US" \
  -F "model=medium"
```

#### Gesture Recognition

```bash
curl -X POST https://api.gestura.app/v1/gestures/recognize \
  -H "Authorization: Bearer YOUR_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{
    "sensor_data": [
      {
        "timestamp_ms": 1640995200000,
        "accelerometer": [0.1, 0.2, 9.8],
        "gyroscope": [0.01, 0.02, 0.03],
        "magnetometer": [25.0, -15.0, 45.0]
      }
    ],
    "user_id": "user123"
  }'
```

## Custom Gestures

### Creating Custom Gestures

```rust
use gestura::gestures::{CustomGesture, GestureType, SensorReading};

// Create custom gesture
let gesture = CustomGesture {
    name: "My Custom Gesture".to_string(),
    gesture_type: GestureType::Motion,
    training_samples: Vec::new(),
    recognition_threshold: 0.8,
};

// Add training samples
for sample_data in training_data {
    let sample = GestureTrainingSample {
        sensor_data: sample_data,
        quality_score: calculate_quality(&sample_data),
        recorded_at: chrono::Utc::now(),
    };
    
    gesture.training_samples.push(sample);
}

// Train the gesture
let trained_gesture = gesture_manager.train_gesture(gesture).await?;
```

### Gesture Recognition Pipeline

```rust
// Gesture recognition pipeline
pub struct GestureRecognitionPipeline {
    feature_extractor: FeatureExtractor,
    classifier: GestureClassifier,
    post_processor: PostProcessor,
}

impl GestureRecognitionPipeline {
    pub async fn recognize(&self, sensor_data: &[SensorReading]) -> Result<GestureResult, Error> {
        // Extract features
        let features = self.feature_extractor.extract(sensor_data)?;
        
        // Classify gesture
        let classification = self.classifier.classify(&features)?;
        
        // Post-process results
        let result = self.post_processor.process(classification)?;
        
        Ok(result)
    }
}
```

## Third-Party Integrations

### Creating Integrations

```rust
use gestura::integrations::{Integration, IntegrationType, IntegrationConfig};

// Create Slack integration
let slack_integration = Integration {
    name: "Slack".to_string(),
    integration_type: IntegrationType::RestApi,
    config: IntegrationConfig {
        endpoint_url: Some("https://slack.com/api".to_string()),
        timeout_seconds: 30,
        headers: HashMap::from([
            ("Content-Type".to_string(), "application/json".to_string()),
        ]),
        ..Default::default()
    },
    credentials: IntegrationCredentials {
        auth_type: AuthType::BearerToken,
        encrypted_data: encrypt_token(slack_token)?,
    },
    is_enabled: true,
    created_at: chrono::Utc::now(),
};

// Add integration
integration_manager.add_integration(slack_integration).await?;
```

### Integration Examples

#### Slack Integration

```javascript
// Send Slack message via voice command
gestura.voice.onCommand('send slack message *', async (text) => {
    const message = text.replace('send slack message ', '');
    
    const response = await gestura.integrations.execute('slack', {
        method: 'POST',
        path: '/chat.postMessage',
        body: {
            channel: '#general',
            text: message
        }
    });
    
    if (response.success) {
        gestura.notify('Slack', 'Message sent successfully');
    } else {
        gestura.notify('Slack', 'Failed to send message');
    }
});
```

#### Smart Home Integration

```python
# Control Philips Hue lights with gestures
@gestura.gesture.on('circle')
async def toggle_lights(gesture_data):
    if gesture_data.confidence > 0.8:
        response = await gestura.integrations.execute('philips_hue', {
            'method': 'PUT',
            'path': '/lights/1/state',
            'body': {'on': True, 'bri': 254}
        })
        
        if response.success:
            await gestura.ring.send_haptic('success', 0.7)
        else:
            await gestura.ring.send_haptic('error', 0.5)
```

## Testing

### Unit Testing

```rust
#[cfg(test)]
mod tests {
    use super::*;
    
    #[tokio::test]
    async fn test_voice_recognition() {
        let processor = VoiceProcessor::new();
        let audio_data = load_test_audio("test_sample.wav");
        
        let result = processor.recognize(audio_data).await.unwrap();
        
        assert!(result.confidence > 0.8);
        assert_eq!(result.text, "hello world");
    }
    
    #[tokio::test]
    async fn test_gesture_recognition() {
        let recognizer = GestureRecognizer::new();
        let sensor_data = load_test_gesture("tap_gesture.json");
        
        let result = recognizer.recognize(sensor_data).await.unwrap();
        
        assert_eq!(result.gesture, "tap");
        assert!(result.confidence > 0.7);
    }
}
```

### Integration Testing

```javascript
// Plugin integration test
describe('Plugin System', () => {
    let pluginManager;
    
    beforeEach(() => {
        pluginManager = new PluginManager();
    });
    
    test('should load plugin successfully', async () => {
        const plugin = await pluginManager.loadPlugin('test-plugin');
        expect(plugin).toBeDefined();
        expect(plugin.name).toBe('Test Plugin');
    });
    
    test('should execute plugin command', async () => {
        const plugin = await pluginManager.loadPlugin('test-plugin');
        const result = await plugin.handleCommand('hello', {});
        
        expect(result.message).toBe('Hello from plugin!');
    });
});
```

### End-to-End Testing

```python
# E2E test for voice-to-gesture workflow
import pytest
from gestura_test import GesturaTestClient

@pytest.mark.asyncio
async def test_voice_to_gesture_workflow():
    client = GesturaTestClient()
    
    # Simulate voice command
    voice_result = await client.voice.recognize("perform tap gesture")
    assert voice_result.text == "perform tap gesture"
    
    # Verify gesture execution
    gesture_events = await client.gestures.get_recent_events()
    assert len(gesture_events) > 0
    assert gesture_events[0].gesture == "tap"
```

## Deployment

### Building for Production

```bash
# Build release version
just build-release

# Create platform packages
just package-mac      # Creates .dmg
just package-windows  # Creates .exe installer
just package-linux    # Creates .deb, .rpm, .AppImage

# Sign and notarize (macOS)
just sign-mac
just notarize-mac

# Create universal packages
just package-all
```

### Distribution

#### App Stores
- **Mac App Store**: Use Xcode for submission
- **Microsoft Store**: Use Partner Center
- **Snap Store**: Use snapcraft
- **Flathub**: Use flatpak-builder

#### Direct Distribution
- **GitHub Releases**: Automated via CI/CD
- **Website Downloads**: Host on CDN
- **Package Managers**: Submit to Homebrew, Chocolatey, etc.

### CI/CD Pipeline

```yaml
# .github/workflows/build.yml
name: Build and Test

on: [push, pull_request]

jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v3
      - uses: actions-rs/toolchain@v1
        with:
          toolchain: stable
      - run: cargo test
      - run: npm test

  build:
    needs: test
    strategy:
      matrix:
        os: [ubuntu-latest, windows-latest, macos-latest]
    runs-on: ${{ matrix.os }}
    steps:
      - uses: actions/checkout@v3
      - uses: tauri-apps/tauri-action@v0
        env:
          GITHUB_TOKEN: ${{ secrets.GITHUB_TOKEN }}
        with:
          tagName: v__VERSION__
          releaseName: 'Gestura v__VERSION__'
          releaseBody: 'See CHANGELOG.md for details'
          releaseDraft: true
          prerelease: false
```

---

## Resources

### Documentation
- **API Reference**: https://docs.gestura.app/api
- **Plugin Guide**: https://docs.gestura.app/plugins
- **Examples**: https://github.com/gestura-ai/examples

### Community
- **Discord**: https://discord.gg/gestura
- **Forum**: https://community.gestura.app
- **GitHub**: https://github.com/gestura-ai

### Support
- **Developer Support**: dev-support@gestura.app
- **Bug Reports**: https://github.com/gestura-ai/gestura-app/issues
- **Feature Requests**: https://github.com/gestura-ai/gestura-app/discussions

---

*Happy coding! Build amazing experiences with Gestura.app.*
