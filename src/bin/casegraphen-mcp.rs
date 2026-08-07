//! Reference external stdio process for the CaseGraphen control-plane wire.

use casegraphen::{
    control_plane::{
        ControlPlaneRefusal, ControlPlaneRequest, ControlPlaneTool, DecisionDelegate,
        ResourceDelegate,
    },
    execution_topology::parse_execution_topology,
    graph_lint::lint_execution_topology,
    mcp_stdio::{serve_stdio, McpStdioServer},
};
use serde_json::Value;
use std::{env, io};

struct ReferenceExternalDelegate;

impl DecisionDelegate for ReferenceExternalDelegate {
    fn invoke(&mut self, request: &ControlPlaneRequest) -> Result<Value, ControlPlaneRefusal> {
        if request.tool == ControlPlaneTool::LintExecutionTopology {
            let topology_json = request
                .payload
                .get("topology_json")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    refusal(
                        "invalid_topology_payload",
                        "lint_execution_topology requires payload.topology_json",
                    )
                })?;
            let topology = parse_execution_topology(topology_json).map_err(|findings| {
                refusal(
                    "invalid_execution_topology",
                    &serde_json::to_string(&findings).expect("findings serialize"),
                )
            })?;
            return serde_json::to_value(lint_execution_topology(&topology))
                .map_err(|error| refusal("serialization_failure", &error.to_string()));
        }
        Err(refusal(
            "external_decision_owner_required",
            "the stdio boundary requires an external adapter to bind existing CaseGraphen operation owners",
        ))
    }
}

impl ResourceDelegate for ReferenceExternalDelegate {
    fn read_resource(&mut self, _uri: &str) -> Result<Value, ControlPlaneRefusal> {
        Err(refusal(
            "external_resource_owner_required",
            "the stdio boundary requires an external adapter to bind a CaseGraphen state projection",
        ))
    }
}

fn refusal(code: &str, detail: &str) -> ControlPlaneRefusal {
    ControlPlaneRefusal {
        code: code.to_owned(),
        detail: detail.to_owned(),
        supplied_base_revision_id: None,
        current_revision_id: None,
        suggested_next_operation: "configure_external_owner_and_retry".to_owned(),
    }
}

/// This reference adapter takes no configuration flags (ADR 0019: it is a
/// stateless child-process transport, not a daemon), so `--help` is the one
/// argument worth recognizing at all — printed and exited 0 rather than
/// silently falling through to `serve_stdio`, which would otherwise start
/// the stdio server without ever explaining what it does.
const USAGE: &str = "casegraphen-mcp reads newline-delimited JSON-RPC 2.0 requests on stdin and \
writes responses on stdout; it takes no arguments and exits at stdin EOF (ADR 0019).";

fn main() -> io::Result<()> {
    if env::args().skip(1).any(|argument| argument == "--help") {
        println!("{USAGE}");
        return Ok(());
    }
    let mut server = McpStdioServer::new(ReferenceExternalDelegate);
    serve_stdio(&mut server, io::stdin().lock(), io::stdout().lock())
}
