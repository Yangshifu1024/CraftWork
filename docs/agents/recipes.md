# Recipes for New Capabilities

Browse by branch. Each branch gives complete steps and a definition of done; **an unmet definition of done means the work is not done**.

Main-line rule: domain logic lands in a domain crate first, `src-tauri/src/services.rs` only does argument validation, orchestration, and error translation, and the renderer only goes through the channel table.

## Adding a tool

Trigger: adding a capability the agent can call.

1. Implement `Tool` in `crates/craft-tools/` (`spec` + `execute`). `execute` is **synchronous**; the caller is responsible for spawning it on a blocking thread.
2. In `spec`, declare the risk level, path argument, command argument, Work-mode availability, title, and summary.
   - `title` **must be a human-readable Chinese phrase** ("读取文件", "执行命令") — it appears in the conversation's tool rows, approval card titles, and desktop notifications; leaving it empty falls back to an internal identifier like `fs_read`.
   - `summary_arg` picks the target argument; with no suitable argument, use `summary_text` for a fixed summary.
3. Add it to `builtin_tools()` in `crates/craft-tools/src/lib.rs`. **This is the single source of truth for built-in tool registration** — not adding it means the tool is never registered; the shell's `register_builtin_tools` iterates it, so never list tools anywhere else.
4. Path resolution and output truncation are handled centrally by the `Registry`; do not reimplement them inside the tool.
5. For outbound-network tools, build the HTTP client through `ctx.env.proxy_config()` applied over a craft-core::network exit — proxy and the unified UA come from there (D14). Never construct a bare reqwest client (see web_fetch in `crates/craft-tools/src/web.rs` for the pattern).
6. Tools with write semantics hand old/new text to `ctx.attach_diff(&mut meta, old, new)` so the conversation's tool card renders a line diff on expand (D21). It writes no field when old equals new and auto-degrades to stats form over 64KB. **Never assemble a diff payload inside the tool yourself**.
7. Add tests through the `Registry::assess` + `Registry::execute` seam.

**Definition of done**: `builtin_tools()` contains the new tool; `every_builtin_tool_has_human_readable_title` (`craft-tools/src/lib.rs`) passes; `pnpm test:rust` is green.

### Protocol tools are the exception

Protocol tools that need the Runner to hold runtime state skip the tool implementation and go through Runner special-casing:

- `enter_plan` / `exit_plan` state transitions live in the Runner (D8).
- `agent_spawn` / `agent_wait` / `agent_list` / `agent_close` dispatch and reporting live in the Runner's `run_subagent_tool` (D15).

Such tools' `execute` only does argument validation and confirmation text, **holding no runtime state**.

## Adding a UI capability

Trigger: adding a UI feature or a system operation.

1. Land the data and behavior in a domain crate (or a deeper module) first, as plain functions.
2. Add the method signature to `CraftworkApi` in `src/shared/channels.ts`.
3. Wire the dispatch and service function in `src-tauri/src/services.rs` in sync. Three sites must agree: the contract (`src/shared`), the bridge (`lib/tauri-bridge.ts`), and the dispatch.
4. Components go in `src/renderer/src/components/` or `overlays/`; state goes into the zustand store, and events are applied via `applyBatch`.
5. Colors, font sizes, and spacing only come from semantic tokens (`bg-background`, `text-foreground-subtle`, etc.). Icons use lucide-react with on-demand imports.

**Definition of done**: `pnpm typecheck` is green; the contract serialization test and the dispatch wiring pairing test pass; no hard-coded color values in the UI.

## Adding a provider protocol

Trigger: supporting a new model service protocol or gateway. **Mind the boundary**: this is not "adding a provider" — providers are added by users; code only adds protocol adapters.

1. The adapter lands in `crates/craft-model/`, mapping to existing error codes (`craft-model::errors`).
2. The protocol allow-list has exactly three: `openai`, `anthropic`, `openai-compatible`. Native Google/Gemini, Azure, Bedrock, and Vertex are **exclusions** — do not integrate them.
3. Outbound requests go through `craft-core::network` (D14); never construct a bare reqwest client.
4. New capability switches go through `caps` (e.g. `vision`); when a capability is false, degrade instead of erroring.

**Definition of done**: the new protocol appears in no branch other than error normalization; `craft-model` unit tests cover the error-code mapping; `cargo test -p craft-model` is green.

## Updating the model catalog

Trigger: catalog models are stale, or a new model must be added.

1. Only touch the rules: `assets/catalog/rules.json`.
2. Run `node scripts/modelsync.mjs` (add `--check` to only check for drift).
3. Commit the snapshot (`catalog.json.gz`), the manifest, and the rules.
4. The Rust side embeds the file at compile time via `include_bytes!` — it **must be recompiled** to take effect.

**Definition of done**: `node scripts/modelsync.mjs --check` reports no drift.

Hard boundary: `catalog.json.gz` is a generated file — hand edits are ineffective and overwritten by the next sync; no built-in providers or model lists are added in code; user providers live in the `model.custom_providers` setting; sessions and settings store `providerID/modelID` references, never a bare model ID; a fresh install has no default model — the user must add a provider and pick a model before sending a message.
