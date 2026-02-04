# Rclaw 🦐

> 🚧 WORK IN PROGRESS 🚧

**Rclaw** is a lightweight Rust imitator of [OpenClaw](https://github.com/openclaw/openclaw), designed to provide a local AI assistant interface with tool-calling capabilities and scheduled tasks. Rclaw aims to be a more secure AI assistant running in isolated containers.

![Rust](https://img.shields.io/badge/built_with-Rust-dca282.svg)
![License](https://img.shields.io/badge/license-MIT-blue.svg)

![](docs/rclaw-example.png)

## 🚀 Vision

**Rclaw** aims to provide the core capabilities of [OpenClaw](https://github.com/openclaw/openclaw) but with the performance, safety, and single-binary convenience of Rust.

- **Secure by Design**: Agents run in isolated Docker containers 🚧
- **Lightweight**: A single compiled binary with minimal footprint 🚧
- **TUI Native**: Includes a built-in Terminal User Interface (Ratatui) for monitoring and control.
- **Database Backed**: Uses SQLite for reliable message queuing and task scheduling.

## 🛠️ Internals

- **Core**: Rust (Tokio async runtime)
- **Database**: SQLite (`rusqlite`)
- **UI**: Ratatui + Crossterm
- **Isolation**: Docker Containers 🚧

More info in [INTERNALS.md](docs/INTERNALS.md).

## 📦 Installation & Usage

### Prerequisites

- [Rust](https://www.rust-lang.org/tools/install) (latest stable)
- [Docker](https://docs.docker.com/get-docker/) (must be running)

### Build from Source

```bash
# Clone the repository
git clone https://github.com/carlosas/rclaw.git
cd rclaw

# Build the project
cargo build --release
```

### Running Rclaw

To start the assistant with the interactive TUI:

```bash
cargo run -- start
```

_(Note: Docker must be running for the agent execution to work)._

## 🚧 Status

**Work in Progress.**

- ✅ TUI (Terminal Interface)
- ✅ Database Layer (Schema & connection)
- ✅ Gemini CLI integration (Oauth2 trick)
- 🚧 Container Runners (Pending)
- 🚧 Session memory (Pending)
- 🚧 Long-term memory (Pending)
- 🚧 Task Scheduler (Pending)
- 🚧 Custom skills (Pending)
- 🚧 Claude Code integration (Pending)

## 🤝 Contributing

This is a personal project, but suggestions are welcome!

1.  Fork it!
2.  Create your feature branch: `git checkout -b my-new-feature`
3.  Commit your changes: `git commit -am 'Add some feature'`
4.  Push to the branch: `git push origin my-new-feature`
5.  Submit a pull request :)
