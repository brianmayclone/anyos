use alloc::string::String;
use alloc::vec::Vec;
use alloc::format;
use anyos_std::json::{Value, Number};

// ════════════════════════════════════════════════════════════════
//  AI Assistant — Vibe Coding Backend
//
//  Supports: OpenAI API (GPT-4, Codex) and Anthropic API (Claude)
//  Features: Chat, code generation, explain, refactor, fix
// ════════════════════════════════════════════════════════════════

/// Which AI provider to use.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum AiProvider {
    OpenAI,      // api.openai.com
    Anthropic,   // api.anthropic.com
}

impl AiProvider {
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::OpenAI => "OpenAI",
            Self::Anthropic => "Claude",
        }
    }

    pub fn default_model(&self) -> &'static str {
        match self {
            Self::OpenAI => "gpt-4o",
            Self::Anthropic => "claude-sonnet-4-20250514",
        }
    }

    pub fn api_url(&self) -> &'static str {
        match self {
            Self::OpenAI => "https://api.openai.com/v1/chat/completions",
            Self::Anthropic => "https://api.anthropic.com/v1/messages",
        }
    }
}

/// A single message in the conversation.
#[derive(Clone, Debug)]
pub struct ChatMessage {
    pub role: MessageRole,
    pub content: String,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum MessageRole {
    System,
    User,
    Assistant,
}

impl MessageRole {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::System => "system",
            Self::User => "user",
            Self::Assistant => "assistant",
        }
    }
}

/// AI action types for code operations.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum CodeAction {
    Explain,
    Refactor,
    Fix,
    Generate,
    Review,
    Document,
    Test,
    Optimize,
}

impl CodeAction {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Explain => "Explain Code",
            Self::Refactor => "Refactor Code",
            Self::Fix => "Fix Code",
            Self::Generate => "Generate Code",
            Self::Review => "Review Code",
            Self::Document => "Add Documentation",
            Self::Test => "Generate Tests",
            Self::Optimize => "Optimize Code",
        }
    }

    pub fn system_prompt(&self) -> &'static str {
        match self {
            Self::Explain => "You are a code explainer. Explain the given code clearly and concisely. Describe what it does, how it works, and any important details.",
            Self::Refactor => "You are a code refactoring expert. Refactor the given code to be cleaner, more readable, and more maintainable. Keep the same functionality. Return only the refactored code.",
            Self::Fix => "You are a debugging expert. Find and fix bugs in the given code. Explain what was wrong and return the corrected code.",
            Self::Generate => "You are a code generator. Generate clean, well-structured code based on the user's request. Use best practices and add brief comments.",
            Self::Review => "You are a code reviewer. Review the given code for bugs, security issues, performance problems, and style issues. Be specific and actionable.",
            Self::Document => "You are a documentation writer. Add clear, helpful documentation comments to the given code. Use the appropriate comment style for the language.",
            Self::Test => "You are a test writer. Generate comprehensive unit tests for the given code. Cover edge cases and common scenarios.",
            Self::Optimize => "You are a performance expert. Optimize the given code for better performance while maintaining readability. Explain the optimizations.",
        }
    }
}

// ════════════════════════════════════════════════════════════════
//  AI Configuration
// ════════════════════════════════════════════════════════════════

const AI_SETTINGS_PATH: &str = "/Users/settings/anycode-ai.json";

pub struct AiConfig {
    pub provider: AiProvider,
    pub api_key: String,
    pub model: String,
    pub max_tokens: u32,
    pub temperature: f64,
    /// Custom API endpoint (for self-hosted / proxy)
    pub custom_endpoint: String,
}

impl AiConfig {
    pub fn load() -> Self {
        let defaults = Self::defaults();
        let data = match anyos_std::fs::read_to_string(AI_SETTINGS_PATH) {
            Ok(s) => s,
            Err(_) => return defaults,
        };
        let val = match Value::parse(&data) {
            Ok(v) => v,
            Err(_) => return defaults,
        };

        let provider = match val["provider"].as_str() {
            Some("anthropic") | Some("claude") => AiProvider::Anthropic,
            _ => AiProvider::OpenAI,
        };

        Self {
            provider,
            api_key: val["api_key"].as_str().map(String::from).unwrap_or_default(),
            model: val["model"].as_str().map(String::from)
                .unwrap_or_else(|| String::from(provider.default_model())),
            max_tokens: val["max_tokens"].as_i64().unwrap_or(4096) as u32,
            temperature: match &val["temperature"] {
                Value::Number(Number::Float(f)) => *f,
                Value::Number(Number::Int(i)) => *i as f64,
                _ => 0.7,
            },
            custom_endpoint: val["endpoint"].as_str().map(String::from).unwrap_or_default(),
        }
    }

    pub fn defaults() -> Self {
        Self {
            provider: AiProvider::OpenAI,
            api_key: String::new(),
            model: String::from("gpt-4o"),
            max_tokens: 4096,
            temperature: 0.7,
            custom_endpoint: String::new(),
        }
    }

    pub fn save(&self) {
        let mut obj = Value::new_object();
        let provider_str = match self.provider {
            AiProvider::OpenAI => "openai",
            AiProvider::Anthropic => "anthropic",
        };
        obj.set("provider", Value::String(String::from(provider_str)));
        obj.set("api_key", Value::String(self.api_key.clone()));
        obj.set("model", Value::String(self.model.clone()));
        obj.set("max_tokens", Value::Number(Number::Int(self.max_tokens as i64)));
        obj.set("temperature", Value::Number(Number::Float(self.temperature)));
        if !self.custom_endpoint.is_empty() {
            obj.set("endpoint", Value::String(self.custom_endpoint.clone()));
        }
        let json = obj.to_json_string_pretty();
        let _ = anyos_std::fs::write_bytes(AI_SETTINGS_PATH, json.as_bytes());
    }

    pub fn is_configured(&self) -> bool {
        !self.api_key.is_empty()
    }

    pub fn api_url(&self) -> &str {
        if !self.custom_endpoint.is_empty() {
            &self.custom_endpoint
        } else {
            self.provider.api_url()
        }
    }
}

// ════════════════════════════════════════════════════════════════
//  AI Client — sends requests to the API
// ════════════════════════════════════════════════════════════════

pub struct AiClient {
    pub config: AiConfig,
    pub history: Vec<ChatMessage>,
    pub is_requesting: bool,
}

impl AiClient {
    pub fn new() -> Self {
        let config = AiConfig::load();
        Self {
            config,
            history: Vec::new(),
            is_requesting: false,
        }
    }

    /// Clear conversation history.
    pub fn clear_history(&mut self) {
        self.history.clear();
    }

    /// Send a chat message and get a response.
    /// This is blocking — call from a timer-polled background process.
    pub fn chat(&mut self, user_message: &str) -> Result<String, String> {
        if !self.config.is_configured() {
            return Err(String::from("API key not configured. Open Settings > AI to set your API key."));
        }

        // Add user message to history
        self.history.push(ChatMessage {
            role: MessageRole::User,
            content: String::from(user_message),
        });

        // Build and send request
        let result = match self.config.provider {
            AiProvider::OpenAI => self.send_openai_request(None),
            AiProvider::Anthropic => self.send_anthropic_request(None),
        };

        match result {
            Ok(response) => {
                self.history.push(ChatMessage {
                    role: MessageRole::Assistant,
                    content: response.clone(),
                });
                Ok(response)
            }
            Err(e) => Err(e),
        }
    }

    /// Send a code action request with context.
    pub fn code_action(&mut self, action: CodeAction, code: &str, language: &str) -> Result<String, String> {
        if !self.config.is_configured() {
            return Err(String::from("API key not configured."));
        }

        let user_msg = format!(
            "Language: {}\n\n```{}\n{}\n```",
            language, language, code
        );

        let system = action.system_prompt();

        // Temporary history for this action
        let saved_history = core::mem::take(&mut self.history);

        self.history.push(ChatMessage {
            role: MessageRole::User,
            content: user_msg,
        });

        let result = match self.config.provider {
            AiProvider::OpenAI => self.send_openai_request(Some(system)),
            AiProvider::Anthropic => self.send_anthropic_request(Some(system)),
        };

        // Restore history
        self.history = saved_history;

        result
    }

    // ── OpenAI API ─────────────────────────────────────────────

    fn send_openai_request(&self, system_override: Option<&str>) -> Result<String, String> {
        let body = self.build_openai_body(system_override);
        let headers = format!(
            "Authorization: Bearer {}\r\n",
            self.config.api_key
        );

        let response = libhttp_client::post_with_headers(
            self.config.api_url(),
            body.as_bytes(),
            "application/json",
            &headers,
        ).ok_or_else(|| {
            let err = libhttp_client::last_error();
            let status = libhttp_client::last_status();
            format!("HTTP error: status={}, error={}", status, err)
        })?;

        let status = libhttp_client::last_status();
        let resp_str = core::str::from_utf8(&response)
            .map_err(|_| String::from("Invalid UTF-8 in response"))?;

        if status != 200 {
            return Err(format!("API error ({}): {}", status, truncate(resp_str, 200)));
        }

        // Parse OpenAI response
        let val = Value::parse(resp_str)
            .map_err(|_| String::from("Failed to parse API response"))?;

        // Extract: choices[0].message.content
        if let Some(choices) = val["choices"].as_array() {
            if let Some(first) = choices.first() {
                if let Some(content) = first["message"]["content"].as_str() {
                    return Ok(String::from(content));
                }
            }
        }

        // Check for error message
        if let Some(err_msg) = val["error"]["message"].as_str() {
            return Err(format!("API: {}", err_msg));
        }

        Err(String::from("Unexpected API response format"))
    }

    fn build_openai_body(&self, system_override: Option<&str>) -> String {
        let mut messages = Value::new_array();

        // System message
        let system_content = system_override.unwrap_or(
            "You are an expert programming assistant integrated into anyOS Code IDE. \
             Help the user with coding tasks. Be concise and provide code when appropriate. \
             Use markdown code blocks with language tags for code."
        );
        let mut sys_msg = Value::new_object();
        sys_msg.set("role", Value::String(String::from("system")));
        sys_msg.set("content", Value::String(String::from(system_content)));
        messages.push(sys_msg);

        // Conversation history
        for msg in &self.history {
            let mut m = Value::new_object();
            m.set("role", Value::String(String::from(msg.role.as_str())));
            m.set("content", Value::String(msg.content.clone()));
            messages.push(m);
        }

        let mut body = Value::new_object();
        body.set("model", Value::String(self.config.model.clone()));
        body.set("messages", messages);
        body.set("max_tokens", Value::Number(Number::Int(self.config.max_tokens as i64)));
        body.set("temperature", Value::Number(Number::Float(self.config.temperature)));

        body.to_json_string()
    }

    // ── Anthropic API ──────────────────────────────────────────

    fn send_anthropic_request(&self, system_override: Option<&str>) -> Result<String, String> {
        let body = self.build_anthropic_body(system_override);
        let headers = format!(
            "x-api-key: {}\r\nanthropic-version: 2023-06-01\r\n",
            self.config.api_key
        );

        let response = libhttp_client::post_with_headers(
            self.config.api_url(),
            body.as_bytes(),
            "application/json",
            &headers,
        ).ok_or_else(|| {
            let err = libhttp_client::last_error();
            let status = libhttp_client::last_status();
            format!("HTTP error: status={}, error={}", status, err)
        })?;

        let status = libhttp_client::last_status();
        let resp_str = core::str::from_utf8(&response)
            .map_err(|_| String::from("Invalid UTF-8 in response"))?;

        if status != 200 {
            return Err(format!("API error ({}): {}", status, truncate(resp_str, 200)));
        }

        // Parse Anthropic response
        let val = Value::parse(resp_str)
            .map_err(|_| String::from("Failed to parse API response"))?;

        // Extract: content[0].text
        if let Some(content) = val["content"].as_array() {
            if let Some(first) = content.first() {
                if let Some(text) = first["text"].as_str() {
                    return Ok(String::from(text));
                }
            }
        }

        // Check for error
        if let Some(err_msg) = val["error"]["message"].as_str() {
            return Err(format!("API: {}", err_msg));
        }

        Err(String::from("Unexpected API response format"))
    }

    fn build_anthropic_body(&self, system_override: Option<&str>) -> String {
        let system_content = system_override.unwrap_or(
            "You are an expert programming assistant integrated into anyOS Code IDE. \
             Help the user with coding tasks. Be concise and provide code when appropriate. \
             Use markdown code blocks with language tags for code."
        );

        let mut messages = Value::new_array();
        for msg in &self.history {
            if msg.role == MessageRole::System {
                continue;
            }
            let mut m = Value::new_object();
            m.set("role", Value::String(String::from(msg.role.as_str())));
            m.set("content", Value::String(msg.content.clone()));
            messages.push(m);
        }

        let mut body = Value::new_object();
        body.set("model", Value::String(self.config.model.clone()));
        body.set("max_tokens", Value::Number(Number::Int(self.config.max_tokens as i64)));
        body.set("system", Value::String(String::from(system_content)));
        body.set("messages", messages);

        body.to_json_string()
    }
}

fn truncate(s: &str, max: usize) -> &str {
    if s.len() <= max { s } else { &s[..max] }
}
