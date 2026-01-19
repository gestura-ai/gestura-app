//! MCP Prompts - Templated messages and workflows
//! Provides prompt definitions for voice commands and common workflows.

use super::types::{
    Prompt, PromptArgument, PromptContent, PromptMessage, PromptRole, PromptsGetResult, TextContent,
};
use std::collections::HashMap;

/// Prompt registry for managing available prompts
#[derive(Debug, Default)]
pub struct PromptRegistry {
    prompts: HashMap<String, RegisteredPrompt>,
}

/// A registered prompt with its handler
#[derive(Debug, Clone)]
pub struct RegisteredPrompt {
    pub definition: Prompt,
    pub template: String,
}

impl PromptRegistry {
    /// Create a new prompt registry with default prompts
    pub fn new() -> Self {
        let mut registry = Self::default();
        registry.register_default_prompts();
        registry
    }

    /// Register default voice command prompts
    fn register_default_prompts(&mut self) {
        // Voice command prompt
        self.register(RegisteredPrompt {
            definition: Prompt {
                name: "voice-command".to_string(),
                description: Some(
                    "Process a voice command and generate an appropriate response".to_string(),
                ),
                arguments: Some(vec![
                    PromptArgument {
                        name: "command".to_string(),
                        description: Some("The transcribed voice command".to_string()),
                        required: true,
                    },
                    PromptArgument {
                        name: "context".to_string(),
                        description: Some("Additional context about the user's environment".to_string()),
                        required: false,
                    },
                ]),
            },
            template: "You are a voice assistant. Process this command: {{command}}\n\nContext: {{context}}".to_string(),
        });

        // Haptic feedback prompt
        self.register(RegisteredPrompt {
            definition: Prompt {
                name: "haptic-feedback".to_string(),
                description: Some(
                    "Generate haptic feedback pattern based on notification type".to_string(),
                ),
                arguments: Some(vec![
                    PromptArgument {
                        name: "notification_type".to_string(),
                        description: Some(
                            "Type of notification (alert, message, reminder)".to_string(),
                        ),
                        required: true,
                    },
                    PromptArgument {
                        name: "urgency".to_string(),
                        description: Some("Urgency level (low, medium, high)".to_string()),
                        required: false,
                    },
                ]),
            },
            template:
                "Generate a haptic pattern for: {{notification_type}} with urgency: {{urgency}}"
                    .to_string(),
        });

        // Code assistance prompt
        self.register(RegisteredPrompt {
            definition: Prompt {
                name: "code-assist".to_string(),
                description: Some("Assist with code-related tasks via voice".to_string()),
                arguments: Some(vec![
                    PromptArgument {
                        name: "task".to_string(),
                        description: Some("The coding task to perform".to_string()),
                        required: true,
                    },
                    PromptArgument {
                        name: "language".to_string(),
                        description: Some("Programming language".to_string()),
                        required: false,
                    },
                    PromptArgument {
                        name: "file_context".to_string(),
                        description: Some("Current file or code context".to_string()),
                        required: false,
                    },
                ]),
            },
            template: "Help with this coding task: {{task}}\nLanguage: {{language}}\nContext: {{file_context}}".to_string(),
        });

        // System control prompt
        self.register(RegisteredPrompt {
            definition: Prompt {
                name: "system-control".to_string(),
                description: Some("Control system settings and preferences".to_string()),
                arguments: Some(vec![PromptArgument {
                    name: "action".to_string(),
                    description: Some("The system action to perform".to_string()),
                    required: true,
                }]),
            },
            template: "Execute system control action: {{action}}".to_string(),
        });
    }

    /// Register a prompt
    pub fn register(&mut self, prompt: RegisteredPrompt) {
        self.prompts.insert(prompt.definition.name.clone(), prompt);
    }

    /// List all prompts
    pub fn list(&self) -> Vec<Prompt> {
        self.prompts
            .values()
            .map(|p| p.definition.clone())
            .collect()
    }

    /// Get a prompt by name and render with arguments
    pub fn get(
        &self,
        name: &str,
        arguments: Option<&HashMap<String, String>>,
    ) -> Option<PromptsGetResult> {
        let registered = self.prompts.get(name)?;

        // Render template with arguments
        let mut rendered = registered.template.clone();
        if let Some(args) = arguments {
            for (key, value) in args {
                rendered = rendered.replace(&format!("{{{{{}}}}}", key), value);
            }
        }
        // Replace any remaining placeholders with empty string
        rendered = regex::Regex::new(r"\{\{[^}]+\}\}")
            .ok()?
            .replace_all(&rendered, "")
            .to_string();

        Some(PromptsGetResult {
            description: registered.definition.description.clone(),
            messages: vec![PromptMessage {
                role: PromptRole::User,
                content: PromptContent::Text(TextContent::new(rendered)),
            }],
        })
    }

    /// Check if a prompt exists
    pub fn contains(&self, name: &str) -> bool {
        self.prompts.contains_key(name)
    }
}
