//! The allowlist of API-key operations. Every request resolves its route here first; a
//! method and path with no row is refused before any network call.

use std::fmt;

/// HTTP method of a registry row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Method {
    /// GET
    Get,
    /// POST
    Post,
    /// PUT
    Put,
    /// PATCH
    Patch,
    /// DELETE
    Delete,
}

impl Method {
    /// Parses a method name case-insensitively.
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_uppercase().as_str() {
            "GET" => Some(Self::Get),
            "POST" => Some(Self::Post),
            "PUT" => Some(Self::Put),
            "PATCH" => Some(Self::Patch),
            "DELETE" => Some(Self::Delete),
            _ => None,
        }
    }

    /// The method name in upper case.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Post => "POST",
            Self::Put => "PUT",
            Self::Patch => "PATCH",
            Self::Delete => "DELETE",
        }
    }

    /// Only GET is retried: the API offers no idempotency keys for writes.
    pub fn is_idempotent(self) -> bool {
        self == Self::Get
    }
}

impl fmt::Display for Method {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<Method> for reqwest::Method {
    fn from(m: Method) -> Self {
        match m {
            Method::Get => reqwest::Method::GET,
            Method::Post => reqwest::Method::POST,
            Method::Put => reqwest::Method::PUT,
            Method::Patch => reqwest::Method::PATCH,
            Method::Delete => reqwest::Method::DELETE,
        }
    }
}

/// What calling a route does to the world. Decides whether a confirmation is needed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Effect {
    /// Reads only.
    Read,
    /// Changes client configuration that real customers do not see until something is published.
    Write,
    /// Changes what real customers experience now, or dials real people.
    Live,
    /// Deletes or ends something for good.
    Destructive,
}

impl Effect {
    /// Lower-case name for prompts and dry-run output.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Write => "write",
            Self::Live => "live",
            Self::Destructive => "destructive",
        }
    }
}

/// One allowed operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Route {
    /// HTTP method.
    pub method: Method,
    /// Path template; `{name}` matches one path segment.
    pub path: &'static str,
    /// API key scope the server checks.
    pub scope: &'static str,
    /// What the call does.
    pub effect: Effect,
}

const fn r(method: Method, path: &'static str, scope: &'static str, effect: Effect) -> Route {
    Route {
        method,
        path,
        scope,
        effect,
    }
}

use Effect::{Destructive, Live, Read, Write};
use Method::{Delete, Get, Patch, Post, Put};

const PROD_R: &str = "client_products:read";
const PROD_W: &str = "client_products:write";
const AGENT_R: &str = "ai_agents:read";
const AGENT_W: &str = "ai_agents:write";
const SESS_R: &str = "chat_sessions:read";
const ANALYTICS_R: &str = "analytics:read";
const OPT_R: &str = "client_outbound_options:read";
const OPT_W: &str = "client_outbound_options:write";
const BATCH_R: &str = "client_outbound_batches:read";
const BATCH_W: &str = "client_outbound_batches:write";
const CONV_W: &str = "text_chat_conversations:write";
const CRM_R: &str = "crm_client_users:read";
const CRM_W: &str = "crm_client_users:write";

/// Every operation a client API key may call through this CLI. `/get-call-token` and
/// JWT-only routes are deliberately absent.
pub static ROUTES: &[Route] = &[
    // AI teams (products)
    r(Get, "/client/ai-agent-teams", PROD_R, Read),
    r(Get, "/client/products", PROD_R, Read),
    r(Get, "/client/products/{id}", PROD_R, Read),
    r(Post, "/client/products", PROD_W, Write),
    r(Put, "/client/products/{id}", PROD_W, Write),
    r(Put, "/client/products/{id}/visibility", PROD_W, Write),
    r(Delete, "/client/products/{id}", PROD_W, Destructive),
    r(Get, "/client/disposition-outcomes", PROD_R, Read),
    r(
        Get,
        "/client/ai-agent-teams/{id}/customer-context-variables",
        PROD_R,
        Read,
    ),
    r(Post, "/client/products/{id}/postprocessors", PROD_W, Write),
    r(
        Put,
        "/client/products/{id}/postprocessors/{postprocessorId}",
        PROD_W,
        Write,
    ),
    r(
        Delete,
        "/client/products/{id}/postprocessors/{postprocessorId}",
        PROD_W,
        Destructive,
    ),
    r(Get, "/client/products/{id}/automations", PROD_R, Read),
    r(
        Get,
        "/client/products/{id}/automations/catalog",
        PROD_R,
        Read,
    ),
    r(Get, "/client/automations/trigger-variables", PROD_R, Read),
    r(Post, "/client/products/{id}/automations", PROD_W, Write),
    r(
        Put,
        "/client/products/{id}/automations/{automationId}",
        PROD_W,
        Write,
    ),
    r(
        Delete,
        "/client/products/{id}/automations/{automationId}",
        PROD_W,
        Destructive,
    ),
    // Agents
    r(Get, "/client/ai-agents", AGENT_R, Read),
    r(Get, "/client/agents/{slug}", AGENT_R, Read),
    r(Get, "/client/agent-templates", AGENT_R, Read),
    r(Post, "/client/agents", AGENT_W, Write),
    r(Delete, "/client/agents/{slug}", AGENT_W, Destructive),
    r(
        Put,
        "/client/agents/{agentSlug}/description",
        AGENT_W,
        Write,
    ),
    r(Put, "/client/agents/{agentSlug}/profile", AGENT_W, Write),
    r(Put, "/client/agents/{agentSlug}/visibility", AGENT_W, Write),
    // Agent revisions
    r(Get, "/client/agents/{slug}/revisions", AGENT_R, Read),
    r(
        Get,
        "/client/agents/{slug}/revisions/{revisionId}",
        AGENT_R,
        Read,
    ),
    r(Post, "/client/agents/{slug}/revisions", AGENT_W, Write),
    r(Post, "/client/agents/{slug}/publish", AGENT_W, Live),
    r(
        Post,
        "/client/agents/{slug}/revisions/{revisionId}/publish",
        AGENT_W,
        Live,
    ),
    r(
        Delete,
        "/client/agents/{slug}/revisions/{revisionId}",
        AGENT_W,
        Destructive,
    ),
    // Agent test sessions
    r(
        Get,
        "/client/agents/{slug}/chat-session-tests",
        SESS_R,
        Read,
    ),
    r(
        Get,
        "/client/agents/{slug}/chat-session-tests/{uuid}",
        SESS_R,
        Read,
    ),
    // Tools
    r(Get, "/client/plugin-function-integrations", AGENT_R, Read),
    r(Get, "/client/plugin-functions", AGENT_R, Read),
    r(Get, "/client/plugin-functions/{id}", AGENT_R, Read),
    r(Post, "/client/plugin-functions", AGENT_W, Write),
    r(Put, "/client/plugin-functions/{id}", AGENT_W, Live),
    r(
        Post,
        "/client/plugin-functions/{id}/duplicate",
        AGENT_W,
        Write,
    ),
    r(
        Post,
        "/client/plugin-functions/{id}/test-run",
        AGENT_W,
        Live,
    ),
    r(
        Delete,
        "/client/plugin-functions/{id}",
        AGENT_W,
        Destructive,
    ),
    r(Get, "/client/phone-tools", AGENT_R, Read),
    r(Get, "/client/text-chat-tools", AGENT_R, Read),
    r(Get, "/client/voices", AGENT_R, Read),
    // Chat sessions and calls
    r(Get, "/client/chat-sessions-list", SESS_R, Read),
    r(Get, "/client/chat-sessions-list/csv", SESS_R, Read),
    r(Get, "/client/chat-sessions", SESS_R, Read),
    r(Get, "/client/chat-sessions/{uuid}", SESS_R, Read),
    r(Get, "/client/chat-sessions/{uuid}/record", SESS_R, Read),
    r(
        Get,
        "/client/chat-sessions/{uuid}/record/download",
        SESS_R,
        Read,
    ),
    r(
        Get,
        "/client/chat-sessions/{uuid}/record/checksum",
        SESS_R,
        Read,
    ),
    r(Get, "/client/text-chats/chat-sessions-list", SESS_R, Read),
    r(
        Get,
        "/client/text-chats/chat-sessions-list/csv",
        SESS_R,
        Read,
    ),
    // Text chat inbox
    r(Get, "/client/text-chats/conversations", SESS_R, Read),
    r(Get, "/client/text-chats/conversations/{uuid}", SESS_R, Read),
    r(Get, "/client/text-chats/tags", SESS_R, Read),
    r(Put, "/client/chat-conversations/{uuid}", CONV_W, Live),
    r(
        Put,
        "/client/text-chats/{provider_type}/channels/{channel_identifier}/conversations/{provider_conversation_id}",
        CONV_W,
        Live,
    ),
    // Text channel configs
    r(Get, "/client/text-channel-config-providers", PROD_R, Read),
    r(Get, "/client/text-channel-configs", PROD_R, Read),
    r(Get, "/client/text-channel-configs/{id}", PROD_R, Read),
    // Outbound templates and batches
    r(Get, "/client/outbound/options", OPT_R, Read),
    r(Get, "/client/outbound/options/{id}", OPT_R, Read),
    r(Post, "/client/outbound/options", OPT_W, Write),
    r(Put, "/client/outbound/options/{id}", OPT_W, Write),
    r(Delete, "/client/outbound/options/{id}", OPT_W, Destructive),
    r(Post, "/client/outbound/batches", BATCH_W, Live),
    r(Get, "/client/outbound/batches/{id}", BATCH_R, Read),
    r(Get, "/client/outbound/batches/{id}/records", BATCH_R, Read),
    r(
        Get,
        "/client/outbound/batches/{id}/records/download",
        BATCH_R,
        Read,
    ),
    r(Get, "/client/outbound/call-data-records", BATCH_R, Read),
    r(Post, "/client/outbound/batches/{id}/pause", BATCH_W, Live),
    r(Post, "/client/outbound/batches/{id}/resume", BATCH_W, Live),
    r(
        Post,
        "/client/outbound/batches/{id}/cancel",
        BATCH_W,
        Destructive,
    ),
    // Analytics
    r(Get, "/client/analytics/summary", ANALYTICS_R, Read),
    r(Get, "/client/analytics/short-calls", ANALYTICS_R, Read),
    r(Get, "/client/analytics/hourly-charts", ANALYTICS_R, Read),
    r(
        Get,
        "/client/analytics/duration-histogram",
        ANALYTICS_R,
        Read,
    ),
    r(Get, "/client/analytics/heatmap", ANALYTICS_R, Read),
    r(Get, "/client/analytics/speech-ratio", ANALYTICS_R, Read),
    r(Get, "/client/analytics/report/download", ANALYTICS_R, Read),
    r(Get, "/client/text-analytics/summary", ANALYTICS_R, Read),
    r(
        Get,
        "/client/text-analytics/messages-hourly",
        ANALYTICS_R,
        Read,
    ),
    r(Get, "/client/text-analytics/heatmap", ANALYTICS_R, Read),
    // CRM and inbound admission
    r(Get, "/crm/users", CRM_R, Read),
    r(Post, "/crm/user", CRM_W, Write),
    r(Patch, "/crm/user", CRM_W, Write),
    r(
        Get,
        "/client/calls/inbound-admission",
        "inbound_admission:read",
        Read,
    ),
];

impl Route {
    fn literal_segments(&self) -> usize {
        self.path
            .split('/')
            .filter(|s| !s.is_empty() && !is_param(s))
            .count()
    }

    fn matches(&self, path: &str) -> bool {
        let mut want = self.path.split('/').filter(|s| !s.is_empty());
        let mut got = path.split('/').filter(|s| !s.is_empty());
        loop {
            match (want.next(), got.next()) {
                (None, None) => return true,
                (Some(w), Some(g)) if is_param(w) || w == g => {}
                _ => return false,
            }
        }
    }
}

fn is_param(segment: &str) -> bool {
    segment.starts_with('{') && segment.ends_with('}')
}

/// Finds the route for a concrete path such as `/client/agents/mali/revisions`.
/// When templates overlap, the one with the most literal segments wins.
pub fn resolve(method: Method, path: &str) -> Option<&'static Route> {
    ROUTES
        .iter()
        .filter(|r| r.method == method && r.matches(path))
        .max_by_key(|r| r.literal_segments())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_matches_template_segments() {
        let route = resolve(Post, "/client/agents/mali/revisions/17/publish");
        assert_eq!(
            route.map(|r| r.path),
            Some("/client/agents/{slug}/revisions/{revisionId}/publish")
        );
    }

    #[test]
    fn resolve_prefers_literal_segment_over_parameter() {
        let route = resolve(Get, "/client/products/3/automations/catalog");
        assert_eq!(
            route.map(|r| r.path),
            Some("/client/products/{id}/automations/catalog")
        );
    }

    #[test]
    fn resolve_refuses_call_token_route() {
        assert_eq!(resolve(Post, "/get-call-token"), None);
    }

    #[test]
    fn resolve_refuses_jwt_only_knowledge_route() {
        assert_eq!(resolve(Get, "/client/documents"), None);
    }

    #[test]
    fn resolve_refuses_wrong_method() {
        assert_eq!(resolve(Delete, "/client/voices"), None);
    }

    #[test]
    fn registry_has_no_duplicate_rows() {
        for (i, a) in ROUTES.iter().enumerate() {
            for b in &ROUTES[i + 1..] {
                assert!(
                    !(a.method == b.method && a.path == b.path),
                    "duplicate {} {}",
                    a.method,
                    a.path
                );
            }
        }
    }
}
