//! Scripting engine for Gestura.app
//! Supports Lua and Python scripting for automation

#[allow(unused_imports)]
use crate::AppError;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Supported scripting languages
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum ScriptLanguage {
    Lua,
    Python,
    JavaScript,
}

/// Script metadata
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Script {
    pub id: String,
    pub name: String,
    pub description: String,
    pub language: ScriptLanguage,
    pub source_code: String,
    pub entry_point: String,
    pub author: String,
    pub version: String,
    pub permissions: Vec<ScriptPermission>,
    pub triggers: Vec<ScriptTrigger>,
    pub is_enabled: bool,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub last_modified: chrono::DateTime<chrono::Utc>,
    pub execution_count: u32,
    pub last_error: Option<String>,
}

/// Script permissions
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum ScriptPermission {
    FileSystem(String),    // Path pattern
    Network(String),       // Host pattern
    SystemCommands,
    VoiceControl,
    GestureControl,
    RingControl,
    Notifications,
    ClipboardAccess,
    WindowManagement,
    DatabaseAccess,
}

/// Script triggers
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum ScriptTrigger {
    VoiceCommand(String),
    Gesture(String),
    TimeSchedule(String), // Cron-like expression
    ApplicationEvent(String),
    FileSystemEvent(String),
    NetworkEvent(String),
    UserAction(String),
    SystemEvent(String),
}

/// Script execution context
#[derive(Debug, Clone)]
pub struct ScriptContext {
    pub script_id: String,
    pub user_id: String,
    pub session_id: String,
    pub variables: HashMap<String, serde_json::Value>,
    pub permissions: Vec<ScriptPermission>,
    pub execution_timeout: std::time::Duration,
}

/// Script execution result
#[derive(Debug, Clone, serde::Serialize)]
pub struct ScriptExecutionResult {
    pub script_id: String,
    pub success: bool,
    pub return_value: Option<serde_json::Value>,
    pub error_message: Option<String>,
    pub execution_time_ms: u64,
    pub output: String,
    pub warnings: Vec<String>,
}

/// Scripting engine
pub struct ScriptingEngine {
    scripts: Arc<RwLock<HashMap<String, Script>>>,
    active_executions: Arc<RwLock<HashMap<String, ScriptExecution>>>,
    lua_runtime: Arc<RwLock<Option<LuaRuntime>>>,
    python_runtime: Arc<RwLock<Option<PythonRuntime>>>,
    js_runtime: Arc<RwLock<Option<JavaScriptRuntime>>>,
    #[allow(dead_code)]
    script_directory: PathBuf,
}

/// Active script execution
#[derive(Debug, Clone)]
struct ScriptExecution {
    #[allow(dead_code)]
    execution_id: String,
    #[allow(dead_code)]
    script_id: String,
    #[allow(dead_code)]
    started_at: chrono::DateTime<chrono::Utc>,
    #[allow(dead_code)]
    context: ScriptContext,
}

impl ScriptingEngine {
    /// Create a new scripting engine
    pub fn new(script_directory: PathBuf) -> Self {
        Self {
            scripts: Arc::new(RwLock::new(HashMap::new())),
            active_executions: Arc::new(RwLock::new(HashMap::new())),
            lua_runtime: Arc::new(RwLock::new(None)),
            python_runtime: Arc::new(RwLock::new(None)),
            js_runtime: Arc::new(RwLock::new(None)),
            script_directory,
        }
    }

    /// Initialize scripting runtimes
    pub async fn initialize(&self) -> Result<(), AppError> {
        // Initialize Lua runtime
        {
            let mut lua_runtime = self.lua_runtime.write().await;
            *lua_runtime = Some(LuaRuntime::new()?);
        }

        // Initialize Python runtime
        {
            let mut python_runtime = self.python_runtime.write().await;
            *python_runtime = Some(PythonRuntime::new()?);
        }

        // Initialize JavaScript runtime
        {
            let mut js_runtime = self.js_runtime.write().await;
            *js_runtime = Some(JavaScriptRuntime::new()?);
        }

        tracing::info!("Scripting engine initialized with Lua, Python, and JavaScript support");
        Ok(())
    }

    /// Load script from file
    pub async fn load_script(&self, script_path: &PathBuf) -> Result<String, AppError> {
        let content = tokio::fs::read_to_string(script_path).await
            .map_err(|e| AppError::Io(e))?;

        // Parse script metadata from comments
        let metadata = self.parse_script_metadata(&content, script_path)?;
        let script_id = metadata.id.clone();

        // Validate script
        self.validate_script(&metadata).await?;

        // Store script
        let mut scripts = self.scripts.write().await;
        scripts.insert(script_id.clone(), metadata);

        tracing::info!("Loaded script: {}", script_id);
        Ok(script_id)
    }

    /// Parse script metadata from source code comments
    fn parse_script_metadata(&self, content: &str, script_path: &PathBuf) -> Result<Script, AppError> {
        let language = self.detect_language(script_path)?;
        let script_id = uuid::Uuid::new_v4().to_string();

        // Extract metadata from comments (simplified parser)
        let mut name = script_path.file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("Unnamed Script")
            .to_string();
        let mut description = "No description".to_string();
        let mut author = "Unknown".to_string();
        let mut version = "1.0.0".to_string();
        let mut permissions = Vec::new();
        let mut triggers = Vec::new();

        for line in content.lines() {
            let line = line.trim();
            if line.starts_with("--") || line.starts_with("#") || line.starts_with("//") {
                if let Some(meta) = line.split_once("@") {
                    let (_, meta_content) = meta;
                    if let Some((key, value)) = meta_content.split_once(" ") {
                        match key {
                            "name" => name = value.to_string(),
                            "description" => description = value.to_string(),
                            "author" => author = value.to_string(),
                            "version" => version = value.to_string(),
                            "permission" => {
                                if let Ok(perm) = self.parse_permission(value) {
                                    permissions.push(perm);
                                }
                            }
                            "trigger" => {
                                if let Ok(trigger) = self.parse_trigger(value) {
                                    triggers.push(trigger);
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
        }

        Ok(Script {
            id: script_id,
            name,
            description,
            language,
            source_code: content.to_string(),
            entry_point: "main".to_string(),
            author,
            version,
            permissions,
            triggers,
            is_enabled: true,
            created_at: chrono::Utc::now(),
            last_modified: chrono::Utc::now(),
            execution_count: 0,
            last_error: None,
        })
    }

    /// Detect script language from file extension
    fn detect_language(&self, script_path: &PathBuf) -> Result<ScriptLanguage, AppError> {
        match script_path.extension().and_then(|ext| ext.to_str()) {
            Some("lua") => Ok(ScriptLanguage::Lua),
            Some("py") => Ok(ScriptLanguage::Python),
            Some("js") => Ok(ScriptLanguage::JavaScript),
            _ => Err(AppError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "Unsupported script language"
            )))
        }
    }

    /// Parse permission from string
    fn parse_permission(&self, perm_str: &str) -> Result<ScriptPermission, AppError> {
        if perm_str.starts_with("filesystem:") {
            Ok(ScriptPermission::FileSystem(perm_str[11..].to_string()))
        } else if perm_str.starts_with("network:") {
            Ok(ScriptPermission::Network(perm_str[8..].to_string()))
        } else {
            match perm_str {
                "system_commands" => Ok(ScriptPermission::SystemCommands),
                "voice_control" => Ok(ScriptPermission::VoiceControl),
                "gesture_control" => Ok(ScriptPermission::GestureControl),
                "ring_control" => Ok(ScriptPermission::RingControl),
                "notifications" => Ok(ScriptPermission::Notifications),
                "clipboard" => Ok(ScriptPermission::ClipboardAccess),
                "window_management" => Ok(ScriptPermission::WindowManagement),
                "database" => Ok(ScriptPermission::DatabaseAccess),
                _ => Err(AppError::Io(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    format!("Unknown permission: {}", perm_str)
                )))
            }
        }
    }

    /// Parse trigger from string
    fn parse_trigger(&self, trigger_str: &str) -> Result<ScriptTrigger, AppError> {
        if trigger_str.starts_with("voice:") {
            Ok(ScriptTrigger::VoiceCommand(trigger_str[6..].to_string()))
        } else if trigger_str.starts_with("gesture:") {
            Ok(ScriptTrigger::Gesture(trigger_str[8..].to_string()))
        } else if trigger_str.starts_with("schedule:") {
            Ok(ScriptTrigger::TimeSchedule(trigger_str[9..].to_string()))
        } else if trigger_str.starts_with("app:") {
            Ok(ScriptTrigger::ApplicationEvent(trigger_str[4..].to_string()))
        } else {
            Err(AppError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("Unknown trigger: {}", trigger_str)
            )))
        }
    }

    /// Validate script before loading
    async fn validate_script(&self, script: &Script) -> Result<(), AppError> {
        // Check for dangerous permissions
        for permission in &script.permissions {
            match permission {
                ScriptPermission::SystemCommands => {
                    tracing::warn!("Script '{}' requests system command access", script.name);
                }
                ScriptPermission::FileSystem(path) if path.contains("..") => {
                    return Err(AppError::Io(std::io::Error::new(
                        std::io::ErrorKind::PermissionDenied,
                        "Invalid file system permission"
                    )));
                }
                _ => {}
            }
        }

        // Basic syntax validation (simplified)
        match script.language {
            ScriptLanguage::Lua => {
                if !script.source_code.contains("function") && !script.source_code.contains("=") {
                    tracing::warn!("Lua script may have syntax issues");
                }
            }
            ScriptLanguage::Python => {
                if script.source_code.contains("import os") && 
                   !script.permissions.contains(&ScriptPermission::SystemCommands) {
                    return Err(AppError::Io(std::io::Error::new(
                        std::io::ErrorKind::PermissionDenied,
                        "Script uses 'os' module without system permission"
                    )));
                }
            }
            ScriptLanguage::JavaScript => {
                if script.source_code.contains("require(") && 
                   !script.permissions.contains(&ScriptPermission::SystemCommands) {
                    tracing::warn!("JavaScript script uses require() without system permission");
                }
            }
        }

        Ok(())
    }

    /// Execute a script
    pub async fn execute_script(&self, script_id: &str, context: ScriptContext) -> Result<ScriptExecutionResult, AppError> {
        let start_time = std::time::Instant::now();
        let execution_id = uuid::Uuid::new_v4().to_string();

        // Get script
        let scripts = self.scripts.read().await;
        let script = scripts.get(script_id)
            .ok_or_else(|| AppError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "Script not found"
            )))?
            .clone();

        if !script.is_enabled {
            return Err(AppError::Io(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "Script is disabled"
            )));
        }

        // Check permissions
        for required_perm in &script.permissions {
            if !context.permissions.contains(required_perm) {
                return Err(AppError::Io(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    format!("Missing permission: {:?}", required_perm)
                )));
            }
        }

        // Track execution
        let execution = ScriptExecution {
            execution_id: execution_id.clone(),
            script_id: script_id.to_string(),
            started_at: chrono::Utc::now(),
            context: context.clone(),
        };

        {
            let mut active = self.active_executions.write().await;
            active.insert(execution_id.clone(), execution);
        }

        drop(scripts);

        // Execute based on language
        let result = match script.language {
            ScriptLanguage::Lua => self.execute_lua_script(&script, &context).await,
            ScriptLanguage::Python => self.execute_python_script(&script, &context).await,
            ScriptLanguage::JavaScript => self.execute_js_script(&script, &context).await,
        };

        // Clean up execution tracking
        {
            let mut active = self.active_executions.write().await;
            active.remove(&execution_id);
        }

        // Update script statistics
        {
            let mut scripts_mut = self.scripts.write().await;
            if let Some(script_mut) = scripts_mut.get_mut(script_id) {
                script_mut.execution_count += 1;
                if let Err(ref error) = result {
                    script_mut.last_error = Some(error.to_string());
                }
            }
        }

        let execution_time = start_time.elapsed().as_millis() as u64;

        match result {
            Ok((return_value, output, warnings)) => {
                Ok(ScriptExecutionResult {
                    script_id: script_id.to_string(),
                    success: true,
                    return_value,
                    error_message: None,
                    execution_time_ms: execution_time,
                    output,
                    warnings,
                })
            }
            Err(error) => {
                Ok(ScriptExecutionResult {
                    script_id: script_id.to_string(),
                    success: false,
                    return_value: None,
                    error_message: Some(error.to_string()),
                    execution_time_ms: execution_time,
                    output: String::new(),
                    warnings: Vec::new(),
                })
            }
        }
    }

    /// Execute Lua script
    async fn execute_lua_script(&self, script: &Script, _context: &ScriptContext) -> Result<(Option<serde_json::Value>, String, Vec<String>), AppError> {
        let lua_runtime = self.lua_runtime.read().await;
        if let Some(runtime) = lua_runtime.as_ref() {
            runtime.execute(&script.source_code).await
        } else {
            Err(AppError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "Lua runtime not initialized"
            )))
        }
    }

    /// Execute Python script
    async fn execute_python_script(&self, script: &Script, _context: &ScriptContext) -> Result<(Option<serde_json::Value>, String, Vec<String>), AppError> {
        let python_runtime = self.python_runtime.read().await;
        if let Some(runtime) = python_runtime.as_ref() {
            runtime.execute(&script.source_code).await
        } else {
            Err(AppError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "Python runtime not initialized"
            )))
        }
    }

    /// Execute JavaScript script
    async fn execute_js_script(&self, script: &Script, _context: &ScriptContext) -> Result<(Option<serde_json::Value>, String, Vec<String>), AppError> {
        let js_runtime = self.js_runtime.read().await;
        if let Some(runtime) = js_runtime.as_ref() {
            runtime.execute(&script.source_code).await
        } else {
            Err(AppError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "JavaScript runtime not initialized"
            )))
        }
    }

    /// Get all scripts
    pub async fn get_scripts(&self) -> Vec<Script> {
        let scripts = self.scripts.read().await;
        scripts.values().cloned().collect()
    }

    /// Get script statistics
    pub async fn get_stats(&self) -> serde_json::Value {
        let scripts = self.scripts.read().await;
        let active = self.active_executions.read().await;

        let total_scripts = scripts.len();
        let enabled_scripts = scripts.values().filter(|s| s.is_enabled).count();
        let active_executions = active.len();
        let total_executions: u32 = scripts.values().map(|s| s.execution_count).sum();

        serde_json::json!({
            "total_scripts": total_scripts,
            "enabled_scripts": enabled_scripts,
            "active_executions": active_executions,
            "total_executions": total_executions
        })
    }
}

/// Lua runtime wrapper
struct LuaRuntime;

impl LuaRuntime {
    fn new() -> Result<Self, AppError> {
        // In real implementation, would initialize Lua interpreter
        Ok(Self)
    }

    async fn execute(&self, code: &str) -> Result<(Option<serde_json::Value>, String, Vec<String>), AppError> {
        // Simplified Lua execution
        tracing::info!("Executing Lua code: {}", &code[..code.len().min(50)]);
        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
        
        Ok((
            Some(serde_json::json!({"result": "lua_executed"})),
            "Lua script executed successfully".to_string(),
            Vec::new()
        ))
    }
}

/// Python runtime wrapper
struct PythonRuntime;

impl PythonRuntime {
    fn new() -> Result<Self, AppError> {
        // In real implementation, would initialize Python interpreter
        Ok(Self)
    }

    async fn execute(&self, code: &str) -> Result<(Option<serde_json::Value>, String, Vec<String>), AppError> {
        // Simplified Python execution
        tracing::info!("Executing Python code: {}", &code[..code.len().min(50)]);
        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
        
        Ok((
            Some(serde_json::json!({"result": "python_executed"})),
            "Python script executed successfully".to_string(),
            Vec::new()
        ))
    }
}

/// JavaScript runtime wrapper
struct JavaScriptRuntime;

impl JavaScriptRuntime {
    fn new() -> Result<Self, AppError> {
        // In real implementation, would initialize JavaScript engine (V8, QuickJS, etc.)
        Ok(Self)
    }

    async fn execute(&self, code: &str) -> Result<(Option<serde_json::Value>, String, Vec<String>), AppError> {
        // Simplified JavaScript execution
        tracing::info!("Executing JavaScript code: {}", &code[..code.len().min(50)]);
        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
        
        Ok((
            Some(serde_json::json!({"result": "js_executed"})),
            "JavaScript executed successfully".to_string(),
            Vec::new()
        ))
    }
}

/// Global scripting engine instance
static SCRIPTING_ENGINE: tokio::sync::OnceCell<ScriptingEngine> = tokio::sync::OnceCell::const_new();

/// Get the global scripting engine
pub async fn get_scripting_engine() -> &'static ScriptingEngine {
    SCRIPTING_ENGINE.get_or_init(|| async {
        let script_dir = std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join("scripts");
        ScriptingEngine::new(script_dir)
    }).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_script_loading() {
        let temp_dir = TempDir::new().unwrap();
        let engine = ScriptingEngine::new(temp_dir.path().to_path_buf());
        
        let script_path = temp_dir.path().join("test.lua");
        let script_content = r#"
-- @name Test Script
-- @description A test script
-- @author Test Author
-- @version 1.0.0
-- @permission voice_control
-- @trigger voice:test

function main()
    print("Hello from Lua!")
    return "success"
end
"#;
        
        tokio::fs::write(&script_path, script_content).await.unwrap();
        
        let script_id = engine.load_script(&script_path).await.unwrap();
        let scripts = engine.get_scripts().await;
        
        assert_eq!(scripts.len(), 1);
        assert_eq!(scripts[0].id, script_id);
        assert_eq!(scripts[0].name, "Test Script");
    }

    #[tokio::test]
    async fn test_script_execution() {
        let temp_dir = TempDir::new().unwrap();
        let engine = ScriptingEngine::new(temp_dir.path().to_path_buf());
        engine.initialize().await.unwrap();
        
        let script_path = temp_dir.path().join("test.lua");
        let script_content = "print('Hello World!')";
        
        tokio::fs::write(&script_path, script_content).await.unwrap();
        
        let script_id = engine.load_script(&script_path).await.unwrap();
        
        let context = ScriptContext {
            script_id: script_id.clone(),
            user_id: "user1".to_string(),
            session_id: "session1".to_string(),
            variables: HashMap::new(),
            permissions: vec![ScriptPermission::VoiceControl],
            execution_timeout: std::time::Duration::from_secs(30),
        };
        
        let result = engine.execute_script(&script_id, context).await.unwrap();
        assert!(result.success);
    }
}
