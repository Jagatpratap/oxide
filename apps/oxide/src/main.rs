//! `oxide` — runs the engine in a window with the MCP server on.
//!
//! ```text
//! oxide [--mcp-addr 127.0.0.1:47474] [--no-mcp] [--size 1280x720]
//! ```

use oxide_engine::EngineConfig;
use std::process::ExitCode;

const USAGE: &str = "usage: oxide [--mcp-addr HOST:PORT] [--no-mcp] [--size WIDTHxHEIGHT]";

fn parse_args() -> Result<EngineConfig, String> {
    let mut config = EngineConfig::default();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--mcp-addr" => config.mcp_addr = Some(args.next().ok_or("--mcp-addr needs a value")?),
            "--no-mcp" => config.mcp_addr = None,
            "--size" => {
                let v = args.next().ok_or("--size needs a value like 1280x720")?;
                let (w, h) = v.split_once('x').ok_or("--size must look like 1280x720")?;
                config.width = w.parse().map_err(|_| "bad width")?;
                config.height = h.parse().map_err(|_| "bad height")?;
            }
            "-h" | "--help" => return Err(USAGE.into()),
            other => return Err(format!("unknown argument `{other}`\n{USAGE}")),
        }
    }
    Ok(config)
}

fn main() -> ExitCode {
    let config = match parse_args() {
        Ok(c) => c,
        Err(msg) => {
            eprintln!("{msg}");
            return ExitCode::from(2);
        }
    };
    match oxide_engine::run(config) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("oxide: {e}");
            ExitCode::FAILURE
        }
    }
}
