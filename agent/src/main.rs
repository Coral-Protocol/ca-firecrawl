use clap::Parser;
use coral_rs::agent::Agent;
use coral_rs::agent_loop::AgentLoop;
use coral_rs::api::generated::types::{AgentClaimAmount, McpToolName};
use coral_rs::claim_manager::ClaimManager;
use coral_rs::completion_evaluated_prompt::CompletionEvaluatedPrompt;
use coral_rs::mcp_server::McpConnectionBuilder;
use coral_rs::repeating_prompt_stream::repeating_prompt_stream;
use coral_rs::rig::client::{CompletionClient, ProviderClient};
use coral_rs::rig::providers::openai::GPT_4_1;
use coral_rs::rig::providers::openrouter;
use coral_rs::rmcp::model::ProtocolVersion;
use coral_rs::telemetry::TelemetryMode;

#[derive(Parser, Debug)]
struct Config {
    #[clap(long, env = "SYSTEM_PROMPT_SUFFIX")]
    prompt_suffix: Option<String>,

    #[clap(long, env = "LOOP_PROMPT_SUFFIX")]
    loop_prompt_suffix: Option<String>,

    #[clap(long, env = "TEMPERATURE")]
    temperature: f64,

    #[clap(long, env = "MAX_TOKENS")]
    max_tokens: u64,

    #[clap(long)]
    #[clap(long, env = "ENABLE_TELEMETRY")]
    enable_telemetry: bool,

    #[clap(long)]
    #[clap(long, env = "LOOP_DELAY")]
    loop_delay: Option<humantime::Duration>,

    #[clap(long)]
    #[clap(long, env = "LOOP_MAX_REPS")]
    loop_max_reps: usize,
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    // FIRECRAWL_API_KEY must be set for the MCP server to work
    std::env::var("FIRECRAWL_API_KEY").expect("FIRECRAWL_API_KEY must be set");

    let model = GPT_4_1;
    let config = Config::parse();

    let coral = McpConnectionBuilder::from_coral_env()
        .connect()
        .await.expect("Failed to connect to the Coral server");

    let firecrawl = McpConnectionBuilder::stdio(
        "/app/run.sh",
        vec![],
        "firecrawl"
    )
        .protocol_version(ProtocolVersion::V_2024_11_05)
        .connect()
        .await.expect("Failed to connect to the firecrawl MCP server");

    let completion_agent = openrouter::Client::from_env()
        .agent(model)
        .temperature(config.temperature)
        .max_tokens(config.max_tokens)
        .build();

    let mut preamble = coral.prompt_with_resources();
    if let Some(prompt_suffix) = config.prompt_suffix {
        preamble = preamble.string(prompt_suffix);
    }

    let claim_manager = ClaimManager::new()
        .mil_input_token_cost(AgentClaimAmount::Usd(1.250))
        .mil_output_token_cost(AgentClaimAmount::Usd(10.000));

    let mut agent = Agent::new(completion_agent)
        .preamble(preamble)
        .claim_manager(claim_manager)
        .mcp_server(coral)
        .mcp_server(firecrawl);

    if config.enable_telemetry {
        agent = agent.telemetry(TelemetryMode::OpenAI, model);
    }

    let mut evaluating_prompt = CompletionEvaluatedPrompt::new()
        .string(format!("1. Repeatedly call {} tool until it returns messages", McpToolName::CoralWaitForMentions))
        .string("2. Respond to any questions amongst the returned messages that could benefit from your toolset");

    if let Some(loop_prompt_suffix) = config.loop_prompt_suffix {
        evaluating_prompt = evaluating_prompt.string(loop_prompt_suffix);
    }

    let prompt_stream = repeating_prompt_stream(
        evaluating_prompt,
        config.loop_delay.map(Into::into),
        config.loop_max_reps
    );

    AgentLoop::new(agent, prompt_stream)
        .iteration_tool_quota(Some(4096))
        .execute()
        .await
        .expect("Agent loop failed");
}
