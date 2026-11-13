//! An MCP (Model Context Protocol) server over the engine's session and commands.
//!
//! The server speaks JSON-RPC 2.0 on the standard input and output, one message per line
//! (the MCP stdio transport). Only protocol messages go to stdout; logs go to stderr. It has
//! no dependency on an MCP library.
//!
//! * **Protocol versions** `2025-06-18`, `2025-03-26` and `2024-11-05`: `initialize` echoes
//!   the client's version when it is one of them and answers the newest otherwise.
//!   `notifications/initialized`, `ping`, `tools/list` and `tools/call` are served; other
//!   methods fail with `-32601`, malformed JSON with `-32700`, bad parameters with `-32602`.
//!   Requests before `initialize` are served too (lenient).
//! * **Tools**: exactly four, because a tool per command would repeat the large schemas of
//!   the design document in every definition. `list_commands` and `describe_command` expose
//!   the registry; `execute` runs one command and `batch` several, on the server's single
//!   [`Session`](raytrace_engine::Session). A command that fails is a tool result with
//!   `isError: true`, so JSON-RPC errors mean protocol mistakes only. A command added to the
//!   engine registry is reachable without a change here.
//! * **Concurrency**: the reading thread never blocks on a calculation. Calls that run
//!   commands are queued and executed one at a time, in order, by a worker thread that owns
//!   the session. `ping`, `tools/list`, `list_commands`, `describe_command` and
//!   `notifications/cancelled` are answered while the worker is busy. A cancellation sets the
//!   `TraceControl` of that call, so a running trace or scan stops and answers with the error
//!   code `cancelled`; a call still in the queue does not start.
//! * **Progress**: a request with `_meta.progressToken` receives `notifications/progress`
//!   from scans (finished cases) and from single traces (rays, sampled every 250 ms).
//!
//! [`serve`] runs the loop on any reader and writer (the tests use in-memory ones);
//! [`serve_stdio`] uses the standard streams, as `raytrace-rs mcp` does.
pub mod protocol;
mod server;
pub mod tools;

pub use server::{serve, serve_connected, serve_stdio, serve_stdio_connected};

/// What `raytrace-rs mcp --help` prints.
pub const USAGE: &str = "\
raytrace-rs mcp [--connect SESSION.json]

Starts an MCP (Model Context Protocol) server on the standard input and output: JSON-RPC 2.0,
one message per line. Register it in an MCP client, for example:

  claude mcp add raytrace-rs -- raytrace-rs mcp

The server keeps one session with an open design and offers four tools: list_commands,
describe_command, execute and batch. Every command of the CLI is available through execute.
Only protocol messages are written to stdout; logs go to stderr. The server ends when the
input closes, after the calls already received have finished.

With --connect, execute and batch operate on a GUI started with --automation-session
SESSION.json. Use session.status first and pass expected_revision to every mutation.
Relative paths are resolved in the GUI's working directory. Keep the session file private.";
