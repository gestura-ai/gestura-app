// Basic tests verifying Stage 1–4 scaffolding compiles and key units behave.
// These are pure unit tests with no network/process dependency.

#[test]
fn stage1_config_loads_defaults() {
    let cfg = gestura::AppConfig::load();
    assert!(!cfg.hotkey_listen.is_empty());
    assert!(cfg.grace_period_secs > 0);
}

#[test]
fn stage4_llm_echo_provider() {
    // Create a config that explicitly uses the "echo" provider to test EchoProvider
    let mut cfg = gestura::AppConfig::load();
    cfg.llm.primary = "echo".to_string(); // Force echo provider

    let provider = gestura::llm_provider::select_provider(
        &cfg,
        &gestura::llm_provider::AgentContext {
            agent_id: "t".into(),
        },
    );
    let rt = tokio::runtime::Runtime::new().unwrap();
    let out = rt.block_on(provider.call("hello")).expect("echo works");
    assert!(out.contains("ECHO: hello"));
}

#[tokio::test]
async fn stage3_agents_spawn_and_event() {
    use gestura::agents::AgentManager;

    let mgr = AgentManager::new(std::env::temp_dir().join("gestura-test.db"));
    mgr.spawn_agent("a".into(), "Agent A".into()).await;
    mgr.send_event("a", "data-query:tests/fixtures/sample.json".into())
        .await;
    // We can at least ensure load_state returns None without KV attached
    assert!(mgr.load_state("a").await.is_none());
    // Shutdown quickly
    mgr.shutdown_all(0).await;
}
