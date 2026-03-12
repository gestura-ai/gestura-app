//! Shared slash-command metadata for the interactive agent surfaces.

/// How a slash command should be presented in interactive shells.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlashSurface {
    /// Root command opens a managed shell/browser; explicit verbs still work.
    RootShell,
    /// Direct command executes immediately.
    Direct,
}

/// High-level help grouping for basic-mode help rendering.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HelpSection {
    Conversation,
    Shells,
    Utilities,
}

/// Declarative metadata for a slash command or command template.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SlashCommandSpec {
    pub command: &'static str,
    pub description: &'static str,
    pub surface: SlashSurface,
    pub help_section: HelpSection,
}

pub const SLASH_COMMANDS: &[SlashCommandSpec] = &[
    SlashCommandSpec {
        command: "/help",
        description: "Show help and keyboard shortcuts",
        surface: SlashSurface::Direct,
        help_section: HelpSection::Conversation,
    },
    SlashCommandSpec {
        command: "/clear",
        description: "Clear the terminal screen",
        surface: SlashSurface::Direct,
        help_section: HelpSection::Conversation,
    },
    SlashCommandSpec {
        command: "/save",
        description: "Save the current session",
        surface: SlashSurface::Direct,
        help_section: HelpSection::Conversation,
    },
    SlashCommandSpec {
        command: "/history",
        description: "Show session statistics",
        surface: SlashSurface::Direct,
        help_section: HelpSection::Conversation,
    },
    SlashCommandSpec {
        command: "/summarize",
        description: "Summarize conversation history",
        surface: SlashSurface::Direct,
        help_section: HelpSection::Conversation,
    },
    SlashCommandSpec {
        command: "/new",
        description: "Start a fresh session",
        surface: SlashSurface::Direct,
        help_section: HelpSection::Conversation,
    },
    SlashCommandSpec {
        command: "/listen",
        description: "Toggle listening mode",
        surface: SlashSurface::Direct,
        help_section: HelpSection::Conversation,
    },
    SlashCommandSpec {
        command: "/voice",
        description: "Record one voice input",
        surface: SlashSurface::Direct,
        help_section: HelpSection::Conversation,
    },
    SlashCommandSpec {
        command: "/exec <prompt>",
        description: "Execute a prompt without slash detection",
        surface: SlashSurface::Direct,
        help_section: HelpSection::Conversation,
    },
    SlashCommandSpec {
        command: "/pause",
        description: "Pause the current task",
        surface: SlashSurface::Direct,
        help_section: HelpSection::Conversation,
    },
    SlashCommandSpec {
        command: "/resume",
        description: "Resume a paused task",
        surface: SlashSurface::Direct,
        help_section: HelpSection::Conversation,
    },
    SlashCommandSpec {
        command: "/continue",
        description: "Alias for /resume",
        surface: SlashSurface::Direct,
        help_section: HelpSection::Conversation,
    },
    SlashCommandSpec {
        command: "/stop",
        description: "Stop the current response",
        surface: SlashSurface::Direct,
        help_section: HelpSection::Conversation,
    },
    SlashCommandSpec {
        command: "/quit",
        description: "Exit the agent",
        surface: SlashSurface::Direct,
        help_section: HelpSection::Conversation,
    },
    SlashCommandSpec {
        command: "/tools [name]",
        description: "Inspect or manage built-in tools",
        surface: SlashSurface::RootShell,
        help_section: HelpSection::Shells,
    },
    SlashCommandSpec {
        command: "/mcp [status|list|tools|get|add|remove|enable|disable|connect|disconnect]",
        description: "Open the MCP server shell",
        surface: SlashSurface::RootShell,
        help_section: HelpSection::Shells,
    },
    SlashCommandSpec {
        command: "/hooks [list|show|enable|disable|new|edit|delete|allowlist]",
        description: "Open the hooks shell",
        surface: SlashSurface::RootShell,
        help_section: HelpSection::Shells,
    },
    SlashCommandSpec {
        command: "/memory [list|search|save|clear|delete]",
        description: "Open the memory shell",
        surface: SlashSurface::RootShell,
        help_section: HelpSection::Shells,
    },
    SlashCommandSpec {
        command: "/session [info|list|load|delete|export]",
        description: "Open the session shell",
        surface: SlashSurface::RootShell,
        help_section: HelpSection::Shells,
    },
    SlashCommandSpec {
        command: "/tasks [list|show|new|done|current|clear-current|delete]",
        description: "Open the task shell",
        surface: SlashSurface::RootShell,
        help_section: HelpSection::Shells,
    },
    SlashCommandSpec {
        command: "/workflow [list|run <name>]",
        description: "Open the workflow shell",
        surface: SlashSurface::RootShell,
        help_section: HelpSection::Shells,
    },
    SlashCommandSpec {
        command: "/config [list|get|set|keys|path|reset]",
        description: "Open the configuration shell",
        surface: SlashSurface::RootShell,
        help_section: HelpSection::Shells,
    },
    SlashCommandSpec {
        command: "/context [session|status|analyze|categories|clear]",
        description: "Open the context shell",
        surface: SlashSurface::RootShell,
        help_section: HelpSection::Shells,
    },
    SlashCommandSpec {
        command: "/a2a [status|profiles|agents|discover|register|token|validate|send]",
        description: "Open the A2A shell",
        surface: SlashSurface::RootShell,
        help_section: HelpSection::Shells,
    },
    SlashCommandSpec {
        command: "/knowledge [list|search|categories|show|enable|disable]",
        description: "Open the knowledge shell",
        surface: SlashSurface::RootShell,
        help_section: HelpSection::Shells,
    },
    SlashCommandSpec {
        command: "/agent [status|config]",
        description: "Open the agent shell",
        surface: SlashSurface::RootShell,
        help_section: HelpSection::Shells,
    },
    SlashCommandSpec {
        command: "/device [list]",
        description: "Open the device shell",
        surface: SlashSurface::RootShell,
        help_section: HelpSection::Shells,
    },
    SlashCommandSpec {
        command: "/permissions [list|grant|revoke|reset|level]",
        description: "Open the permissions shell",
        surface: SlashSurface::RootShell,
        help_section: HelpSection::Shells,
    },
    SlashCommandSpec {
        command: "/privacy [status|report|policy|export|delete]",
        description: "Open the privacy shell",
        surface: SlashSurface::RootShell,
        help_section: HelpSection::Shells,
    },
    SlashCommandSpec {
        command: "/theme [name]",
        description: "Open the theme picker or set a theme",
        surface: SlashSurface::RootShell,
        help_section: HelpSection::Shells,
    },
    SlashCommandSpec {
        command: "/model [provider:model]",
        description: "Open the model picker or set an override",
        surface: SlashSurface::RootShell,
        help_section: HelpSection::Shells,
    },
    SlashCommandSpec {
        command: "/health",
        description: "Run health diagnostics",
        surface: SlashSurface::Direct,
        help_section: HelpSection::Utilities,
    },
    SlashCommandSpec {
        command: "/init",
        description: "Run the first-time setup wizard",
        surface: SlashSurface::Direct,
        help_section: HelpSection::Utilities,
    },
];

pub const HELP_SECTION_ORDER: &[HelpSection] = &[
    HelpSection::Conversation,
    HelpSection::Shells,
    HelpSection::Utilities,
];

pub fn section_title(section: HelpSection) -> &'static str {
    match section {
        HelpSection::Conversation => "Conversation",
        HelpSection::Shells => "Managed shells",
        HelpSection::Utilities => "Utilities",
    }
}

/// Normalize interactive root-command aliases to a canonical slash token.
///
/// The input is expected to already be trimmed and lowercased.
pub fn canonical_command(command: &str) -> &str {
    match command {
        "/q" | "/exit" => "/quit",
        "/?" => "/help",
        "/continue" => "/resume",
        "/find" => "/search",
        "/sessions" => "/session",
        "/workflows" => "/workflow",
        "/themes" => "/theme",
        "/hook" => "/hooks",
        "/permission" => "/permissions",
        "/task" => "/tasks",
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn command_spec(command: &str) -> &'static SlashCommandSpec {
        SLASH_COMMANDS
            .iter()
            .find(|spec| spec.command == command)
            .unwrap_or_else(|| panic!("missing slash command spec for {command}"))
    }

    #[test]
    fn managed_shell_commands_are_classified_as_root_shells() {
        for command in [
            "/tools [name]",
            "/mcp [status|list|tools|get|add|remove|enable|disable|connect|disconnect]",
            "/hooks [list|show|enable|disable|new|edit|delete|allowlist]",
            "/memory [list|search|save|clear|delete]",
            "/session [info|list|load|delete|export]",
            "/tasks [list|show|new|done|current|clear-current|delete]",
            "/workflow [list|run <name>]",
            "/config [list|get|set|keys|path|reset]",
            "/context [session|status|analyze|categories|clear]",
            "/a2a [status|profiles|agents|discover|register|token|validate|send]",
            "/knowledge [list|search|categories|show|enable|disable]",
            "/agent [status|config]",
            "/device [list]",
            "/permissions [list|grant|revoke|reset|level]",
            "/privacy [status|report|policy|export|delete]",
            "/theme [name]",
            "/model [provider:model]",
        ] {
            assert_eq!(command_spec(command).surface, SlashSurface::RootShell);
        }
    }

    #[test]
    fn quick_action_commands_are_classified_as_direct() {
        for command in [
            "/help",
            "/clear",
            "/save",
            "/history",
            "/summarize",
            "/new",
            "/listen",
            "/voice",
            "/exec <prompt>",
            "/pause",
            "/resume",
            "/continue",
            "/stop",
            "/quit",
            "/health",
            "/init",
        ] {
            assert_eq!(command_spec(command).surface, SlashSurface::Direct);
        }
    }

    #[test]
    fn canonical_command_normalizes_common_aliases() {
        assert_eq!(canonical_command("/q"), "/quit");
        assert_eq!(canonical_command("/exit"), "/quit");
        assert_eq!(canonical_command("/?"), "/help");
        assert_eq!(canonical_command("/continue"), "/resume");
        assert_eq!(canonical_command("/find"), "/search");
        assert_eq!(canonical_command("/sessions"), "/session");
        assert_eq!(canonical_command("/workflows"), "/workflow");
        assert_eq!(canonical_command("/themes"), "/theme");
        assert_eq!(canonical_command("/hook"), "/hooks");
        assert_eq!(canonical_command("/permission"), "/permissions");
        assert_eq!(canonical_command("/task"), "/tasks");
        assert_eq!(canonical_command("/config"), "/config");
    }
}
