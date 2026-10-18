# Security policy

## Supported versions

Oxide is pre-1.0. Security fixes go into the latest release and `main`; older releases don't
get patches.

## Reporting a problem

Please don't open a public issue for a security problem. Instead, report it privately through
GitHub: open the repository's **Security** tab and choose **Report a vulnerability**. Include
what you found, how to reproduce it, and what an attacker could do with it.

You'll get a reply within a week. Once a fix is ready, it's released and the report is credited
to you unless you'd rather stay anonymous.

## Things to know

- **The MCP server listens on `127.0.0.1` only**, without authentication: any program running as
  you on the same machine can drive an open editor through it. Turn it off with `--no-mcp`, and
  don't forward its port to other machines.
- **Projects and packages run code.** Opening a project compiles and runs its Rust scripts, and
  packages can contain scripts and editor extensions. Only open projects and install packages
  you trust, as with any game engine.
- **API keys** entered in packages (for example BlenderKit) are stored in your user settings
  folder with permissions only you can read, never inside a project or a game build.
