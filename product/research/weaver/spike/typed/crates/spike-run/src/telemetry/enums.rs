//! The enum attributes this crate's signals carry.

/// The class of provider error: `retryable`, `context_exhausted`, `auth`, `fatal`, or `malformed`; or `cancelled` when the run was cancelled while the attempt was in flight.
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

/// Always `chat`.
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

/// What a provider call is for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LabletChatPurpose {
    /// The call asks the model for the next turn of the conversation.
    Turn,
}

impl LabletChatPurpose {
    /// The value on the wire.
    pub fn as_str(&self) -> &str {
        match self {
            Self::Turn => "turn",
        }
    }
}

/// Where a tool comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LabletToolSource {
    /// One of lablet's built-in tools.
    Builtin,
    /// A tool served by a configured MCP server.
    Mcp,
}

impl LabletToolSource {
    /// The value on the wire.
    pub fn as_str(&self) -> &str {
        match self {
            Self::Builtin => "builtin",
            Self::Mcp => "mcp",
        }
    }
}

/// How a tool call ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LabletToolStatus {
    /// The tool ran and returned a result.
    Ok,
    /// The tool ran and reported an error in its own result.
    ToolError,
    /// No configured tool has the name the model called.
    Unknown,
    /// The model's arguments for the call weren't valid JSON, so nothing ran.
    MalformedInput,
    /// The loop declined a `task_complete` call that wasn't the response's only call, so nothing ran.
    Rejected,
    /// The call's turn came when the run had no time left, or once the run had been cancelled, so nothing was started for it. No span carries it.
    NotRun,
    /// The call ran past its deadline, the shorter of its executor's own limit and the time the run had left.
    Timeout,
    /// The executor failed before the tool could answer.
    Failed,
    /// The run was cancelled while the call ran, so the call was stopped where it was.
    Cancelled,
}

impl LabletToolStatus {
    /// The value on the wire.
    pub fn as_str(&self) -> &str {
        match self {
            Self::Ok => "ok",
            Self::ToolError => "tool_error",
            Self::Unknown => "unknown",
            Self::MalformedInput => "malformed_input",
            Self::Rejected => "rejected",
            Self::NotRun => "not_run",
            Self::Timeout => "timeout",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }
}

/// The name of the request or notification method.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum McpMethodName {
    /// Notification cancelling a previously-issued request.
    NotificationsCancelled,
    /// Request to initialize the MCP client.
    Initialize,
    /// Notification indicating that the MCP client has been initialized.
    NotificationsInitialized,
    /// Notification indicating the progress for a long-running operation.
    NotificationsProgress,
    /// Request to check that the other party is still alive.
    Ping,
    /// Request to list resources available on server.
    ResourcesList,
    /// Request to list resource templates available on server.
    ResourcesTemplatesList,
    /// Request to read a resource.
    ResourcesRead,
    /// Notification indicating that the list of resources has changed.
    NotificationsResourcesListChanged,
    /// Request to subscribe to a resource.
    ResourcesSubscribe,
    /// Request to unsubscribe from resource updates.
    ResourcesUnsubscribe,
    /// Notification indicating that a resource has been updated.
    NotificationsResourcesUpdated,
    /// Request to list prompts available on server.
    PromptsList,
    /// Request to get a prompt.
    PromptsGet,
    /// Notification indicating that the list of prompts has changed.
    NotificationsPromptsListChanged,
    /// Request to list tools available on server.
    ToolsList,
    /// Request to call a tool.
    ToolsCall,
    /// Notification indicating that the list of tools has changed.
    NotificationsToolsListChanged,
    /// Request to set the logging level.
    LoggingSetLevel,
    /// Notification indicating that a message has been received.
    NotificationsMessage,
    /// Request to create a sampling message.
    SamplingCreateMessage,
    /// Request to complete a prompt.
    CompletionComplete,
    /// Request to list roots available on server.
    RootsList,
    /// Notification indicating that the list of roots has changed.
    NotificationsRootsListChanged,
    /// Request from the server to elicit additional information from the user via the client
    ElicitationCreate,
    /// A value the registry doesn't list; the attribute's enum is open.
    Other(String),
}

impl McpMethodName {
    /// The value on the wire.
    pub fn as_str(&self) -> &str {
        match self {
            Self::NotificationsCancelled => "notifications/cancelled",
            Self::Initialize => "initialize",
            Self::NotificationsInitialized => "notifications/initialized",
            Self::NotificationsProgress => "notifications/progress",
            Self::Ping => "ping",
            Self::ResourcesList => "resources/list",
            Self::ResourcesTemplatesList => "resources/templates/list",
            Self::ResourcesRead => "resources/read",
            Self::NotificationsResourcesListChanged => "notifications/resources/list_changed",
            Self::ResourcesSubscribe => "resources/subscribe",
            Self::ResourcesUnsubscribe => "resources/unsubscribe",
            Self::NotificationsResourcesUpdated => "notifications/resources/updated",
            Self::PromptsList => "prompts/list",
            Self::PromptsGet => "prompts/get",
            Self::NotificationsPromptsListChanged => "notifications/prompts/list_changed",
            Self::ToolsList => "tools/list",
            Self::ToolsCall => "tools/call",
            Self::NotificationsToolsListChanged => "notifications/tools/list_changed",
            Self::LoggingSetLevel => "logging/setLevel",
            Self::NotificationsMessage => "notifications/message",
            Self::SamplingCreateMessage => "sampling/createMessage",
            Self::CompletionComplete => "completion/complete",
            Self::RootsList => "roots/list",
            Self::NotificationsRootsListChanged => "notifications/roots/list_changed",
            Self::ElicitationCreate => "elicitation/create",
            Self::Other(value) => value,
        }
    }
}

/// `pipe` for a stdio MCP server, `tcp` for an HTTP one.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum NetworkTransport {
    /// TCP
    Tcp,
    /// UDP
    Udp,
    /// Named or anonymous pipe.
    Pipe,
    /// UNIX domain socket
    Unix,
    /// QUIC
    Quic,
    /// A value the registry doesn't list; the attribute's enum is open.
    Other(String),
}

impl NetworkTransport {
    /// The value on the wire.
    pub fn as_str(&self) -> &str {
        match self {
            Self::Tcp => "tcp",
            Self::Udp => "udp",
            Self::Pipe => "pipe",
            Self::Unix => "unix",
            Self::Quic => "quic",
            Self::Other(value) => value,
        }
    }
}
