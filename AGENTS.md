# AGENTS.md

Standing collaboration rules for CraftWork: where to change things, how to verify, and what must not be touched. Product introduction and feature list live in `README.md`; this file is for coding agents.

Audience is agents: every claim here must be verifiable in code or configuration, or be an environment-specific rationale or pitfall that cannot be looked up elsewhere.

## 1. Project Overview

CraftWork is a desktop agent: it reads files, edits code, runs commands, and inspects diffs inside the user's workspace. Write operations are approved before they run, with audit records and backups.

Tech stack and the three layers (find your layer before changing anything; per-crate ownership is in §5):

- `src/renderer/`: React + TypeScript frontend (Vite). Only applies events and dispatches actions; no Node APIs. Every system operation goes through the `window.craftwork` channel table (defined in `src/shared/channels.ts`, dispatched in `src-tauri`).
- `src-tauri/`: Tauri 2 desktop shell (crate name `craftwork`), the **only** layer that touches Tauri APIs: channel dispatch (`services.rs`), tray, app menu, windows.
- `crates/*`: ten Rust domain crates that each run `cargo test` standalone (agent / tools / policy / model / store / core / git / marketplace / mcp / doc). Ownership table in §5.
- Data: a single SQLite database as source of truth (`craftwork.db`, event-sourced, per-session `seq` is monotonic); keys in `keys.json` (0600); model providers and keys come entirely from user configuration — there are no built-in providers.

Deep documentation reading order: `GLOSSARY.md` (term boundaries) → `docs/arch.md` (§7 decision table D1–D33, §8 implementation status) → GitHub Issues (work tickets) → `docs/agents/` (recipes / pitfalls / domain / issue-tracker / triage-labels).

## 2. Environment and Common Commands

- Requirements: Node.js + pnpm; the Rust toolchain is pinned to 1.99.1 by `rust-toolchain.toml` (rustup installs it automatically).
- All commands go through `package.json` scripts. Do not invoke tauri / vite / cargo directly for project tasks:

```bash
pnpm install          # install frontend dependencies
pnpm dev              # desktop dev: tauri dev (frontend vite :5173, hot reload)
pnpm dev:web          # frontend only: vite --port 5173 (no Tauri shell)
pnpm typecheck        # tsc --noEmit -p tsconfig.web.json
pnpm check:rust       # cargo check --workspace (fast compile validation)
pnpm lint:rust        # cargo clippy --workspace --all-targets -- -D warnings
pnpm test:rust        # cargo test --workspace --no-fail-fast
pnpm check:docs       # verify paths / tool names / decision ids / script names in docs
pnpm build            # tauri build (full bundle, slow; required before release)
pnpm dist:dir         # tauri build --debug --bundles app (debug bundle)
```

- Rust test subsets: `cargo test -p <crate>` (e.g. `cargo test -p craft-agent`); append a test-name fragment to run a single test.
- Smoke tests always use an isolated data directory: `CRAFTWORK_HOME=/tmp/cw-<topic> pnpm dev`. Never touch the default data directory (see §10).

## 3. Routing Table

| Task | Read |
| --- | --- |
| Add built-in tool / UI capability / model protocol / update model catalog | `docs/agents/recipes.md` |
| Deadlock, blank screen, hang, rejected request, infinite re-render | `docs/agents/pitfalls.md` |
| Check whether a capability exists, what is still stubbed | `docs/arch.md` §8 |
| Why a decision was made, whether a change conflicts with it | `docs/arch.md` §7 (D1–D33 + ADR-0001/0002) |
| Behavior contract of a feature | Module-level `//!` comments and constant-side comments at the implementation; trade-offs live in the matching `docs/arch.md` §7 decision |
| Layout, sidebar widths, window sizing | `src/renderer/src/lib/resize.ts`, `components/resize-handle.tsx` (widths are per-session posture, not persisted — D17) |
| Windows custom title bar, window buttons, frameless window geometry | `src/renderer/src/components/window-controls.tsx` (inline migrated controls) + host `panel.tsx` / `conversation.tsx` / `settings.tsx` + `src-tauri/tauri.windows.conf.json` (D22; at startup show the window before changing geometry — see pitfalls) |
| Command execution environment, shell config discovery, env var stripping rules | `crates/craft-core/src/shellenv.rs` (D18; Windows detection and PowerShell capture in D24; home directory always via `platform::home_dir`) |
| Which shell runs a command, PowerShell invocation args and exit codes, zsh / csh expansion quirks (`== not found`, `no matches found`) | `command_invocation` and `invocation_args` in `crates/craft-core/src/shellenv.rs` (D25; no shell selected falls back to `/bin/sh -c` / `cmd /C`; zsh runs with `-f -o noequals -o nonomatch`, csh gets a `set nonomatch` prefix) |
| Path validation, escape detection, temp-dir allowance | `crates/craft-policy/src/resolve.rs` module header “system temp dirs are an implicit accessible root” (D28; `system_temp_roots` is the single source of truth, branch on `Resolved::system_temp`, never add temp dirs to `RunEnv roots`) |
| File write matching and line-ending behavior (CRLF replace failures) | `crates/craft-tools/src/fs.rs` module header “line-ending conventions” (D26; `.gitattributes` normalizes the index only, never the working tree — do not rely on it) |
| Git path filtering (`git_diff` / `git_log -- path` reporting “no changes”) | `path_matches` in `crates/craft-git/src/log.rs` (D30; backslash display paths, trailing separators, and `.` whole-repo are normalized at the entry — do not change `craft-policy::relative` output, approval summaries also use it) |
| What goes into a git commit (CRLF committed verbatim, `-text` in `.gitattributes` ignored) | `to_git_blob_content` in `crates/craft-git/src/commit.rs` (D13 commit section; attribute source must be `WorktreeThenIdMapping` — `IdMapping` cannot read `.gitattributes` added in the same commit) |
| Steering delivery timing, acknowledgement requirement, unanswered corrections | Steering branches in `crates/craft-agent/src/runner.rs` + behavior contract in `prompt.rs` (D19) |
| Plan mode enter / exit / auto-enter rules (multi-file write interception) | Plan protocol branches and the auto-enter gate in `crates/craft-agent/src/runner.rs` + behavior contract in `prompt.rs` + `is_file_write` / `file_key` in `crates/craft-tools/src/registry.rs` (D8 / D32) |
| web_fetch fetch / extraction / content-type branches | `crates/craft-tools/src/web.rs` module header (D33; outbound client goes through `ToolEnv.proxy_config` → craft-core::network for unified proxy and UA, D14) |
| Subagent spawn / wait / close semantics, background-by-default, delegation prompts | Spawn / wait branches in `crates/craft-agent/src/runner.rs` + `crates/craft-agent/src/subagent.rs` (tool surface and profiles) + spec and `SPAWN_STRATEGY` in `crates/craft-tools/src/agent.rs` (D15 / D31) |
| Tray, window close / quit semantics, single instance | `src-tauri/src/tray.rs` module header + `CloseRequested` / `ExitRequested` in `lib.rs` (D20) |
| Storage statistics scope, cleanup actions, orphan detection, session-delete file cleanup | `src-tauri/src/storage.rs` module header (D27) + `crates/craft-store/src/storage.rs` (SQL aggregation) + `platform::dir_usage` |
| Terms, concept boundaries, near-synonyms | `GLOSSARY.md` |
| Open issues, triage levels, which domain doc applies | `docs/agents/issue-tracker.md`, `docs/agents/triage-labels.md`, `docs/agents/domain.md` |

When a change conflicts with an existing decision, surface the conflict explicitly first, then decide whether to overturn it.

Behavior contracts never live in separate spec files: executable constraints are written as code-side comments and test names; trade-offs that code cannot carry go into the `docs/arch.md` §7 decision table. When behavior changes, update the comments — **comments are the spec**.

## 4. Hard Constraints

Violating any of these means the change is broken; roll back and redo.

- **Layering**: `crates/*` are plain Rust modules and must not import Tauri; only `src-tauri` touches Tauri APIs. Every domain crate runs `cargo test` standalone.
- **Git engine**: `gix` is only referenced inside `crates/craft-git`. All in-app Git capability (git_* tools, environment summary, sidebar status, display names, worktrees, store downloads) goes through `craft-git` (D13). Git commands the model or user runs through the shell are shell semantics and use the user's git.
- **Outbound network**: every outbound request goes through `craft-core::network`; proxy and GitHub acceleration are resolved only there (D14). Outbound `User-Agent` is injected there too (`APP_USER_AGENT` = `CraftWork/<version>`). New outbound clients must accept that module's builder exit points; do not set your own UA. No bypassing.
- **Tool registry**: the single registry for built-in tools is `builtin_tools()` in `crates/craft-tools/src/lib.rs`.
- **Providers and models**: come only from the `assets/catalog/` snapshot and user configuration; there are no built-in providers and no recommended models in code.
- **Keys**: exist only in process memory and the data directory's `keys.json` (0600). Never into logs, the database, the renderer, or the UI.
- **Approval**: request → card → user decision → run or skip; all four steps required before execution. No silent-allow path in the backend. Approval decisions and tool executions both write `audit_log`.
- **Migrations**: when changing table structure, append a new version in `crates/craft-store/src/db.rs` (`SCHEMA_VERSION`); existing migrations and released version semantics stay untouched.
- **Theme colors**: new colors are added to both `.theme-craft-light` and `.theme-craft-dark` and mapped to semantic tokens via `@theme inline`; no hard-coded color values in components.
- **File writes**: back up the original file to the data directory first (`ctx.env.backup`); use `expectedHash` to block overwriting externally modified files.

## 5. Domain Crate Ownership

Find the owning crate before adding a capability; putting it in the wrong layer violates the layering constraint above.

| Crate | Responsibility |
| --- | --- |
| `craft-agent` | Run loop, approval state machine, grant cache, history rebuild and compaction, prompt assembly, attachments, environment context |
| `craft-tools` | Tool framework (Registry) and built-in tools; registry `builtin_tools()` |
| `craft-policy` | Risk grading (R0–R4) and approval levels (graded / free), command classification, safe path resolution |
| `craft-model` | Unified model client: error normalization, model catalog, SSE, OpenAI / Anthropic adapters, factory |
| `craft-store` | SQLite source of truth: migration chain, events, runs / approvals, audit / usage / settings, Recorder |
| `craft-core` | Bus, contract types, IDs, platform capabilities (atomic write / hash / env sanitizing / subprocess), keys, line diff, outbound network resolution |
| `craft-git` | The only gix wrapper for in-app Git capability (D13) |
| `craft-marketplace` | Skill store: market source discovery, download orchestration (via `craft-git`), versioned cache, install / upgrade / rollback (D12) |
| `craft-mcp` | MCP client: dedicated thread + command loop, server pool, tool invocation, image content normalization |
| `craft-doc` | Text-level docx / xlsx / pdf read-write (no layout fidelity guarantees) |

Dependency versions are declared once in the root `Cargo.toml` `[workspace.dependencies]`; member crates use `dep = { workspace = true }` and keep only crate-specific features — never repeat a version. Add new third-party dependencies in the root file first; `Cargo.lock` is the single resolved result and is never hand-edited.
Definition of done: `grep -rn 'version = "' crates/*/Cargo.toml src-tauri/Cargo.toml` shows no version numbers for third-party dependencies.

## 6. Change Workflow

1. **Locate**: follow the routing table, read the feature's issue document and the decisions it touches.
   Done when: you can name which issue this belongs to and which decision constrains it.
2. **Land in the domain layer**: behavior is written as plain functions in domain crates, testable standalone.
   Done when: core logic in policy, path, diff, storage, agent, tool, and model layers has `cargo test` coverage.
   Behavior contracts are written as module-level `//!` comments and constant-side comments — that is the single spec carrier; update them when behavior changes.
3. **Wire up**: contract (`src/shared/channels.ts`) → dispatch in `src-tauri/src/services.rs` → renderer components and store.
   Done when: the three sites agree and the contract-vs-dispatch pairing test passes.
4. **Test naming**: English snake_case, underscore-separated behavior descriptions (`single_root_symlink_escape_rejected`).
   Done when: no Chinese or mixed-language identifiers.
5. **Verify**: see §8.
6. **Sync docs**: see §9.

## 7. Domain Conventions

### 7.1 UI and data flow

- The renderer state shell only does “apply events + dispatch actions”; `applyBatch` is the single event entry, and session messages load from snapshots.
- System operations only go through the `window.craftwork` channel table; the renderer never uses Node APIs.
- Large objects (file contents, diffs, directory trees) are fetched on demand, never kept in global state.
- Lists render with stable `key`s; the message stream sticks to bottom via `use-stick-to-bottom`.

### 7.2 Approvals

- `once` / `session` / `workspace` scopes are validated server-side; when `allowPermanent` is false the latter two degrade to `once`.
- Restricted workspace: R3 is rejected outright, permanent grants are not offered, existing grants do not apply.
- Cancel, window close, and app exit all land as deny or cancel audit records.
- `exit_plan` always requires approval, even at the free tier — the core boundary of plan mode (D8).

### 7.3 Security

- The only external-link exit is `open_external` in `src-tauri/src/system.rs`: validate, then hand to the system browser; the renderer never loads remote pages.
- Markdown renders through streamdown: `rehype-raw` + `rehype-sanitize` — benign HTML renders as HTML, dangerous content is stripped (`src/renderer/src/components/markdown.tsx`).
- Path validation happens only in the backend: `craft-policy::resolve` does lexical checks first, then resolves symlinks to real paths; renderer-provided paths are untrusted. System temp dirs are an implicit accessible root (D28, absolute paths may land there, flagged by `Resolved::system_temp`) and never enter `RunEnv roots`.
- `shell_exec` sanitizes environment variables (blacklist strips secret-like vars + keeps functional ones like `SSH_AUTH_SOCK`, D18), runs in its own process group, kills the process tree on timeout, and truncates output. The user-selected shell environment is sanitized by the same rules before reaching the child; prompts inject variable **names** only, never values.
- CSP stays on (`app.security.csp` in `tauri.conf.json`) and the capabilities allow-list is minimal per entry (`src-tauri/capabilities/default.json`); navigation interception and new-window hooks are not wired — the renderer's safety net is not loading remote pages itself.

### 7.4 Data

- SQLite is the source of truth; events increment a per-session `seq` monotonically, and seq allocation plus insert happen in one transaction.
- Event writes and broadcasts go through `craft-store::recorder::Recorder`: `append` persists and auto-broadcasts when a `part_event` mapping exists; use `emit` for extra event types. The only exception: compaction's `context_summary` persists without broadcasting (`craft-agent/src/context.rs`), while `context_compressed` has no `part_event` mapping so it needs `append` + `emit` in pair.
- `db.lock_conn()` is a non-reentrant lock: take it once per statement, split multi-step work into separate `let` bindings. Details in `docs/agents/pitfalls.md`.

### 7.5 Errors

- Backend errors are readable Chinese strings; the renderer shows them as toasts (an invoke rejection is the error string).
- Model errors only use `craft-model::errors` codes (`auth`, `rate_limit`, `timeout`, `context_overflow`, ...); new protocols map into existing codes, raw HTTP errors never escape.
- Background async failures write logs or emit an `error` event; never exit silently.

### 7.6 UI conventions

- Settings take effect immediately; unwired controls are disabled and labeled with a milestone.
- Animations respect `prefers-reduced-motion`; running states use text shimmer (`animated-gradient-text`) by default, no spinning icons. **Single exception**: progress indicators may use the lucide `Loader` spinner in exactly two places — the sidebar session status `running` slot and the end-of-stream “working” marker in the conversation; both must carry `motion-reduce:animate-none`, and the icon only signals “running” while concrete copy stays on the shimmer text.
- **Shimmer convergence on screen**: at most one shimmer element per card (e.g. the network page's “test connection” shimmers on the button while the nine probe rows are plain text). List rows render fixed and update a single cell instead of swapping whole blocks — structural churn combined with multiple `background-clip:text` animations triggers WebView2 paint ghosting (see `docs/agents/pitfalls.md`).
- Window-level shortcuts live in `src/renderer/src/hooks/shortcuts.ts`; Esc unwinds one layer at a time: overlay first, never intercepting the input box.
- Drag regions add `data-tauri-drag-region` to each draggable element individually (per-element hit testing; rationale in `docs/agents/pitfalls.md`).
- The macOS app menu is assembled only in `src-tauri/src/app_menu.rs`: patch `Menu::default`, keep the system behavior of File/Edit/Window/Help and Services. Menu actions reach the renderer via `craftwork:menu`; menu item ids are consts; add items as plain `MenuItem` + event (`PredefinedMenuItem::about` and other native items emit no events).
- The system tray is assembled only in the dedicated tray module of the app shell (`src-tauri/src/tray.rs` module header documents scope and degradation): closing the window hides to tray by default (`CloseRequested` → `prevent_close` + `hide`), `Settings.quitOnClose` switches back to quitting; ⌘Q and the tray “quit” share the same cancel / cleanup / audit path. Tray state reads backend truth only, never renderer state.

## 8. Testing and Verification

Run on every change:

```bash
pnpm typecheck
pnpm lint:rust              # clippy -D warnings
pnpm test:rust
pnpm check:docs             # verifies paths / tool names / decision ids / script names in docs
```

Faster subsets while iterating: `cargo test -p <crate>` (single crate), `cargo test -p <crate> <test-name-fragment>` (single test); the full suite is the gate before commit.

Also run when the shell layer, packaging config, or dependencies change, and before release:

```bash
pnpm build   # = tauri build, full bundle, slow
```

For UI changes, additionally:

- Smoke test with an isolated data directory: `CRAFTWORK_HOME=/tmp/cw-<topic> pnpm dev`. Tests and smoke runs use isolated directories; never the default data directory.
- Walk the chain: first-run empty state → add provider → configure key → send message → tool call → approve/deny → data panel → light/dark theme switch.
- Tray changes additionally require on-device walkthrough on macOS and Windows: close → hide/restore, first-hide notice, status count, Windows badge, ⌘Q / tray quit, duplicate-launch focus.

## 9. Commits and Documentation Sync

- Commit messages follow **Conventional Commits**: `type: description`. Type is lowercase English (`feat` / `fix` / `docs` / `refactor` / `perf` / `test` / `build` / `ci` / `chore` / `style` / `revert`); the description is one line stating what the commit does. With a scope: `type(scope): description` (e.g. `fix(git): ...`). Mark breaking changes with `!` after the type and explain in the body.
- **Merge commits are allowed**: several independent pieces of logic may share one commit (this avoids retroactively rewriting the working tree just to split history), but the commit body **must break down each piece** — the title carries the dominant type plus a summary, and each body line states what changed with its decision id or scope. A merge commit without the breakdown is considered broken. “One logical change per commit” remains the preferred default; merging is an explicitly allowed option.
- Sign off with `git commit -s` (DCO).
- Build artifacts never enter the repository: `dist/`, `target/`, `node_modules/`, `.DS_Store` (see `.gitignore`). `src-tauri/gen/` is the exception — its schema is committed.
- Line endings: **LF in the repository**, guaranteed by `.gitattributes` `text=auto` (checkouts restore per platform; with local `core.autocrlf=true` the working tree is CRLF, which is normal). Do not rewrite files wholesale with a script and commit the result — a doc once saved as CRLF without the attributes file made `git diff` show a one-line change as a full rewrite. Add new binary types to the `binary` section of `.gitattributes`.
- UI visual and interaction changes: the behavior contract goes into component/constant-side comments, trade-offs into the `docs/arch.md` §7 decision table.
- The version source of truth is `version` in `src-tauri/tauri.conf.json`; packaging picks it up automatically.

| Change | Sync to |
| --- | --- |
| Terms or concept boundaries | `GLOSSARY.md` |
| Usage, features, architecture, contribution flow | `README.md`, `CONTRIBUTING.md` |
| Architecture, main flows, decisions | `docs/arch.md` |
| Tray, close / quit semantics, single instance | `src-tauri/src/tray.rs` module header, `docs/arch.md` §7 (D20) |
| A capability's behavior contract | Module-level `//!` comments and constant comments at the implementation (the single spec carrier) |
| This file's rules, commands, pitfalls | This file and `docs/agents/` |
| A capability's implementation status | `docs/arch.md` §8 (the single source of truth) |

## 10. Data Directory

`CRAFTWORK_HOME` isolates the whole data directory; default `~/.craftwork`: `craftwork.db`, `keys.json` (0600), `backups/`, `attachments/`, `workspace/default` (built-in scratch session directory), `logs/` (`<thread-id>.log` per-session run timeline + `app.log` / `model.log`). Logs are JSONL appends; the level is adjustable in Settings → General and exists to diagnose model stream truncation and UI stalls.

The concrete files inside the directory are decided by code; this document does not duplicate the list.

## 11. Agent Workflow Skills

- **Issue tracker**: GitHub Issues (origin: https://github.com/Yangshifu1024/CraftWork), operated via the `gh` CLI — create with `gh issue create`, pick up work with `gh issue list --label ready-for-agent`. Full convention: `docs/agents/issue-tracker.md`. Behavior contracts are carried by code comments, not spec files.
- **Triage labels**: the five-role vocabulary maps to native GitHub labels of the same names; see `docs/agents/triage-labels.md`.
- **Domain docs**: single-context layout — root `GLOSSARY.md` + `docs/arch.md`.
- **Specs**: New specs saves to docs/<N-spec-title>/spec.md, then update or insert index in docs/specs.md.
