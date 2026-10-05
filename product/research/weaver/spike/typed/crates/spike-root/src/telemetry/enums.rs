//! The enum attributes this crate's signals carry.

/// The stop reason, when the run didn't complete.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ErrorType {
    /// A value the registry doesn't list; the attribute's enum is open.
    Other(String),
}

impl ErrorType {
    /// The value on the wire.
    pub fn as_str(&self) -> &str {
        match self {
            Self::Other(value) => value,
        }
    }
}

/// Always `invoke_agent`.
#[derive(Debug, Clone, PartialEq, Eq)]
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
    /// A value the registry doesn't list; the attribute's enum is open.
    Other(String),
}

impl GenAiOperationName {
    /// The value on the wire.
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
            Self::Other(value) => value,
        }
    }
}

/// The Generative AI provider as identified by the client or server instrumentation.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum GenAiProviderName {
    /// [OpenAI](https://openai.com/)
    Openai,
    /// Any Google generative AI endpoint
    GcpGenAi,
    /// [Vertex AI](https://cloud.google.com/vertex-ai)
    GcpVertexAi,
    /// [Gemini](https://cloud.google.com/products/gemini)
    GcpGemini,
    /// [Anthropic](https://www.anthropic.com/)
    Anthropic,
    /// [Cohere](https://cohere.com/)
    Cohere,
    /// Azure AI Inference
    AzureAiInference,
    /// [Azure OpenAI](https://learn.microsoft.com/en-us/azure/ai-services/openai/overview)
    AzureAiOpenai,
    /// [IBM Watsonx AI](https://www.ibm.com/products/watsonx-ai)
    IbmWatsonxAi,
    /// [AWS Bedrock](https://aws.amazon.com/bedrock)
    AwsBedrock,
    /// [Perplexity](https://www.perplexity.ai/)
    Perplexity,
    /// [xAI](https://x.ai/)
    XAi,
    /// [DeepSeek](https://www.deepseek.com/)
    Deepseek,
    /// [Groq](https://groq.com/)
    Groq,
    /// [Mistral AI](https://mistral.ai/)
    MistralAi,
    /// [Moonshot AI](https://www.moonshot.ai/)
    MoonshotAi,
    /// A value the registry doesn't list; the attribute's enum is open.
    Other(String),
}

impl GenAiProviderName {
    /// The value on the wire.
    pub fn as_str(&self) -> &str {
        match self {
            Self::Openai => "openai",
            Self::GcpGenAi => "gcp.gen_ai",
            Self::GcpVertexAi => "gcp.vertex_ai",
            Self::GcpGemini => "gcp.gemini",
            Self::Anthropic => "anthropic",
            Self::Cohere => "cohere",
            Self::AzureAiInference => "azure.ai.inference",
            Self::AzureAiOpenai => "azure.ai.openai",
            Self::IbmWatsonxAi => "ibm.watsonx.ai",
            Self::AwsBedrock => "aws.bedrock",
            Self::Perplexity => "perplexity",
            Self::XAi => "x_ai",
            Self::Deepseek => "deepseek",
            Self::Groq => "groq",
            Self::MistralAi => "mistral_ai",
            Self::MoonshotAi => "moonshot_ai",
            Self::Other(value) => value,
        }
    }
}

/// How long the run's MCP servers live.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LabletMcpLifetime {
    /// The servers are started again for each run.
    Run,
    /// The servers are started once and serve every run of their `Lablet`.
    Lablet,
}

impl LabletMcpLifetime {
    /// The value on the wire.
    pub fn as_str(&self) -> &str {
        match self {
            Self::Run => "run",
            Self::Lablet => "lablet",
        }
    }
}

/// The API the run reached its model through.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LabletRequestApi {
    /// Anthropic's Messages API.
    Messages,
    /// OpenAI's Responses API.
    Responses,
    /// OpenAI's chat completions, and every server that speaks them.
    ChatCompletions,
    /// The scripted provider's script.
    Script,
}

impl LabletRequestApi {
    /// The value on the wire.
    pub fn as_str(&self) -> &str {
        match self {
            Self::Messages => "messages",
            Self::Responses => "responses",
            Self::ChatCompletions => "chat_completions",
            Self::Script => "script",
        }
    }
}

/// Which runs share what the provider caches of the run's requests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LabletRequestCacheScope {
    /// Every run that sends the same prefix shares what the provider caches of it.
    Shared,
    /// Each request carries the run id as its cache key, so no other run reads what this one cached.
    Run,
}

impl LabletRequestCacheScope {
    /// The value on the wire.
    pub fn as_str(&self) -> &str {
        match self {
            Self::Shared => "shared",
            Self::Run => "run",
        }
    }
}

/// How the run decides that the model has finished.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LabletRunCompletionMode {
    /// The run completes when the model returns a turn with no tool calls.
    Natural,
    /// The run completes when the model calls `task_complete`.
    Explicit,
}

impl LabletRunCompletionMode {
    /// The value on the wire.
    pub fn as_str(&self) -> &str {
        match self {
            Self::Natural => "natural",
            Self::Explicit => "explicit",
        }
    }
}

/// Why the run ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LabletRunStopReason {
    /// The model finished, by a turn with no tool calls or by calling `task_complete`.
    Completed,
    /// In explicit mode, the model returned a turn with no tool calls and never called `task_complete`.
    EndedWithoutCompletion,
    /// The run reached `run.max_turns`.
    MaxTurns,
    /// The run reached `run.timeout`.
    Timeout,
    /// Input plus output tokens reached `run.max_total_tokens`.
    MaxTotalTokens,
    /// The last response hit the output token limit. Its tool calls, if any, weren't executed.
    OutputTruncated,
    /// The conversation outgrew the model's context. The provider rejected the request as too long, or cut the response short at the window.
    ContextExhausted,
    /// One provider call failed on every attempt `run.max_retries` allows.
    RetriesExhausted,
    /// `run.max_consecutive_invalid_turns` turns in a row made no call that reached a tool.
    InvalidCallsExhausted,
    /// The run was cancelled.
    Cancelled,
    /// The provider returned an error that isn't retryable.
    ProviderError,
    /// The model declined to answer, or a content filter withheld the response.
    Refused,
}

impl LabletRunStopReason {
    /// The value on the wire.
    pub fn as_str(&self) -> &str {
        match self {
            Self::Completed => "completed",
            Self::EndedWithoutCompletion => "ended_without_completion",
            Self::MaxTurns => "max_turns",
            Self::Timeout => "timeout",
            Self::MaxTotalTokens => "max_total_tokens",
            Self::OutputTruncated => "output_truncated",
            Self::ContextExhausted => "context_exhausted",
            Self::RetriesExhausted => "retries_exhausted",
            Self::InvalidCallsExhausted => "invalid_calls_exhausted",
            Self::Cancelled => "cancelled",
            Self::ProviderError => "provider_error",
            Self::Refused => "refused",
        }
    }
}
