use gestura::{AppConfig, voice_select::select_voice};

#[test]
fn selects_mock_when_no_config() {
    let mut cfg = AppConfig::default();
    cfg.voice.provider = "none".into();
    cfg.voice.input_path = None;
    let engine = select_voice(&cfg);
    assert_eq!(engine.engine_name(), "mock");
}
