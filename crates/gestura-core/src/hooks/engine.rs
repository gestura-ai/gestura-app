//! Hook engine.

use std::collections::HashMap;
use std::sync::Arc;

use tokio::time::timeout;

use crate::error::{AppError, Result};

use super::executor::{HookExecutor, HookOutput, ProcessHookExecutor};
use super::template::{TemplateVars, render_template};
use super::types::{HookContext, HookEvent, HooksSettings};

/// A record of a hook execution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookExecutionRecord {
    /// Hook name.
    pub name: String,
    /// Event emitted.
    pub event: HookEvent,
    /// Rendered program.
    pub program: String,
    /// Rendered args.
    pub args: Vec<String>,
    /// Execution output.
    pub output: HookOutput,
}

/// Hook engine.
///
/// The engine is safe-by-default:
/// - disabled unless `settings.enabled == true`
/// - refuses to execute programs not present in `settings.allowed_programs`
pub struct HookEngine {
    settings: HooksSettings,
    executor: Arc<dyn HookExecutor>,
}

impl HookEngine {
    /// Create a hook engine using the default OS-process executor.
    pub fn new(settings: HooksSettings) -> Self {
        Self {
            settings,
            executor: Arc::new(ProcessHookExecutor),
        }
    }

    /// Create a hook engine with a custom executor.
    pub fn new_with_executor(settings: HooksSettings, executor: Arc<dyn HookExecutor>) -> Self {
        Self { settings, executor }
    }

    /// Execute all hooks registered for `event`.
    pub async fn run(
        &self,
        event: HookEvent,
        ctx: &HookContext,
    ) -> Result<Vec<HookExecutionRecord>> {
        if !self.settings.enabled {
            return Ok(Vec::new());
        }

        let vars = context_to_vars(ctx);
        let mut records = Vec::new();

        for hook in self.settings.hooks.iter().filter(|h| h.event == event) {
            let program = render_template(&hook.command.program, &vars);
            let args: Vec<String> = hook
                .command
                .args
                .iter()
                .map(|a| render_template(a, &vars))
                .collect();

            if !self.settings.allowed_programs.iter().any(|p| p == &program) {
                return Err(AppError::PermissionDenied(format!(
                    "Hook '{}' attempted to execute disallowed program '{program}'",
                    hook.name
                )));
            }

            let cwd = ctx.workspace_dir.as_deref();
            let output = timeout(
                std::time::Duration::from_millis(self.settings.timeout_ms),
                self.executor
                    .execute(&program, &args, cwd, self.settings.max_output_bytes),
            )
            .await
            .map_err(|_| {
                AppError::Timeout(format!(
                    "Hook '{}' exceeded timeout ({}ms)",
                    hook.name, self.settings.timeout_ms
                ))
            })??;

            records.push(HookExecutionRecord {
                name: hook.name.clone(),
                event,
                program,
                args,
                output,
            });
        }

        Ok(records)
    }
}

/// Convert [`HookContext`] into template variables.
///
/// Keys are intentionally flat and stable.
fn context_to_vars(ctx: &HookContext) -> TemplateVars {
    let mut vars: HashMap<String, String> = HashMap::new();
    if let Some(id) = &ctx.session_id {
        vars.insert("session_id".to_string(), id.clone());
    }
    if let Some(name) = &ctx.tool_name {
        vars.insert("tool_name".to_string(), name.clone());
    }
    if let Some(args) = &ctx.tool_arguments_json {
        vars.insert("tool_args".to_string(), args.clone());
    }
    if let Some(success) = ctx.tool_success {
        vars.insert("tool_success".to_string(), success.to_string());
    }
    if let Some(out) = &ctx.tool_output {
        vars.insert("tool_output".to_string(), out.clone());
    }
    if let Some(prompt) = &ctx.pipeline_prompt {
        vars.insert("pipeline_prompt".to_string(), prompt.clone());
    }
    vars
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hooks::types::{HookCommandTemplate, HookDefinition, HookEvent, HooksSettings};

    #[tokio::test]
    async fn engine_skips_when_disabled() {
        let settings = HooksSettings::default();
        let engine = HookEngine::new(settings);
        let ctx = HookContext::default();
        let out = engine.run(HookEvent::PrePipeline, &ctx).await.unwrap();
        assert!(out.is_empty());
    }

    #[tokio::test]
    async fn engine_denies_disallowed_program() {
        let mut settings = HooksSettings {
            enabled: true,
            ..Default::default()
        };
        settings.hooks.push(HookDefinition {
            name: "deny".to_string(),
            event: HookEvent::PrePipeline,
            command: HookCommandTemplate {
                program: "sh".to_string(),
                args: vec!["-c".to_string(), "echo hi".to_string()],
            },
        });

        let engine = HookEngine::new(settings);
        let ctx = HookContext::default();
        let err = engine.run(HookEvent::PrePipeline, &ctx).await.unwrap_err();
        assert!(matches!(err, AppError::PermissionDenied(_)));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn engine_executes_allowed_program_and_renders_templates() {
        let mut settings = HooksSettings {
            enabled: true,
            allowed_programs: vec!["sh".to_string()],
            ..Default::default()
        };
        settings.hooks.push(HookDefinition {
            name: "echo-tool".to_string(),
            event: HookEvent::PreTool,
            command: HookCommandTemplate {
                program: "sh".to_string(),
                args: vec!["-c".to_string(), "printf %s {{tool_name}}".to_string()],
            },
        });

        let engine = HookEngine::new(settings);
        let ctx = HookContext {
            tool_name: Some("git".to_string()),
            ..Default::default()
        };

        let out = engine.run(HookEvent::PreTool, &ctx).await.unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].output.stdout, "git");
    }
}
