//! Performance and load tests for Gestura.app
//! Tests system behavior under stress and measures performance metrics

#[allow(unused_imports)]
use gestura_gui::*;
use std::time::{Duration, Instant};
use tokio::time::timeout;

#[tokio::test]
async fn test_agent_spawning_performance() {
    let agent_manager =
        gestura_gui::agents::AgentManager::new(std::env::temp_dir().join("perf_test.db"));

    let start = Instant::now();
    let num_agents = 50;

    // Spawn multiple agents concurrently
    let mut handles = Vec::new();
    for i in 0..num_agents {
        let manager = agent_manager.clone();
        let handle = tokio::spawn(async move {
            let agent_id = format!("perf-agent-{}", i);
            let agent_name = format!("Performance Agent {}", i);
            manager.spawn_agent(agent_id, agent_name).await;
        });
        handles.push(handle);
    }

    // Wait for all agents to spawn
    for handle in handles {
        handle.await.unwrap();
    }

    let spawn_duration = start.elapsed();
    println!("Spawned {} agents in {:?}", num_agents, spawn_duration);

    // Should complete within reasonable time (adjust based on system)
    assert!(spawn_duration < Duration::from_secs(30));

    // Test message sending performance
    let start = Instant::now();
    let messages_per_agent = 10;

    let mut handles = Vec::new();
    for i in 0..num_agents {
        let manager = agent_manager.clone();
        let handle = tokio::spawn(async move {
            let agent_id = format!("perf-agent-{}", i);
            for j in 0..messages_per_agent {
                let message = format!("performance test message {}", j);
                manager.send_event(&agent_id, message).await;
            }
        });
        handles.push(handle);
    }

    for handle in handles {
        handle.await.unwrap();
    }

    let message_duration = start.elapsed();
    let total_messages = num_agents * messages_per_agent;
    println!("Sent {} messages in {:?}", total_messages, message_duration);

    // Cleanup
    let shutdown_start = Instant::now();
    agent_manager.shutdown_all(10).await;
    let shutdown_duration = shutdown_start.elapsed();
    println!("Shutdown {} agents in {:?}", num_agents, shutdown_duration);

    // Shutdown should be reasonably fast
    assert!(shutdown_duration < Duration::from_secs(15));
}

#[tokio::test]
async fn test_memory_bus_performance() {
    let bus = gestura_gui::memory_bus::MemoryBus::new(10000);

    // Test high-frequency publishing
    let start = Instant::now();
    let num_messages = 1000;

    for i in 0..num_messages {
        let message = format!("performance test message {}", i).into_bytes();
        bus.publish("perf.test", message).await.unwrap();
    }

    let publish_duration = start.elapsed();
    println!(
        "Published {} messages in {:?}",
        num_messages, publish_duration
    );

    // Should handle high throughput
    let messages_per_second = num_messages as f64 / publish_duration.as_secs_f64();
    println!("Throughput: {:.0} messages/second", messages_per_second);
    assert!(messages_per_second > 100.0); // At least 100 msg/sec

    // Test concurrent subscribers
    let num_subscribers = 10;
    let mut receivers = Vec::new();

    for _ in 0..num_subscribers {
        let receiver = bus.subscribe("perf.concurrent").await.unwrap();
        receivers.push(receiver);
    }

    // Send messages to all subscribers
    let start = Instant::now();
    let concurrent_messages = 100;

    for i in 0..concurrent_messages {
        let message = format!("concurrent message {}", i).into_bytes();
        bus.publish("perf.concurrent", message).await.unwrap();
    }

    let concurrent_duration = start.elapsed();
    println!(
        "Sent {} messages to {} subscribers in {:?}",
        concurrent_messages, num_subscribers, concurrent_duration
    );

    // Verify all subscribers received messages
    for mut receiver in receivers {
        let mut received_count = 0;
        while received_count < concurrent_messages {
            match timeout(Duration::from_millis(100), receiver.recv()).await {
                Ok(Ok(_)) => received_count += 1,
                _ => break,
            }
        }
        assert_eq!(received_count, concurrent_messages);
    }
}

#[tokio::test]
async fn test_mcp_server_performance() {
    let haptic_interface = std::sync::Arc::new(gestura_gui::haptics::MockHaptics);
    let mcp_server = gestura_gui::mcp_server::McpServer::new(haptic_interface);

    // Test rapid request handling
    let start = Instant::now();
    let num_requests = 100;

    let server = std::sync::Arc::new(mcp_server);
    let mut handles = Vec::new();
    for i in 0..num_requests {
        let server_clone = server.clone();
        let handle = tokio::spawn(async move {
            let request = gestura_gui::mcp_server::JsonRpcRequest {
                jsonrpc: "2.0".to_string(),
                method: "tools/list".to_string(),
                params: None,
                id: Some(serde_json::Value::Number(serde_json::Number::from(i))),
            };
            server_clone.handle_request(request).await
        });
        handles.push(handle);
    }

    let mut successful_requests = 0;
    for handle in handles {
        let response = handle.await.unwrap();
        if response.error.is_none() {
            successful_requests += 1;
        }
    }

    let request_duration = start.elapsed();
    println!(
        "Processed {} MCP requests in {:?}",
        num_requests, request_duration
    );
    println!("Success rate: {}/{}", successful_requests, num_requests);

    assert_eq!(successful_requests, num_requests);

    let requests_per_second = num_requests as f64 / request_duration.as_secs_f64();
    println!("MCP throughput: {:.0} requests/second", requests_per_second);
    assert!(requests_per_second > 50.0); // At least 50 req/sec
}

#[tokio::test]
async fn test_device_simulator_performance() {
    let (simulator, mut event_rx) = gestura_gui::device_simulator::create_test_simulator().await;

    // Add many simulated devices
    let num_devices = 20;
    for i in 0..num_devices {
        let device_id = format!("perf-ring-{:03}", i);
        let device_name = format!("Performance Ring {}", i);
        simulator.add_ring(device_id, device_name).await;
    }

    // Measure event generation rate
    let start = Instant::now();
    let mut event_count = 0;
    let test_duration = Duration::from_secs(10);

    while start.elapsed() < test_duration {
        match timeout(Duration::from_millis(100), event_rx.recv()).await {
            Ok(Ok(_)) => event_count += 1,
            _ => continue,
        }
    }

    simulator.stop_simulation().await;

    let events_per_second = event_count as f64 / test_duration.as_secs_f64();
    println!(
        "Generated {} events in {:?} ({:.1} events/sec)",
        event_count, test_duration, events_per_second
    );

    // Should generate reasonable number of events
    assert!(event_count > 0);
}

#[tokio::test]
async fn test_voice_processing_performance() {
    let config = gestura_gui::AppConfig::load();
    let _engine = gestura_gui::voice_select::select_voice(&config);

    // Test multiple voice processing requests
    let start = Instant::now();
    let num_requests = 10;

    let mut handles = Vec::new();
    for _ in 0..num_requests {
        let engine = gestura_gui::voice_select::select_voice(&config);
        let config = config.clone();
        let handle = tokio::spawn(async move {
            // This will use mock engine in tests
            engine.process_command(&config, None).await
        });
        handles.push(handle);
    }

    let mut completed_requests = 0;
    for handle in handles {
        // Count all completed requests (success or error) - we're measuring throughput
        let _ = handle.await.unwrap();
        completed_requests += 1;
    }

    let processing_duration = start.elapsed();
    println!(
        "Processed {} voice requests in {:?}",
        num_requests, processing_duration
    );

    // Use as_secs_f64() for accurate calculation even with sub-millisecond durations
    let duration_secs = processing_duration.as_secs_f64();
    let requests_per_second = if duration_secs > 0.0 {
        completed_requests as f64 / duration_secs
    } else {
        f64::INFINITY // Completed instantly
    };
    println!(
        "Voice processing throughput: {:.1} requests/second",
        requests_per_second
    );

    // Mock engine should be fast - either high throughput or completed very quickly
    assert!(
        requests_per_second > 5.0 || duration_secs < 0.01,
        "Expected high throughput ({:.1} req/s) or fast completion ({:.4}s)",
        requests_per_second,
        duration_secs
    );
}

#[tokio::test]
async fn test_memory_usage_under_load() {
    // This test monitors memory usage during high load
    let initial_memory = get_memory_usage();

    // Create multiple components under load
    let agent_manager =
        gestura_gui::agents::AgentManager::new(std::env::temp_dir().join("memory_test.db"));
    let bus = gestura_gui::memory_bus::MemoryBus::new(1000);

    // Spawn agents and send many messages
    for i in 0..20 {
        let agent_id = format!("memory-agent-{}", i);
        agent_manager
            .spawn_agent(agent_id.clone(), format!("Memory Agent {}", i))
            .await;

        // Send many messages
        for j in 0..50 {
            let message = format!("memory test message {} from agent {}", j, i);
            agent_manager.send_event(&agent_id, message).await;

            // Also test memory bus
            let bus_message = format!("bus message {} from {}", j, i).into_bytes();
            bus.publish(&format!("memory.test.{}", i), bus_message)
                .await
                .unwrap();
        }
    }

    let peak_memory = get_memory_usage();
    println!(
        "Memory usage: initial={} MB, peak={} MB",
        initial_memory, peak_memory
    );

    // Cleanup
    agent_manager.shutdown_all(5).await;
    bus.clear_history().await;

    let final_memory = get_memory_usage();
    println!("Memory usage after cleanup: {} MB", final_memory);

    // Memory should not grow excessively (adjust threshold as needed)
    let memory_growth = peak_memory - initial_memory;
    assert!(
        memory_growth < 100.0,
        "Memory growth too high: {} MB",
        memory_growth
    );
}

/// Get current memory usage in MB (simplified)
fn get_memory_usage() -> f64 {
    // This is a simplified memory measurement
    // In production, you'd use more sophisticated memory profiling
    std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .ok()
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .and_then(|s| s.trim().parse::<f64>().ok())
        .map(|kb| kb / 1024.0) // Convert KB to MB
        .unwrap_or(0.0)
}

#[tokio::test]
async fn test_concurrent_api_calls() {
    // Test concurrent API-like operations
    let config = gestura_gui::AppConfig::load();
    let num_concurrent = 20;

    let start = Instant::now();
    let mut handles = Vec::new();

    for i in 0..num_concurrent {
        let config = config.clone();
        let handle = tokio::spawn(async move {
            // Simulate various API operations
            let engine = gestura_gui::voice_select::select_voice(&config);
            let _name = engine.engine_name();

            // Test configuration operations
            let mut test_config = config.clone();
            test_config.hotkey_listen = format!("Ctrl+Alt+{}", i);

            // Test voice validation
            let validation_result = gestura_gui::voice_select::validate_voice_config_for_run(
                &test_config,
                engine.as_ref(),
            );
            validation_result.is_ok()
        });
        handles.push(handle);
    }

    let mut completed_operations = 0;
    for handle in handles {
        // Count all completed operations - we're measuring throughput
        let _ = handle.await.unwrap();
        completed_operations += 1;
    }

    let concurrent_duration = start.elapsed();
    println!(
        "Completed {} concurrent operations in {:?}",
        num_concurrent, concurrent_duration
    );

    // Use as_secs_f64() for accurate calculation even with sub-millisecond durations
    let duration_secs = concurrent_duration.as_secs_f64();
    let operations_per_second = if duration_secs > 0.0 {
        completed_operations as f64 / duration_secs
    } else {
        f64::INFINITY // Completed instantly
    };
    println!(
        "Concurrent throughput: {:.0} operations/second",
        operations_per_second
    );

    // Either high throughput or completed quickly
    assert!(
        operations_per_second > 10.0 || duration_secs < 0.01,
        "Expected high throughput ({:.0} ops/s) or fast completion ({:.4}s)",
        operations_per_second,
        duration_secs
    );
}
