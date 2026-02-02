//! Integration tests for Gestura.app
//! Tests the interaction between major components

use gestura_core::McpIntegrator;
use gestura_gui::*;
use gestura_gui::ble::RingManager;
use std::time::Duration;
use tokio::time::timeout;

#[tokio::test]
async fn test_voice_to_agent_pipeline() {
    // Test voice processing -> agent communication pipeline
    let config = AppConfig::load();
    let engine = gestura_gui::voice_select::select_voice(&config);

    // Validate engine selection
    assert!(!engine.engine_name().is_empty());

    // Test agent manager
    let agent_manager =
        gestura_gui::agents::AgentManager::new(std::env::temp_dir().join("test_agents.db"));
    agent_manager
        .spawn_agent(
            "test-voice-agent".to_string(),
            "Voice Test Agent".to_string(),
        )
        .await;

    // Send test event
    agent_manager
        .send_event("test-voice-agent", "voice:test message".to_string())
        .await;

    // Cleanup
    agent_manager.shutdown_all(5).await;
}

#[tokio::test]
async fn test_mcp_server_functionality() {
    // Test MCP server with haptic integration
    let haptic_interface = std::sync::Arc::new(gestura_gui::haptics::MockHaptics);
    let mcp_server = gestura_gui::mcp_server::McpServer::new(haptic_interface);

    // Test tools/list
    let request = gestura_gui::mcp_server::JsonRpcRequest {
        jsonrpc: "2.0".to_string(),
        method: "tools/list".to_string(),
        params: None,
        id: Some(serde_json::Value::Number(serde_json::Number::from(1))),
    };

    let response = mcp_server.handle_request(request).await;
    assert_eq!(response.jsonrpc, "2.0");
    assert!(response.result.is_some());
    assert!(response.error.is_none());

    // Generate a valid auth token for testing
    let mcp = gestura_core::mcp::get_mcp();
    let token_info = mcp
        .generate_token("test-client", vec!["haptic".to_string()], 1)
        .expect("Failed to generate test token");

    // Grant haptic permission to the token
    mcp.grant_haptic_permission(&token_info.token)
        .await
        .expect("Failed to grant haptic permission");

    // Test haptic tool call with auth token
    let haptic_request = gestura_gui::mcp_server::JsonRpcRequest {
        jsonrpc: "2.0".to_string(),
        method: "tools/call".to_string(),
        params: Some(serde_json::json!({
            "name": "send_haptic",
            "arguments": {
                "pattern": "click",
                "intensity": 0.7,
                "duration_ms": 100
            },
            "auth_token": token_info.token
        })),
        id: Some(serde_json::Value::Number(serde_json::Number::from(2))),
    };

    let response = mcp_server.handle_request(haptic_request).await;
    assert_eq!(response.jsonrpc, "2.0");
    assert!(
        response.result.is_some(),
        "Expected result but got error: {:?}",
        response.error
    );
    assert!(response.error.is_none());
}

#[tokio::test]
async fn test_ble_ring_integration() {
    // Test BLE ring manager functionality
    // Test BLE ring manager functionality
    // Use MockRingManager directly to avoid hardware dependency in CI
    let ring_manager = gestura_gui::ble::MockRingManager;

    // Test scanning (should work with mock)
    let rings = ring_manager.scan_for_rings().await;
    assert!(rings.is_ok());

    let ring_ids = rings.unwrap();
    if !ring_ids.is_empty() {
        let device_id = &ring_ids[0];

        // Test pairing
        let pair_result = ring_manager.pair_ring(device_id).await;
        assert!(pair_result.is_ok());

        // Test status
        let status = ring_manager.get_ring_status(device_id).await;
        assert!(status.is_ok());
        assert!(status.unwrap().is_some());

        // Test haptic feedback
        let haptic_request = gestura_gui::haptics::HapticRequest::click();
        let haptic_result = ring_manager.send_haptic(device_id, haptic_request).await;
        assert!(haptic_result.is_ok());
    }
}

#[tokio::test]
async fn test_event_dispatcher() {
    // Test NATS event dispatcher
    let agent_manager =
        gestura_gui::agents::AgentManager::new(std::env::temp_dir().join("test_dispatcher.db"));
    let agent_spawner: std::sync::Arc<dyn gestura_gui::agents::AgentSpawner> =
        std::sync::Arc::new(agent_manager);
    let dispatcher = gestura_gui::dispatcher::EventDispatcher::new(agent_spawner);

    // Test voice event dispatch
    let result = dispatcher
        .dispatch("events.voice", b"test voice data".to_vec())
        .await;
    assert!(result.is_ok());

    // Test hotkey event dispatch
    let result = dispatcher
        .dispatch("events.hotkey", b"trigger".to_vec())
        .await;
    assert!(result.is_ok());

    // Test agent event dispatch
    let result = dispatcher
        .dispatch("agents.test", b"agent data".to_vec())
        .await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_haptic_patterns() {
    // Test advanced haptic pattern builder
    let click_pattern = gestura_gui::haptics::HapticRequest::click();
    assert_eq!(click_pattern.duration_ms, 50);
    assert_eq!(click_pattern.repeat_count, 0);

    let notification_pattern = gestura_gui::haptics::HapticRequest::notification();
    assert_eq!(notification_pattern.repeat_count, 2);
    assert_eq!(notification_pattern.repeat_delay_ms, 300);

    let alert_pattern = gestura_gui::haptics::HapticRequest::alert();
    assert_eq!(alert_pattern.intensity, 1.0);
    assert_eq!(alert_pattern.repeat_count, 3);

    // Test custom pattern builder
    let custom_pattern =
        gestura_gui::haptics::HapticPatternBuilder::new(gestura_gui::haptics::HapticPattern::Pulse)
            .intensity(0.8)
            .duration(200)
            .repeat(5, 100)
            .build();

    assert_eq!(custom_pattern.intensity, 0.8);
    assert_eq!(custom_pattern.duration_ms, 200);
    assert_eq!(custom_pattern.repeat_count, 5);
    assert_eq!(custom_pattern.repeat_delay_ms, 100);
}

#[cfg(feature = "security")]
#[tokio::test]
async fn test_security_encryption() {
    use gestura_gui::security::encryption::Encryptor;

    // Test encryption/decryption
    let encryptor = Encryptor::new().expect("Failed to create encryptor");
    let test_data = b"Hello, secure world!";

    let encrypted = encryptor.encrypt(test_data).expect("Failed to encrypt");
    assert_ne!(encrypted, test_data);
    assert!(encrypted.len() > test_data.len()); // Should be larger due to nonce + tag

    let decrypted = encryptor.decrypt(&encrypted).expect("Failed to decrypt");
    assert_eq!(decrypted, test_data);
}

#[tokio::test]
async fn test_mdh_translation() {
    // Test MDH JSON-LD translation
    let translator = gestura_gui::mdh_translator::MdhTranslator::new();

    // Create a temporary JSON-LD file
    let mut temp_file = tempfile::NamedTempFile::new().unwrap();
    let json_ld_content = r#"{
        "@context": "https://schema.org",
        "@type": "Person",
        "@id": "https://example.com/person/1",
        "name": "Test Person",
        "email": "test@example.com"
    }"#;

    std::io::Write::write_all(&mut temp_file, json_ld_content.as_bytes()).unwrap();

    let result = translator.translate(temp_file.path().to_path_buf()).await;
    assert!(result.is_ok());

    let resource = result.unwrap();
    assert_eq!(resource.uri, "mcp://mdh/Person");
    assert!(resource.data.get("name").is_some());
    assert!(resource.data.get("@context").is_none()); // Should be removed
}

#[tokio::test]
async fn test_configuration_persistence() {
    // Test configuration loading and saving
    let temp_dir = tempfile::tempdir().unwrap();
    let config_path = temp_dir.path().join("config.yaml");
    let mut config = AppConfig::load_from_path(&config_path);

    // Modify config
    config.hotkey_listen = "Ctrl+Alt+G".to_string();
    config.grace_period_secs = 45;
    config.ui.theme_mode = "dark".to_string();
    config.ui.accent = Some("emerald".to_string());

    // Save and reload
    assert!(config.save_to_path(&config_path).is_ok());
    let reloaded_config = AppConfig::load_from_path(&config_path);

    assert_eq!(reloaded_config.hotkey_listen, "Ctrl+Alt+G");
    assert_eq!(reloaded_config.grace_period_secs, 45);
    assert_eq!(reloaded_config.ui.theme_mode, "dark");
    assert_eq!(reloaded_config.ui.accent, Some("emerald".to_string()));
}

#[tokio::test]
async fn test_pipeline_settings_persistence() {
    use gestura_core::pipeline::CompactionStrategy;

    // Test pipeline settings loading and saving
    let temp_dir = tempfile::tempdir().unwrap();
    let config_path = temp_dir.path().join("config.yaml");
    let mut config = AppConfig::load_from_path(&config_path);

    // Modify pipeline settings
    config.pipeline.max_history_messages = 20;
    config.pipeline.auto_compact_threshold_percent = 75;
    config.pipeline.compaction_strategy = CompactionStrategy::MemoryBank;
    config.pipeline.max_context_tokens = 100_000;
    config.pipeline.log_token_usage = false;

    // Save and reload
    assert!(config.save_to_path(&config_path).is_ok());
    let reloaded_config = AppConfig::load_from_path(&config_path);

    // Verify pipeline settings persisted
    assert_eq!(reloaded_config.pipeline.max_history_messages, 20);
    assert_eq!(reloaded_config.pipeline.auto_compact_threshold_percent, 75);
    assert_eq!(
        reloaded_config.pipeline.compaction_strategy,
        CompactionStrategy::MemoryBank
    );
    assert_eq!(reloaded_config.pipeline.max_context_tokens, 100_000);
    assert!(!reloaded_config.pipeline.log_token_usage);

    // Verify auto_compact_threshold() helper method
    assert_eq!(reloaded_config.pipeline.auto_compact_threshold(), 0.75);
}

#[tokio::test]
async fn test_voice_engine_selection() {
    // Test voice engine selection logic
    let mut config = AppConfig::load();

    // Test with no local model (should select mock or OpenAI)
    config.voice.local_model_path = None;
    let engine = gestura_gui::voice_select::select_voice(&config);
    let name = engine.engine_name();
    assert!(name == "mock" || name == "openai-whisper");

    // Test with local model path
    config.voice.local_model_path = Some("/path/to/model.bin".to_string());
    let engine = gestura_gui::voice_select::select_voice(&config);
    let name = engine.engine_name();
    // Should prefer faster-whisper if available, otherwise whisper-local
    assert!(name == "faster-whisper-local" || name == "whisper-local" || name == "mock");
}

#[tokio::test]
async fn test_stress_agent_spawning() {
    // Stress test agent spawning and shutdown
    let agent_manager =
        gestura_gui::agents::AgentManager::new(std::env::temp_dir().join("test_stress.db"));

    // Spawn multiple agents
    for i in 0..10 {
        let agent_id = format!("stress-agent-{}", i);
        let agent_name = format!("Stress Agent {}", i);
        agent_manager.spawn_agent(agent_id, agent_name).await;
    }

    // Send events to all agents
    for i in 0..10 {
        let agent_id = format!("stress-agent-{}", i);
        agent_manager
            .send_event(&agent_id, format!("test-message-{}", i))
            .await;
    }

    // Shutdown all agents
    let shutdown_result = timeout(Duration::from_secs(10), agent_manager.shutdown_all(5)).await;
    assert!(shutdown_result.is_ok());
}
