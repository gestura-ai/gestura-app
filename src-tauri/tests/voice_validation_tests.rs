use gestura::{
    AppConfig,
    voice_select::{select_voice, validate_voice_config_for_run},
};

#[test]
fn validation_fails_when_input_missing_for_real_engines() {
    let mut cfg = AppConfig::default();
    // Force OpenAI path by simulating api key present but no input
    cfg.voice.openai_api_key = Some("sk-test".into());
    cfg.voice.input_path = None;

    let engine = select_voice(&cfg);
    let err = validate_voice_config_for_run(&cfg, engine.as_ref()).unwrap_err();
    assert!(err.to_string().contains("input_path"));
}
