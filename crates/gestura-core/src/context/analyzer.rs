//! Request analyzer for determining needed context
//!
//! Analyzes user requests to determine what tools and context are needed,
//! without calling the LLM.

use super::types::{ContextCategory, EntityType, ExtractedEntity, RequestAnalysis};
use regex::Regex;
use std::collections::HashMap;
use std::sync::LazyLock;

/// Patterns for detecting context categories
struct CategoryPattern {
    keywords: &'static [&'static str],
    phrases: &'static [&'static str],
    category: ContextCategory,
}

const CATEGORY_PATTERNS: &[CategoryPattern] = &[
    CategoryPattern {
        keywords: &[
            "file", "read", "write", "edit", "create", "delete", "save", "open", "path",
        ],
        phrases: &[
            "show me",
            "look at",
            "what's in",
            "create a",
            "edit the",
            "modify",
        ],
        category: ContextCategory::FileSystem,
    },
    CategoryPattern {
        keywords: &[
            "run", "execute", "shell", "command", "terminal", "bash", "sh", "npm", "cargo",
        ],
        phrases: &["run this", "execute the", "in terminal", "run the"],
        category: ContextCategory::Shell,
    },
    CategoryPattern {
        keywords: &[
            "git", "commit", "branch", "merge", "push", "pull", "diff", "log", "status",
        ],
        phrases: &[
            "commit the",
            "push to",
            "pull from",
            "merge into",
            "git status",
        ],
        category: ContextCategory::Git,
    },
    CategoryPattern {
        keywords: &[
            "code", "function", "class", "struct", "impl", "method", "variable", "symbol",
        ],
        phrases: &[
            "find the",
            "where is",
            "definition of",
            "references to",
            "usage of",
        ],
        category: ContextCategory::Code,
    },
    CategoryPattern {
        keywords: &[
            "search", "web", "google", "url", "fetch", "download", "http", "api",
        ],
        phrases: &["search for", "look up", "find online", "on the web"],
        category: ContextCategory::Web,
    },
    CategoryPattern {
        keywords: &[
            "voice",
            "speak",
            "listen",
            "audio",
            "microphone",
            "transcribe",
            "whisper",
        ],
        phrases: &["say this", "read aloud", "voice command", "start listening"],
        category: ContextCategory::Voice,
    },
    CategoryPattern {
        keywords: &["config", "setting", "configure", "preference", "option"],
        phrases: &["change the", "set the", "update config", "configure the"],
        category: ContextCategory::Config,
    },
    CategoryPattern {
        keywords: &[
            "session", "history", "previous", "earlier", "last", "before",
        ],
        phrases: &["what did", "earlier we", "last time", "in this session"],
        category: ContextCategory::Session,
    },
    CategoryPattern {
        keywords: &["tool", "tools", "capability", "available", "can you"],
        phrases: &["what tools", "show tools", "list tools", "available tools"],
        category: ContextCategory::Tools,
    },
    CategoryPattern {
        keywords: &["agent", "delegate", "orchestrate", "supervisor", "worker"],
        phrases: &["delegate to", "have an agent", "multi-agent"],
        category: ContextCategory::Agent,
    },
    CategoryPattern {
        keywords: &["mcp", "protocol", "server", "client", "capability"],
        phrases: &["mcp server", "protocol message", "mcp client"],
        category: ContextCategory::Mcp,
    },
    CategoryPattern {
        keywords: &[
            "a2a",
            "agent-to-agent",
            "remote agent",
            "agent communication",
        ],
        phrases: &["send to agent", "agent profile", "a2a protocol"],
        category: ContextCategory::A2a,
    },
];

/// Compiled regex for file path extraction.
/// Matches paths like: src/main.rs, ./config.json, ~/Documents/file.txt
static FILE_PATH_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?:^|[\s\(\[])([./~]?(?:[\w-]+/)*[\w.-]+\.[a-zA-Z0-9]+)(?:[\s\)\]]|$)")
        .expect("Invalid file path regex")
});

/// Compiled regex for URL extraction.
/// Matches HTTP/HTTPS URLs with optional paths and query strings.
static URL_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"https?://[\w.-]+(?:/[\w./?%&=-]*)?").expect("Invalid URL regex"));

/// Compiled regex for git branch extraction.
/// Matches common branch naming patterns: main, master, develop, feature/*, bugfix/*, release/*
static GIT_BRANCH_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b(?:main|master|develop|feature/[\w-]+|bugfix/[\w-]+|release/[\w-]+)\b")
        .expect("Invalid git branch regex")
});

/// Analyzes requests to determine what context is needed
pub struct RequestAnalyzer {
    /// Tool name to category mapping
    tool_categories: HashMap<String, ContextCategory>,
    /// Follow-up indicators
    followup_patterns: Vec<&'static str>,
}

impl RequestAnalyzer {
    /// Create a new analyzer
    pub fn new() -> Self {
        let mut tool_categories = HashMap::new();
        // File tools
        for tool in [
            "read_file",
            "write_file",
            "list_directory",
            "search_files",
            "tree",
        ] {
            tool_categories.insert(tool.to_string(), ContextCategory::FileSystem);
        }
        // Shell tools
        for tool in ["run_command", "shell", "execute"] {
            tool_categories.insert(tool.to_string(), ContextCategory::Shell);
        }
        // Git tools
        for tool in ["git_status", "git_log", "git_diff", "git_branch"] {
            tool_categories.insert(tool.to_string(), ContextCategory::Git);
        }

        Self {
            tool_categories,
            followup_patterns: vec![
                "and also",
                "also",
                "additionally",
                "another thing",
                "one more",
                "what about",
                "how about",
                "can you also",
                "please also",
            ],
        }
    }

    /// Analyze a request and determine what context is needed
    pub fn analyze(&self, request: &str) -> RequestAnalysis {
        let lower = request.to_lowercase();
        let mut analysis = RequestAnalysis::new(request);

        // Detect categories based on patterns
        for pattern in CATEGORY_PATTERNS {
            let mut score = 0;
            for kw in pattern.keywords {
                if lower.contains(kw) {
                    score += 1;
                }
            }
            for phrase in pattern.phrases {
                if lower.contains(phrase) {
                    score += 2;
                }
            }
            if score > 0 {
                analysis.categories.insert(pattern.category);
                analysis.confidence += score as f32 * 0.1;
            }
        }

        // Extract entities
        self.extract_entities(request, &mut analysis);

        // Check for follow-up
        for pattern in &self.followup_patterns {
            if lower.contains(pattern) {
                analysis.is_followup = true;
                analysis.categories.insert(ContextCategory::Session);
                break;
            }
        }

        // Suggest tools based on categories
        for category in &analysis.categories {
            for (tool, cat) in &self.tool_categories {
                if cat == category && !analysis.suggested_tools.contains(tool) {
                    analysis.suggested_tools.push(tool.clone());
                }
            }
        }

        analysis.needs_tools = !analysis.categories.is_empty()
            && !analysis.categories.contains(&ContextCategory::General);

        // If no categories detected, it's general conversation
        if analysis.categories.is_empty() {
            analysis.categories.insert(ContextCategory::General);
            analysis.confidence = 0.8;
        }

        // Clamp confidence
        analysis.confidence = analysis.confidence.min(1.0);

        analysis
    }

    /// Extract entities from the request using regex patterns.
    /// Uses compiled regex patterns for accurate extraction of file paths, URLs, and git branches.
    fn extract_entities(&self, request: &str, analysis: &mut RequestAnalysis) {
        // Track already-extracted positions to avoid duplicates
        let mut extracted_ranges: Vec<(usize, usize)> = Vec::new();

        // Extract URLs using regex (do this first to avoid false positives in file paths)
        for cap in URL_REGEX.find_iter(request) {
            let start = cap.start();
            let end = cap.end();
            if !Self::overlaps_any(&extracted_ranges, start, end) {
                analysis.entities.push(ExtractedEntity {
                    entity_type: EntityType::Url,
                    value: cap.as_str().to_string(),
                    start,
                    end,
                });
                analysis.categories.insert(ContextCategory::Web);
                extracted_ranges.push((start, end));
            }
        }

        // Extract file paths using regex
        for cap in FILE_PATH_REGEX.captures_iter(request) {
            if let Some(m) = cap.get(1) {
                let start = m.start();
                let end = m.end();
                let value = m.as_str();

                // Skip if already extracted (e.g., as part of a URL)
                if Self::overlaps_any(&extracted_ranges, start, end) {
                    continue;
                }

                // Determine if it's a file or directory
                let entity_type = if value.ends_with('/') {
                    EntityType::DirectoryPath
                } else {
                    EntityType::FilePath
                };

                analysis.entities.push(ExtractedEntity {
                    entity_type,
                    value: value.to_string(),
                    start,
                    end,
                });
                analysis.categories.insert(ContextCategory::FileSystem);
                extracted_ranges.push((start, end));
            }
        }

        // Extract git branches using regex
        for cap in GIT_BRANCH_REGEX.find_iter(request) {
            let start = cap.start();
            let end = cap.end();
            if !Self::overlaps_any(&extracted_ranges, start, end) {
                analysis.entities.push(ExtractedEntity {
                    entity_type: EntityType::GitBranch,
                    value: cap.as_str().to_string(),
                    start,
                    end,
                });
                analysis.categories.insert(ContextCategory::Git);
                extracted_ranges.push((start, end));
            }
        }

        // Fallback: simple word-based extraction for paths not caught by regex
        // (e.g., paths without extensions like "src/lib")
        for word in request.split_whitespace() {
            if let Some(start) = request.find(word) {
                let end = start + word.len();

                // Skip if already extracted
                if Self::overlaps_any(&extracted_ranges, start, end) {
                    continue;
                }

                // Check for directory paths (contains / but no extension)
                if word.contains('/')
                    && !word.starts_with("http")
                    && !word.contains('.')
                    && word.len() > 2
                {
                    analysis.entities.push(ExtractedEntity {
                        entity_type: EntityType::DirectoryPath,
                        value: word.to_string(),
                        start,
                        end,
                    });
                    analysis.categories.insert(ContextCategory::FileSystem);
                    extracted_ranges.push((start, end));
                }
            }
        }
    }

    /// Check if a range overlaps with any existing ranges
    fn overlaps_any(ranges: &[(usize, usize)], start: usize, end: usize) -> bool {
        // Two ranges [start, end) and [s, e) overlap if start < e && end > s
        ranges.iter().any(|(s, e)| start < *e && end > *s)
    }
}

impl Default for RequestAnalyzer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_file_request_analysis() {
        let analyzer = RequestAnalyzer::new();
        let analysis = analyzer.analyze("Read the file src/main.rs and show me its contents");

        assert!(analysis.categories.contains(&ContextCategory::FileSystem));
        assert!(!analysis.entities.is_empty());
        assert!(analysis.needs_tools);
    }

    #[test]
    fn test_git_request_analysis() {
        let analyzer = RequestAnalyzer::new();
        let analysis = analyzer.analyze("Show me the git status and recent commits");

        assert!(analysis.categories.contains(&ContextCategory::Git));
        assert!(analysis.needs_tools);
    }

    #[test]
    fn test_general_conversation() {
        let analyzer = RequestAnalyzer::new();
        let analysis = analyzer.analyze("What is the meaning of life?");

        assert!(analysis.categories.contains(&ContextCategory::General));
        assert!(!analysis.needs_tools);
    }

    #[test]
    fn test_url_extraction() {
        let analyzer = RequestAnalyzer::new();
        let analysis = analyzer.analyze("Fetch https://example.com/api");

        assert!(analysis.categories.contains(&ContextCategory::Web));
        assert!(
            analysis
                .entities
                .iter()
                .any(|e| e.entity_type == EntityType::Url)
        );
    }

    #[test]
    fn test_followup_detection() {
        let analyzer = RequestAnalyzer::new();
        let analysis = analyzer.analyze("And also show me the tests");

        assert!(analysis.is_followup);
        assert!(analysis.categories.contains(&ContextCategory::Session));
    }
}
