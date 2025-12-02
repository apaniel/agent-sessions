# Agent Sessions

A macOS desktop app to monitor all running Claude Code sessions.

![Demo](demo/claude-sessions-demo.gif)

## Features

- View all active Claude Code sessions in one place
- Real-time status detection (Thinking, Processing, Waiting, Idle)
- Global hotkey to toggle visibility (configurable)
- Click to focus on a specific session's terminal

## Installation

### Homebrew (recommended)

```bash
brew tap ozankasikci/tap
brew install --cask agent-sessions
```

### Manual

Download the latest DMG from [Releases](https://github.com/ozankasikci/agent-sessions/releases).

## Development

```bash
npm run tauri dev
```

## Tech Stack

- Tauri 2.x
- React + TypeScript
- Tailwind CSS + shadcn/ui
