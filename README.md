<h1 align="center">CraftWork</h1>

---

## What is this

CraftWork is a desktop agent. It reads files, edits code, runs commands, and inspects diffs in your workspace, and can also read documents, organize meeting notes, and draft emails.

Writing files, running commands, and modifying documents all go through approval first.

## Core Features

### Workspace and scratch sessions

- Workspace sessions: bound to project directories you add; files, commands, and git all operate inside that directory.
- Create-project dialog: name it (auto-derived from the folder name when empty, duplicates auto-suffixed), multiple source folders (first is primary), and per-folder trust confirmation before submit.
- Scratch sessions: chat out of the box without picking a workspace; full session capabilities, with produced files kept in the app's built-in scratch directory (`~/.craftwork/workspace/default`), automatically recreated on restart if deleted.
- The session list groups by ownership: scratch sessions pinned on top, the rest grouped by workspace. Right-click a session row to copy its ID / rename / delete; right-click the "scratch sessions" group header to clean up all scratch sessions at once (running or approval-pending ones are skipped).

### Code mode

- Read / write files, precise replace, `apply_patch` dry-run and per-hunk application, glob / grep, shell.
- Git tools: status, diff, log, and commit (never pushes by default).
- web_fetch: fetch a web page and hand the extracted text to the model (outbound network, per-approval).
- Side panel: project file tree, Word / Excel / PDF document preview, subagent status and details, and the plan tab; maximizes into a drawer that covers the conversation column (its width is kept — click "restore" or collapse to return to 380px).

### Work mode

- Text-level read and edit for Word / Excel, text extraction for PDF; scanned files report that OCR is unsupported.
- Documents go through approval before writing; editing Word text or Excel cells preserves the original structure.

### Skills

- Drop a skill at `~/.craftwork/skills/<name>/SKILL.md` (user level) or in the workspace's `.craftwork/skills/`, `.agents/skills/` (workspace level, compatible with the agents ecosystem) and the agent loads it on demand: each run injects the available-skills listing, and the model reads the body via the `skill` tool and follows it (D11).
- `/skill-name [args]` in the composer invokes explicitly: live completion and capsule highlight while typing, rendered in the stream as a capsule header plus the argument body.
- Settings → Skills: enable/disable individual skills, view the body, delete, and a master switch; disable and delete take effect in the next task.
- Commands and writes inside a skill still go through approval — skill content is an instruction package, not a permission.

### Conversation and tools

- Streaming text, reasoning collapse, tool cards, todo lists, error cards, subagent cards, and steering while running (steering messages carry a "pending delivery" marker).
- Markdown rendering: syntax-highlighted fenced code blocks (shiki, fully offline, separate palettes per theme), inline code with a tinted background.
- Message attachments: image thumbnails with zoom preview, non-image file listings; paste-to-upload supported (screenshots go straight into the conversation).
- Input prediction: after a turn ends and you idle for 5 seconds, a ghost-text suggestion for the next prompt appears in the composer (Tab to apply); typing your own content dismisses it; turn off in Settings → Personalization.
- Stop generation (click the button again or double-Esc to confirm), regenerate.
- Multiple providers: OpenAI, Anthropic, and OpenAI-compatible services; the app ships no built-in provider — import from the model catalog or create one manually.
- Model catalog: the models.dev snapshot ships offline; provider logos are downloaded and cleaned at build time into local assets, most providers prefill the endpoint on import, and context / max output / capabilities / pricing come along automatically.
- Model management: Settings → "Models & Providers" is a master-detail layout (provider list on the left + detail card on the right); supports two-level enable toggles for providers and models, model CRUD, and fetching model lists from the endpoint (merged by ID with dedupe); the chat model selector lists only enabled providers and models, and calling a disabled one returns a readable message.
- Error classification (rate limit, auth, timeout, context overflow, ...), bounded retries, and failover (disabled entries are skipped automatically); the usage panel estimates cost from catalog prices.
- Shortcuts: ⌘K command palette, ⌘N new session, ⌘B sidebar toggle, ⌘D data panel, ⌘, settings.

### Desktop integration

- System tray: closing the window hides to tray and keeps running (Settings → General switches back to "quit on close"); the tray menu shows running / approval-pending session counts, Windows adds an approval red dot, macOS counts pending approvals in the Dock badge; a duplicate launch raises the existing window instead of spawning a process.

### Approvals and security

| Risk | Meaning | Default policy |
| --- | --- | --- |
| R0 | Read-only | Auto-allowed |
| R1 | Workspace write | Asks every time; grantable for the session / workspace |
| R2 | Execute | Asks every time |
| R3 | Network or out-of-bounds | Asks every time, no permanent grants |
| R4 | Destructive | Blocked |

- Two approval tiers: graded / free; switching to "free" requires the capability list plus an "I understand the risks" confirmation, and tier changes take effect immediately. Plan mode covers the read-only investigation semantics (see below).
- Plan mode: non-trivial implementation tasks go through investigation and a plan approved by you first; after 2 modified files in a run, further file writes are intercepted and the run auto-enters plan mode for your review.
- System notifications: approval pending, completion, and failure remind you while the window is unfocused (disable in settings).
- Restricted workspaces reject network access outright and offer no permanent grants of any kind.
- Files are backed up automatically before writing, restorable from the data panel.

### Data and privacy

- SQLite (WAL, rusqlite bundled static build) stores sessions, events, approvals, usage, and settings; the events table increments a per-session `seq` monotonically and is the single source of truth for messages and parts.
- Keys live in `~/.craftwork/keys.json` (0600) and only inside the main process.
- The UI runs in the system WebView with CSP and the capabilities allow-list kept on; no remote pages are loaded; external links must be http/https and open in the system browser.
- Tool subprocesses run in a sanitized environment (secret-like variables stripped by blacklist), in their own process group, with process-tree kill on timeout and output truncation.
- No telemetry, no code or document uploads.
- Settings → About: app logo and name, version and git commit hash, build time, local database size (including WAL), and one-click open of the data directory in the system file manager. The macOS menu bar's "About CraftWork" jumps to that section (no native about panel).

## Technical Architecture

```text
┌──────────────────────────────────────────────────────────┐
│ Renderer  src/renderer (React 19 + zustand + Tailwind v4)│
│ sidebar (sessions) · stream (cards) · side panel · overlays│
└───────────────┬──────────────────────────────────────────┘
                │ invoke (craftwork_invoke, central channels.ts)
┌───────────────▼──────────────────────────────────────────┐
│ App shell  src-tauri (Tauri 2: windows, wiring, dispatch)│
│ lib.rs wiring & dispatch · services · system dialogs/links│
└───────────────┬──────────────────────────────────────────┘
┌───────────────▼──────────────────────────────────────────┐
│ Domain kernel  crates/craft-* (plain Rust, no Tauri)     │
│ agent run loop · tools · policy · model access           │
│ store SQLite & events · core bus/contracts/keys · doc    │
│ git engine · marketplace · mcp client                    │
└───────────────┬──────────────────────────────────────────┘
                │
   SQLite (rusqlite bundled) / filesystem / keys.json
```

| Layer | Directory | Responsibility |
| --- | --- | --- |
| App shell | `src-tauri` | Tauri 2 windows and wiring, `craftwork_invoke` unified dispatch, system services (dialogs / external links / clipboard / notifications) |
| UI | `src/renderer` | React UI; event-driven state shell (zustand); tauri-bridge provides the `window.craftwork` surface |
| Domain | `crates/craft-agent`, `craft-tools`, `craft-policy`, `craft-model` | Run loop and approval state machine, tools, risk grading, model access |
| Integration | `crates/craft-git`, `craft-marketplace`, `craft-mcp` | git engine (the only gix site), skill store (downloads via craft-git), MCP client |
| Infrastructure | `crates/craft-store`, `craft-core`, `craft-doc` | SQLite and events, bus / contracts / atomic write / env sanitizing / keys / outbound network, line diff, document processing |
| Shared | `src/shared` | Event contracts, types, channel table (gold-master paired against the Rust contracts to prevent drift) |

### Key design

- **The events table is the single source of truth**: messages and parts are rebuilt from event rows (snapshots), with no separate copies; user messages are persisted before the run starts.
- **Three independent defense lines**: tool-level argument validation; the policy engine decides approval; path resolution rejects out-of-bounds access (lexical + symlink real-path, double-checked).
- **The run environment is a value, not global state**: resolved once from the session's workspace at run start and passed down the call graph.
- **Approvals require an explicit decision**: request → card → user decision → run/skip; a written decision is immutable, and denials always authorize exactly once.
- **Two theme palettes**: semantic color tokens mapped via Tailwind v4 `@theme inline`, following the system or a manual switch, effective immediately.

For deeper development details see [AGENTS.md](AGENTS.md), architecture and main flows in [docs/arch.md](docs/arch.md), and the issue-tracking convention in [docs/agents/issue-tracker.md](docs/agents/issue-tracker.md).

## Getting Started

### Requirements

- Node.js ≥ 20 (renderer build and model-catalog sync scripts)
- pnpm (package manager)
- Rust 1.99.1, pinned by `rust-toolchain.toml` (rustup installs it automatically)
- macOS (the currently tested platform; Windows / Linux await on-device verification)

### Run

```bash
pnpm install
pnpm dev
```

### Build

```bash
pnpm dist        # target/release/bundle/dmg/*.dmg (Tauri bundler)
pnpm dist:dir    # .app directory only (debug build)
```

### Model catalog

The model catalog comes from [models.dev](https://models.dev), synced at build time and embedded in the source — no network needed at runtime. After updating the catalog, commit the generated files:

```bash
node scripts/modelsync.mjs          # pull upstream and regenerate the snapshot and embedded module
node scripts/modelsync.mjs --check  # check-only against upstream
```

A fresh install has no providers; in "Settings → Models & Providers" import one from the catalog (endpoint prefilled, model parameters carried over — add/remove/edit models in the detail card or fetch the latest list from the endpoint), or configure Ollama, a company gateway, etc. manually. Conversations work once a key is configured. Use `CRAFTWORK_HOME=/tmp/craftwork-dev` to isolate the data directory.

> For all development commands, directory conventions, implementation constraints, and common pitfalls, see [AGENTS.md](AGENTS.md).

## Contributing

Issues and Pull Requests are welcome. Issues are tracked on GitHub Issues; the label vocabulary and conventions are in [docs/agents/issue-tracker.md](docs/agents/issue-tracker.md). For environment setup, code conventions, check commands, and commit rules, see [CONTRIBUTING.md](CONTRIBUTING.md).

### Before you start

1. Read [AGENTS.md](AGENTS.md): commands, code structure, approval / security / UI conventions, and the agent workflow conventions (issue tracker, triage labels, domain docs).
2. For design trade-offs, check the decision summary in [docs/arch.md](docs/arch.md); when changing a feature's behavior, read that module's module-level and constant-side comments (the single carrier of behavior contracts).

### Process

1. Fork the repository and branch from `main`.
2. Make the change with matching tests (core logic in `cargo test`, covering policy, diff, storage, agent, tools, and model layers).
3. Run the checks:

```bash
pnpm typecheck     # renderer type check
pnpm test:rust     # cargo test --workspace (domain kernel)
pnpm lint:rust     # cargo clippy
pnpm check:docs    # doc reference validation
```

4. For UI changes, confirm the three-column layout and light/dark themes at a 1440×900 window.
5. Open a PR describing the change, verification, and affected docs.
6. Contributions use DCO: `git commit -s`.

### Good first areas

- Sandbox and auto-update.
- Structured parsing and writing for Word / Excel / PDF (currently text-level).
- Real-key acceptance: configure an OpenAI, Anthropic, or any OpenAI-compatible gateway key and close the tool-call loop end to end.
- Cross-platform: on-device verification of Windows and Linux packaging.

## Docs

- [Architecture](docs/arch.md): layered architecture, main flows (with flow / swimlane diagrams), security model, and the decision summary
- Behavior contracts: co-located with the implementation (module-level `//!` comments and constant comments), backed by test names for executable constraints
- Issue tracking: GitHub Issues, conventions in [docs/agents/issue-tracker.md](docs/agents/issue-tracker.md)
- [Glossary](GLOSSARY.md): the repository's authoritative domain vocabulary
- [Agent workflow conventions](docs/agents/): issue tracker (GitHub Issues), triage labels, and domain doc consumption rules
- [Contributing](CONTRIBUTING.md)
- [Development and collaboration rules](AGENTS.md)

## License

Apache-2.0, see [LICENSE](LICENSE); contributions use DCO (`git commit -s`).

The embedded model catalog data comes from [models.dev](https://models.dev) under the MIT license; see [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md) for the full notice.
