# Gestura.app API Documentation

## Overview

Gestura.app provides a comprehensive REST API for voice recognition, gesture control, and haptic feedback integration. The API is designed for developers who want to integrate Gestura's capabilities into their applications.

## Base URL

```
https://api.gestura.app/v1
```

## Authentication

All API requests require authentication using an API key. Include your API key in the request header:

```http
Authorization: Bearer YOUR_API_KEY
```

### Getting an API Key

1. Open Gestura.app
2. Go to Settings → Developer → API Keys
3. Click "Generate New Key"
4. Copy the generated key (starts with `gsk_`)

## Rate Limits

- **Free Tier**: 1,000 requests per hour
- **Pro Tier**: 10,000 requests per hour
- **Enterprise**: Custom limits

Rate limit headers are included in all responses:
- `X-RateLimit-Limit`: Request limit per hour
- `X-RateLimit-Remaining`: Remaining requests
- `X-RateLimit-Reset`: Time when limit resets (Unix timestamp)

## Voice Recognition API

### Recognize Speech

Convert audio to text using advanced speech recognition.

```http
POST /voice/recognize
```

**Parameters:**

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `audio_data` | File | Yes | Audio file (WAV, MP3, FLAC) |
| `language` | String | No | Language code (default: "en-US") |
| `model` | String | No | Model type ("base", "small", "medium", "large") |
| `enable_vad` | Boolean | No | Enable voice activity detection |

**Example Request:**

```bash
curl -X POST https://api.gestura.app/v1/voice/recognize \
  -H "Authorization: Bearer YOUR_API_KEY" \
  -F "audio_data=@recording.wav" \
  -F "language=en-US" \
  -F "model=medium"
```

**Response:**

```json
{
  "text": "Hello, this is a test recording",
  "confidence": 0.95,
  "language": "en-US",
  "duration_ms": 2500,
  "words": [
    {
      "word": "Hello",
      "start_time": 0.0,
      "end_time": 0.5,
      "confidence": 0.98
    }
  ]
}
```

### Voice Activity Detection

Detect speech segments in audio.

```http
POST /voice/vad
```

**Parameters:**

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `audio_data` | File | Yes | Audio file |
| `threshold` | Float | No | Detection threshold (0.0-1.0) |
| `min_duration` | Integer | No | Minimum segment duration (ms) |

**Response:**

```json
{
  "segments": [
    {
      "start_time": 1.2,
      "end_time": 3.8,
      "confidence": 0.92
    }
  ],
  "total_speech_duration": 2.6,
  "speech_ratio": 0.65
}
```

## Gesture Recognition API

### Recognize Gesture

Identify gestures from sensor data.

```http
POST /gestures/recognize
```

**Parameters:**

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `sensor_data` | Array | Yes | Array of sensor readings |
| `user_id` | String | No | User ID for personalized recognition |
| `gesture_types` | Array | No | Limit to specific gesture types |

**Sensor Data Format:**

```json
{
  "sensor_data": [
    {
      "timestamp_ms": 1640995200000,
      "accelerometer": [0.1, 0.2, 9.8],
      "gyroscope": [0.01, 0.02, 0.03],
      "magnetometer": [25.0, -15.0, 45.0],
      "quaternion": [1.0, 0.0, 0.0, 0.0]
    }
  ]
}
```

**Response:**

```json
{
  "gesture": "tap",
  "confidence": 0.89,
  "alternatives": [
    {
      "gesture": "double_tap",
      "confidence": 0.23
    }
  ],
  "processing_time_ms": 15
}
```

### Custom Gestures

#### Create Custom Gesture

```http
POST /gestures/custom
```

**Parameters:**

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `name` | String | Yes | Gesture name |
| `description` | String | No | Gesture description |
| `gesture_type` | String | Yes | Type: "motion", "tap", "swipe", etc. |
| `user_id` | String | Yes | User ID |

#### Train Custom Gesture

```http
POST /gestures/custom/{gesture_id}/train
```

**Parameters:**

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `training_samples` | Array | Yes | Array of sensor data samples |
| `target_samples` | Integer | No | Number of samples needed (default: 5) |

## Ring Integration API

### Ring Status

Get Haptic Harmony ring connection status.

```http
GET /ring/status
```

**Response:**

```json
{
  "connected": true,
  "battery_level": 85,
  "signal_strength": -45,
  "firmware_version": "1.2.3",
  "last_seen": "2024-01-15T10:30:00Z"
}
```

### Send Haptic Feedback

```http
POST /ring/haptic
```

**Parameters:**

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `pattern` | String | Yes | Pattern: "tap", "pulse", "vibrate", "custom" |
| `intensity` | Float | No | Intensity (0.0-1.0, default: 0.5) |
| `duration_ms` | Integer | No | Duration in milliseconds |
| `custom_pattern` | Array | No | Custom pattern data |

**Response:**

```json
{
  "success": true,
  "pattern_id": "pat_123456",
  "estimated_duration_ms": 500
}
```

## Analytics API

### Usage Analytics

Get usage analytics and insights.

```http
GET /analytics/usage
```

**Parameters:**

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `days` | Integer | No | Number of days (default: 7, max: 365) |
| `user_id` | String | No | Specific user ID |
| `include_details` | Boolean | No | Include detailed breakdown |

**Response:**

```json
{
  "total_events": 1250,
  "unique_users": 45,
  "active_sessions": 12,
  "most_used_features": [
    {
      "feature": "voice_commands",
      "usage_count": 450
    }
  ],
  "usage_patterns": {
    "peak_usage_hours": [9, 14, 20],
    "average_session_duration_minutes": 15.5
  },
  "performance_metrics": {
    "average_response_time_ms": 125,
    "gesture_recognition_accuracy": 0.92,
    "voice_recognition_accuracy": 0.95
  }
}
```

## Plugin System API

### List Plugins

```http
GET /plugins
```

**Response:**

```json
{
  "plugins": [
    {
      "id": "plugin_123",
      "name": "Weather Plugin",
      "version": "1.0.0",
      "state": "running",
      "permissions": ["network", "notifications"]
    }
  ]
}
```

### Execute Plugin Command

```http
POST /plugins/{plugin_id}/execute
```

**Parameters:**

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `command` | String | Yes | Command to execute |
| `args` | Object | No | Command arguments |

## Scripting API

### Execute Script

```http
POST /scripts/execute
```

**Parameters:**

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `script_id` | String | Yes | Script ID |
| `context` | Object | No | Execution context |
| `timeout_seconds` | Integer | No | Execution timeout |

**Response:**

```json
{
  "success": true,
  "return_value": {"result": "completed"},
  "execution_time_ms": 250,
  "output": "Script executed successfully"
}
```

## Error Handling

All API endpoints return standard HTTP status codes:

- `200 OK`: Request successful
- `400 Bad Request`: Invalid parameters
- `401 Unauthorized`: Invalid or missing API key
- `403 Forbidden`: Insufficient permissions
- `404 Not Found`: Resource not found
- `429 Too Many Requests`: Rate limit exceeded
- `500 Internal Server Error`: Server error

**Error Response Format:**

```json
{
  "error": {
    "code": "INVALID_PARAMETER",
    "message": "The 'language' parameter must be a valid language code",
    "details": {
      "parameter": "language",
      "provided_value": "invalid",
      "valid_values": ["en-US", "es-ES", "fr-FR"]
    }
  }
}
```

## SDKs and Libraries

Official SDKs are available for:

- **JavaScript/Node.js**: `npm install @gestura/sdk`
- **Python**: `pip install gestura-sdk`
- **Rust**: `cargo add gestura-sdk`
- **Go**: `go get github.com/gestura-ai/go-sdk`

### JavaScript Example

```javascript
import { GesturaSDK } from '@gestura/sdk';

const client = new GesturaSDK({
  apiKey: 'YOUR_API_KEY',
  baseUrl: 'https://api.gestura.app/v1'
});

// Recognize speech
const result = await client.voice.recognize({
  audioData: audioFile,
  language: 'en-US'
});

console.log('Recognized text:', result.text);
```

### Python Example

```python
from gestura_sdk import GesturaClient

client = GesturaClient(api_key='YOUR_API_KEY')

# Recognize gesture
result = client.gestures.recognize(sensor_data=sensor_readings)
print(f'Recognized gesture: {result.gesture}')
```

## Webhooks

Configure webhooks to receive real-time notifications:

```http
POST /webhooks
```

**Parameters:**

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `url` | String | Yes | Webhook URL |
| `events` | Array | Yes | Events to subscribe to |
| `secret` | String | No | Webhook secret for verification |

**Supported Events:**

- `voice.recognized`: Speech recognition completed
- `gesture.detected`: Gesture detected
- `ring.connected`: Ring connected/disconnected
- `error.occurred`: Error occurred

## Support

- **Documentation**: https://docs.gestura.app
- **API Status**: https://status.gestura.app
- **Support**: support@gestura.app
- **Discord**: https://discord.gg/gestura

## Changelog

### v1.0.0 (2024-01-15)
- Initial API release
- Voice recognition endpoints
- Gesture recognition endpoints
- Ring integration
- Analytics API
- Plugin system
- Scripting support
