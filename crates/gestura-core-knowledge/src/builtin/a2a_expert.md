# A2A Expert

You are an expert in the Agent-to-Agent (A2A) protocol.

## Protocol Overview

A2A is Google's open protocol (Linux Foundation) for agent interoperability:
- Agent discovery via Agent Cards
- Task delegation and execution
- Authentication and authorization
- Multi-agent collaboration

## Agent Card

The Agent Card is a JSON document describing an agent's capabilities:

```json
{
  "name": "Gestura Agent",
  "description": "Voice-first AI assistant",
  "url": "https://agent.example.com",
  "version": "1.0.0",
  "capabilities": {
    "streaming": true,
    "pushNotifications": true,
    "stateTransitionHistory": true
  },
  "authentication": {
    "schemes": ["bearer"]
  },
  "defaultInputModes": ["text", "voice"],
  "defaultOutputModes": ["text"],
  "skills": [
    {
      "id": "voice-transcription",
      "name": "Voice Transcription",
      "description": "Convert speech to text"
    }
  ]
}
```

## Task Lifecycle

```
┌─────────┐    ┌──────────┐    ┌───────────┐    ┌──────────┐
│ PENDING │───▶│ RUNNING  │───▶│ COMPLETED │    │ FAILED   │
└─────────┘    └──────────┘    └───────────┘    └──────────┘
                    │                               ▲
                    └───────────────────────────────┘
```

### Task States
| State | Description |
|-------|-------------|
| `pending` | Task received, not started |
| `running` | Task in progress |
| `completed` | Task finished successfully |
| `failed` | Task failed with error |
| `cancelled` | Task was cancelled |

## API Endpoints

### Discovery
```
GET /.well-known/agent.json
```

### Task Management
```
POST /tasks/send          # Send a new task
GET  /tasks/{id}          # Get task status
POST /tasks/{id}/cancel   # Cancel a task
```

### Streaming
```
POST /tasks/sendSubscribe # Send task with SSE streaming
```

## Authentication

### Bearer Token
```http
Authorization: Bearer <token>
```

### Token Generation
```rust
fn generate_token(agent_id: &str, hours: u64) -> String {
    let expiry = SystemTime::now() + Duration::from_secs(hours * 3600);
    // Generate JWT or opaque token
}
```

## Message Format

### Task Request
```json
{
  "id": "task-123",
  "message": {
    "role": "user",
    "parts": [
      { "type": "text", "text": "Transcribe this audio" },
      { "type": "file", "mimeType": "audio/wav", "data": "base64..." }
    ]
  }
}
```

### Task Response
```json
{
  "id": "task-123",
  "status": {
    "state": "completed",
    "message": {
      "role": "agent",
      "parts": [
        { "type": "text", "text": "Transcription complete" }
      ]
    }
  }
}
```

## Multi-Agent Patterns

1. **Delegation**: Agent A delegates subtask to Agent B
2. **Collaboration**: Multiple agents work on parts of a task
3. **Orchestration**: Central agent coordinates others
4. **Pipeline**: Tasks flow through agent chain

## Best Practices

1. **Validate tokens**: Check expiry and signature
2. **Handle timeouts**: Set reasonable task timeouts
3. **Support cancellation**: Allow graceful task cancellation
4. **Emit progress**: Stream updates for long tasks
5. **Error details**: Provide actionable error messages

