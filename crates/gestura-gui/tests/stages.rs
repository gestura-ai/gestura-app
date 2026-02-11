// Basic tests verifying Stage 1–4 scaffolding compiles and key units behave.
// These are pure unit tests with no network/process dependency.
use gestura_gui::AppConfigSecurityExt;

#[test]
fn stage1_config_loads_defaults() {
    let cfg = gestura_gui::AppConfig::load();
    assert!(!cfg.hotkey_listen.is_empty());
    assert!(cfg.grace_period_secs > 0);
}

/// Test that unconfigured providers return an error.
/// This ensures we don't silently fail when a provider is not configured.
#[test]
fn stage4_llm_unconfigured_provider() {
    // When no provider is configured (using default config which has no API keys),
    // UnconfiguredProvider is returned
    let cfg = gestura_gui::AppConfig::default();

    let provider = gestura_gui::llm_provider::select_provider(
        &cfg,
        &gestura_gui::llm_provider::AgentContext {
            agent_id: "t".into(),
        },
    );
    let rt = tokio::runtime::Runtime::new().unwrap();
    // This should return an error since no provider API key is configured
    let result = rt.block_on(provider.call("hello"));
    assert!(result.is_err(), "Expected error for unconfigured provider");
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
