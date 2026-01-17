// Basic tests verifying Stage 1–4 scaffolding compiles and key units behave.
// These are pure unit tests with no network/process dependency.

#[test]
fn stage1_config_loads_defaults() {
    let cfg = gestura_gui::AppConfig::load();
    assert!(!cfg.hotkey_listen.is_empty());
    assert!(cfg.grace_period_secs > 0);
}

/// Test EchoProvider - only available with `dev` feature.
/// In production builds without `dev` feature, EchoProvider is not available.
#[test]
#[cfg(feature = "dev")]
fn stage4_llm_echo_provider() {
    // Create a config that explicitly uses the "echo" provider to test EchoProvider
    // Note: In production builds, "echo" provider will return UnconfiguredProvider
    // which returns an error. This test verifies the echo fallback works in dev/test.
    let mut cfg = gestura_gui::AppConfig::load();
    cfg.llm.primary = "echo".to_string(); // Force echo provider

    let provider = gestura_gui::llm_provider::select_provider(
        &cfg,
        &gestura_gui::llm_provider::AgentContext {
            agent_id: "t".into(),
        },
    );
    let rt = tokio::runtime::Runtime::new().unwrap();
    // In dev mode, EchoProvider is available via cfg(feature = "dev") in gestura-core
    let out = rt.block_on(provider.call("hello")).expect("echo works in dev mode");
    assert!(out.contains("ECHO: hello"));
}

/// Test that unconfigured providers return an error in production builds.
/// This ensures we don't silently fail when a provider is not configured.
#[test]
#[cfg(not(feature = "dev"))]
fn stage4_llm_unconfigured_provider() {
    // In production builds, "echo" provider returns UnconfiguredProvider
    let mut cfg = gestura_gui::AppConfig::load();
    cfg.llm.primary = "echo".to_string(); // Force echo provider

    let provider = gestura_gui::llm_provider::select_provider(
        &cfg,
        &gestura_gui::llm_provider::AgentContext {
            agent_id: "t".into(),
        },
    );
    let rt = tokio::runtime::Runtime::new().unwrap();
    // In production mode, this should return an error
    let result = rt.block_on(provider.call("hello"));
    assert!(result.is_err(), "Expected error for unconfigured provider in production");
}

#[tokio::test]
async fn stage3_agents_spawn_and_event() {
    use gestura_gui::agents::AgentManager;

    let mgr = AgentManager::new(std::env::temp_dir().join("gestura-test.db"));
    mgr.spawn_agent("a".into(), "Agent A".into()).await;
    mgr.send_event("a", "data-query:tests/fixtures/sample.json".into())
        .await;
    // We can at least ensure load_state returns None without KV attached
    assert!(mgr.load_state("a").await.is_none());
    // Shutdown quickly
    mgr.shutdown_all(0).await;
}
