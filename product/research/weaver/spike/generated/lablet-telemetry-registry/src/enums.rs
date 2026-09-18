//! Closed value sets: one Rust enum per enum-typed attribute in the contract.
//!
//! DO NOT EDIT. Generated from the registry by `weaver registry generate`.
//! Semantic-convention enums are open by definition, so every enum carries an
//! `Unlisted(String)` escape hatch and `as_str()` returns the wire value.

/// Values of `error.type`.
/// Describes a class of error the operation ended with
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ErrorType {
    /// A fallback error value to be used when the instrumentation doesn't define a custom value
    Other,
    /// A value not (yet) in the registry (semantic-convention enums are open).
    Unlisted(String),
}

impl ErrorType {
    /// The attribute value as it appears on the wire.
    pub fn as_str(&self) -> &str {
        match self {
            Self::Other => "_OTHER",
            Self::Unlisted(v) => v.as_str(),
        }
    }
}

impl core::fmt::Display for ErrorType {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Values of `gen_ai.operation.name`.
/// The name of the operation being performed
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum GenAiOperationName {
    /// Chat completion operation such as [OpenAI Chat API](https://platform.openai.com/docs/api-reference/chat)
    Chat,
    /// Multimodal content generation operation such as [Gemini Generate Content](https://ai.google.dev/api/generate-content)
    GenerateContent,
    /// Text completions operation such as [OpenAI Completions API (Legacy)](https://platform.openai.com/docs/api-reference/completions)
    TextCompletion,
    /// Embeddings operation such as [OpenAI Create embeddings API](https://platform.openai.com/docs/api-reference/embeddings/create)
    Embeddings,
    /// Retrieval operation such as [OpenAI Search Vector Store API](https://platform.openai.com/docs/api-reference/vector-stores/search)
    Retrieval,
    /// Fetch a previously generated model response by its identifier, without performing inference, such as [OpenAI Get a model response](https://platform.openai.com/docs/api-reference/responses/get)
    FetchResponse,
    /// Create GenAI agent
    CreateAgent,
    /// Invoke GenAI agent
    InvokeAgent,
    /// Execute a tool
    ExecuteTool,
    /// Invoke GenAI workflow
    InvokeWorkflow,
    /// Agent planning or task decomposition phase
    Plan,
    /// Search/query memories from a memory store
    SearchMemory,
    /// Create new memory records
    CreateMemory,
    /// Update existing memory records
    UpdateMemory,
    /// Create or update memory records without the caller choosing which
    UpsertMemory,
    /// Delete memory records
    DeleteMemory,
    /// Create or initialize a memory store
    CreateMemoryStore,
    /// Delete or deprovision a memory store
    DeleteMemoryStore,
    /// A value not (yet) in the registry (semantic-convention enums are open).
    Unlisted(String),
}

impl GenAiOperationName {
    /// The attribute value as it appears on the wire.
    pub fn as_str(&self) -> &str {
        match self {
            Self::Chat => "chat",
            Self::GenerateContent => "generate_content",
            Self::TextCompletion => "text_completion",
            Self::Embeddings => "embeddings",
            Self::Retrieval => "retrieval",
            Self::FetchResponse => "fetch_response",
            Self::CreateAgent => "create_agent",
            Self::InvokeAgent => "invoke_agent",
            Self::ExecuteTool => "execute_tool",
            Self::InvokeWorkflow => "invoke_workflow",
            Self::Plan => "plan",
            Self::SearchMemory => "search_memory",
            Self::CreateMemory => "create_memory",
            Self::UpdateMemory => "update_memory",
            Self::UpsertMemory => "upsert_memory",
            Self::DeleteMemory => "delete_memory",
            Self::CreateMemoryStore => "create_memory_store",
            Self::DeleteMemoryStore => "delete_memory_store",
            Self::Unlisted(v) => v.as_str(),
        }
    }
}

impl core::fmt::Display for GenAiOperationName {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Values of `lablet.run.stop_reason`.
/// Why the run ended
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum LabletRunStopReason {
    /// The model called `task_complete` or produced a final answer
    Completed,
    /// The run hit `run.max_turns`
    MaxTurns,
    /// The run hit `run.timeout_ms`
    Timeout,
    /// A fatal provider or tool error ended the run
    Error,
    /// A value not (yet) in the registry (semantic-convention enums are open).
    Unlisted(String),
}

impl LabletRunStopReason {
    /// The attribute value as it appears on the wire.
    pub fn as_str(&self) -> &str {
        match self {
            Self::Completed => "completed",
            Self::MaxTurns => "max_turns",
            Self::Timeout => "timeout",
            Self::Error => "error",
            Self::Unlisted(v) => v.as_str(),
        }
    }
}

impl core::fmt::Display for LabletRunStopReason {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.as_str())
    }
}

