//! Request analyzer for determining needed context
//!
//! Analyzes user requests to determine what tools and context are needed,
//! without calling the LLM.

use gestura_core_foundation::context::{
    ContextCategory, EntityType, ExtractedEntity, RequestAnalysis,
};
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
            "build", "test", "compile", "check", "scaffold",
        ],
        phrases: &[
            "run this",
            "execute the",
            "in terminal",
            "run the",
            "build and test",
            "build it",
            "run tests",
            "compile it",
            "scaffold the",
        ],
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
            "search", "web", "google", "url", "fetch", "download", "http", "api", "lookup",
            "browse", "website", "page", "online", "internet",
            // Natural-language synonyms for "retrieve from the web"
            "locate", "retrieve", "navigate", "domain", "link",
        ],
        phrases: &[
            "search for",
            "look up",
            "lookup",
            "find online",
            "on the web",
            "browse to",
            "visit",
            "check the",
            "go to",
            "open the",
            // Additional natural-language patterns
            "locate the",
            "retrieve the",
            "retrieve from",
            "navigate to",
            "from the web",
            "on the site",
        ],
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
    CategoryPattern {
        keywords: &["task", "todo", "track", "checklist", "reminder"],
        phrases: &[
            "add a task",
            "create a task",
            "task list",
            "my tasks",
            "mark as done",
            "complete this task",
        ],
        category: ContextCategory::Task,
    },
    CategoryPattern {
        keywords: &[
            "screenshot",
            "screen_record",
            "record",
            "video",
            "capture",
            "recording",
            "screencast",
            "screengrab",
        ],
        phrases: &[
            "take a screenshot",
            "record the screen",
            "record yourself",
            "create a video",
            "make a video",
            "screen capture",
            "screen recording",
            "record a video",
            "capture the screen",
            "video of yourself",
            "video of the screen",
        ],
        category: ContextCategory::Screen,
    },
];

/// Compiled regex for file path extraction.
/// Matches paths like: src/main.rs, ./config.yaml, ~/Documents/file.txt, allowing trailing punctuation
static FILE_PATH_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?:^|[\s\(\[])([./~]?(?:[\w-]+/)*[\w.-]+\.[a-zA-Z0-9]+)(?:[\s\)\].,;:!?]|$)")
        .expect("Invalid file path regex")
});

/// Well-known project root files that must be detected by name alone even when
/// no explicit path prefix (`./`, `../`, `/`) is present in the request.
const WELL_KNOWN_FILES: &[&str] = &[
    "AGENTS.md",
    "CLAUDE.md",
    ".cursorrules",
    "README.md",
    "README.rst",
    "README.txt",
    "CONTRIBUTING.md",
    "CHANGELOG.md",
    "Cargo.toml",
    "Cargo.lock",
    "package.json",
    "package-lock.json",
    "yarn.lock",
    "pnpm-lock.yaml",
    ".gitignore",
    ".env",
    ".env.example",
    "Makefile",
    "Justfile",
    "justfile",
    "Dockerfile",
    "docker-compose.yml",
    "docker-compose.yaml",
    "go.mod",
    "go.sum",
    "pyproject.toml",
    "requirements.txt",
    "tsconfig.json",
    "vite.config.ts",
    "vitest.config.ts",
    "eslint.config.js",
    ".eslintrc.json",
];

/// Compiled regex for bare filenames (e.g. `AGENTS.md`, `config.yaml`) that
/// lack an explicit path prefix but carry a file extension.  Anchored on
/// word-like boundaries so we don't misfire inside URLs or longer paths.
static BARE_FILENAME_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?:^|[\s\(\[,;'"\`])([A-Za-z][A-Za-z0-9_.-]*\.[a-zA-Z0-9]+)(?:[\s\)\].,;:!?'"\`]|$)"#,
    )
    .expect("Invalid bare filename regex")
});

/// Compiled regex for URL extraction.
/// Matches HTTP/HTTPS URLs with optional paths and query strings.
static URL_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"https?://[\w.-]+(?:/[\w./?%&=-]*)?").expect("Invalid URL regex"));

/// Compiled regex for bare domain names without a URL scheme (e.g. `Gestura.ai`, `example.com`).
/// Only matches well-known TLDs to avoid false-positives with file extensions like `.txt` or `.rs`.
/// Uses a `(label\.)+TLD` structure so the TLD alternation is always anchored to the final
/// dot-separated component — preventing greedy intermediate groups from consuming the TLD.
/// Runs before `BARE_FILENAME_REGEX` so domains are classified as URLs, not file paths.
static BARE_DOMAIN_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?:^|[\s\(\[,;'`])((?:[A-Za-z0-9][A-Za-z0-9-]*\.)+(?:com|org|net|io|ai|dev|co|app|tech|edu|gov|info|biz|online|site|web|so|me|tv|us|uk|ca|de|fr|au|jp|cn))(?:[/\s,;:!?\)\]'`]|$)"
    )
    .expect("Invalid bare domain regex")
});

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
        // Canonical built-in tool names. These are the names exposed by the
        // registry and understood by prompt/tool-schema construction.
        tool_categories.insert("file".to_string(), ContextCategory::FileSystem);
        tool_categories.insert("shell".to_string(), ContextCategory::Shell);
        tool_categories.insert("git".to_string(), ContextCategory::Git);
        tool_categories.insert("code".to_string(), ContextCategory::Code);
        tool_categories.insert("web".to_string(), ContextCategory::Web);
        tool_categories.insert("web_search".to_string(), ContextCategory::Web);
        tool_categories.insert("permissions".to_string(), ContextCategory::Tools);
        tool_categories.insert("a2a".to_string(), ContextCategory::A2a);
        tool_categories.insert("mcp".to_string(), ContextCategory::Mcp);
        tool_categories.insert("screenshot".to_string(), ContextCategory::Screen);
        tool_categories.insert("screen_record".to_string(), ContextCategory::Screen);
        tool_categories.insert("task".to_string(), ContextCategory::Task);

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
                // A fully-qualified URL is a strong, unambiguous web signal — boost confidence
                // so the pipeline's tool-selection logic doesn't fall back to all-tools even
                // when no Web keywords appeared in the request text.
                analysis.confidence += 0.4;
                extracted_ranges.push((start, end));
            }
        }

        // Detect bare domain names (e.g. `Gestura.ai`, `example.com`) BEFORE the file-path
        // passes so domains are classified as URLs/Web entities and not as file paths.
        // FILE_PATH_REGEX would otherwise capture `Gestura.ai` as a bare `word.ext` filename.
        for cap in BARE_DOMAIN_REGEX.captures_iter(request) {
            if let Some(m) = cap.get(1) {
                let start = m.start();
                let end = m.end();
                if !Self::overlaps_any(&extracted_ranges, start, end) {
                    analysis.entities.push(ExtractedEntity {
                        entity_type: EntityType::Url,
                        value: m.as_str().to_string(),
                        start,
                        end,
                    });
                    analysis.categories.insert(ContextCategory::Web);
                    // A bare domain is a clear web intent signal — boost confidence so that
                    // requests like "locate llm.txt for Gestura.ai" (zero keyword hits) still
                    // clear the 0.2 threshold and route to web tools rather than all-tools.
                    analysis.confidence += 0.3;
                    extracted_ranges.push((start, end));
                }
            }
        }

        // Extract file paths using regex
        for cap in FILE_PATH_REGEX.captures_iter(request) {
            if let Some(m) = cap.get(1) {
                let start = m.start();
                let end = m.end();
                let value = m.as_str();

                // Skip if already extracted (e.g., as part of a URL or domain)
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

        // G2: Detect well-known project root files by name alone (case-insensitive).
        // This ensures files like AGENTS.md, Cargo.toml, README.md are captured even
        // when the user doesn't include an explicit path prefix like `./`.
        let lower = request.to_lowercase();
        for &well_known in WELL_KNOWN_FILES {
            let needle = well_known.to_lowercase();
            if lower.contains(&needle) {
                // Only add if not already captured by FILE_PATH_REGEX.
                let already_extracted = analysis
                    .entities
                    .iter()
                    .any(|e| e.value.to_lowercase().ends_with(&needle));
                if !already_extracted {
                    // Find the byte offset in the original (case-preserved) text.
                    let start = lower.find(&needle).unwrap_or(0);
                    let end = start + well_known.len();
                    analysis.entities.push(ExtractedEntity {
                        entity_type: EntityType::FilePath,
                        value: well_known.to_string(),
                        start,
                        end,
                    });
                    analysis.categories.insert(ContextCategory::FileSystem);
                    extracted_ranges.push((start, end));
                }
            }
        }

        // G2: Detect any other bare filenames (e.g. `config.yaml`, `schema.json`)
        // that FILE_PATH_REGEX missed because they lack a path separator.
        for cap in BARE_FILENAME_REGEX.captures_iter(request) {
            if let Some(m) = cap.get(1) {
                let start = m.start();
                let end = m.end();
                let value = m.as_str();

                if Self::overlaps_any(&extracted_ranges, start, end) {
                    continue;
                }

                // Skip single-component names without extensions that look like
                // plain words (e.g. "it", "Go", "Rust"), keeping only names that
                // have an extension after the first dot.
                if !value.contains('.') {
                    continue;
                }

                analysis.entities.push(ExtractedEntity {
                    entity_type: EntityType::FilePath,
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
        assert!(analysis.suggested_tools.contains(&"file".to_string()));
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

    #[test]
    fn test_web_lookup_detection() {
        let analyzer = RequestAnalyzer::new();
        let analysis = analyzer.analyze(
            "please lookup the langchain landing page and tell me the main links it talks about",
        );

        assert!(analysis.categories.contains(&ContextCategory::Web));
        assert!(analysis.needs_tools);
    }

    #[test]
    fn test_web_browse_detection() {
        let analyzer = RequestAnalyzer::new();
        let analysis = analyzer.analyze("browse to the documentation website");

        assert!(analysis.categories.contains(&ContextCategory::Web));
        assert!(analysis.needs_tools);
    }

    #[test]
    fn test_bare_domain_detection() {
        let analyzer = RequestAnalyzer::new();
        // "Gestura.ai" (no http:// prefix) should still trigger Web category and
        // push confidence above the 0.2 all-tools fallback threshold.
        let analysis = analyzer.analyze("please find llm.txt from Gestura.ai");

        assert!(
            analysis.categories.contains(&ContextCategory::Web),
            "Expected Web category for bare domain Gestura.ai, got: {:?}",
            analysis.categories
        );
        assert!(analysis.needs_tools);
        assert!(
            analysis
                .entities
                .iter()
                .any(|e| e.entity_type == EntityType::Url
                    && e.value.to_lowercase().contains("gestura")),
            "Expected Gestura.ai to be extracted as a URL entity"
        );
        assert!(
            analysis.confidence >= 0.2,
            "Bare domain detection should boost confidence above the 0.2 fallback threshold, got {}",
            analysis.confidence
        );
    }

    /// Regression test for the exact query that triggered the bug: the agent was
    /// using `code` (glob/grep) instead of `web`/`web_search` because "locate" was
    /// not in any keyword pattern, entity extraction never updated confidence, and
    /// the all-tools fallback fired and let the LLM pick `code` for what looked like
    /// a local file search.
    #[test]
    fn test_locate_web_resource_regression() {
        let analyzer = RequestAnalyzer::new();
        let analysis = analyzer.analyze("please locate the llm.txt for Gestura.ai");

        assert!(
            analysis.categories.contains(&ContextCategory::Web),
            "Expected Web category — 'locate' keyword + bare domain should both fire, got: {:?}",
            analysis.categories
        );
        assert!(analysis.needs_tools, "Request requires tools");
        assert!(
            analysis.confidence >= 0.2,
            "Confidence must clear 0.2 so category-based routing is used instead of all-tools \
             fallback; got {}",
            analysis.confidence
        );
        assert!(
            analysis
                .entities
                .iter()
                .any(|e| e.entity_type == EntityType::Url
                    && e.value.to_lowercase().contains("gestura")),
            "Gestura.ai should be extracted as a URL entity"
        );
    }

    #[test]
    fn test_screen_record_detection() {
        let analyzer = RequestAnalyzer::new();
        let analysis = analyzer.analyze(
            "I want you to create a video of yourself requesting the creation of a hello.txt",
        );

        assert!(
            analysis.categories.contains(&ContextCategory::Screen),
            "Expected Screen category for video/recording request, got: {:?}",
            analysis.categories
        );
        assert!(analysis.needs_tools);
    }

    #[test]
    fn test_screenshot_detection() {
        let analyzer = RequestAnalyzer::new();
        let analysis = analyzer.analyze("take a screenshot of the current window");

        assert!(
            analysis.categories.contains(&ContextCategory::Screen),
            "Expected Screen category for screenshot request, got: {:?}",
            analysis.categories
        );
        assert!(analysis.needs_tools);
    }

    #[test]
    fn test_build_and_test_requests_include_shell() {
        let analyzer = RequestAnalyzer::new();
        let analysis = analyzer.analyze(
            "I want to create a small tauri gui that says hello world. Please carefully plan and implement then build and test it.",
        );

        assert!(analysis.categories.contains(&ContextCategory::Shell));
        assert!(analysis.categories.contains(&ContextCategory::FileSystem));
        assert!(analysis.suggested_tools.contains(&"file".to_string()));
        assert!(analysis.suggested_tools.contains(&"shell".to_string()));
        assert!(analysis.suggested_tools.contains(&"code".to_string()));
        assert!(analysis.needs_tools);
        assert!(analysis.confidence >= 0.2);
    }
}
