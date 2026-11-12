# Pitfalls

Places in this repository that "look right but blow up". **When you hit strange behavior (deadlock, blank screen, hang, rejected request), read this first** before debugging.

Each entry follows "symptom → cause → fix". These entries cannot be found by searching the environment or configuration — they can only be written by whoever stepped on them; new entries keep the same three sections and drop the retrospective.

## Tauri and toolchain

### Renderer blank screen

- **Symptom**: after packaging, the window opens blank while dev mode works.
- **Cause**: renderer build output loads via `tauri://`, and absolute paths do not resolve.
- **Fix**: keep `base` at `"./"` in `vite.config.ts`.

### Semantic color utility classes not generated

- **Symptom**: you defined `--color-x` but `bg-x` / `text-x` classes do not exist.
- **Cause**: Tailwind v4 only generates utility classes for tokens mapped in `@theme inline`; `--color-*` defined in plain selectors does not participate.
- **Fix**: add new colors to the `@theme inline` mapping block with token values as `var(--craft-*)`; add them to both themes (`.theme-craft-light` / `.theme-craft-dark`) in sync.

### Drag region hit scope is wrong

- **Symptom**: you added `data-tauri-drag-region` to the title-bar container, but dragging over child elements still does not move the window.
- **Cause**: Tauri hits test **per element** — only the element carrying the attribute directly is draggable; children are never draggable. This is a completely different model from Electron's rectangle-subtraction compositing.
- **Fix**: add the attribute individually to every element that should drag; never expect container inheritance.

### Synchronous domain operations blocking or panicking

- **Symptom**: a service call stalls the event loop; or it panics with "no reactor running" on a non-tokio thread.
- **Cause**: `craftwork_invoke`'s synchronous domain operations must run on `spawn_blocking`; `Runner::run` needs to build its own runtime on a reactor-less thread.
- **Fix**: new synchronous entry points follow the existing paths — wrap domain calls in `spawn_blocking`; `Runner::run` uses the dual spawn path in `runner.rs` (auto-starting a current-thread runtime). Do not `block_on` yourself.

### Subprocess output occasionally empty

- **Symptom**: output from `platform::run` occasionally misses content the subprocess clearly wrote; more visible on slow CI machines (once seen as `echo hello` assertions failing randomly).
- **Cause**: reading the shared buffer right after `try_wait` returns races the stdout/stderr reader threads, which may not have been scheduled yet; dropping the `JoinHandle` directly does not wait for the pipes to drain.
- **Fix**: after the subprocess exits, wait for the reader threads to drain the pipes (bounded 500ms in `wait_for_readers`, avoiding a hang when a grandchild holds the pipe) before reading the buffer; when adding a reader thread, register its completion flag too.

### Crate naming

- **Symptom**: `::core::` absolute paths fail and derive macros error along with them (E0432).
- **Cause**: a crate named `core` shadows Rust's built-in crate.
- **Fix**: domain crates always carry the `craft-` prefix (decision D3).

### rusqlite version specifics

- **Symptom**: assuming you need the `json` feature, or that `Connection` is shareable across threads.
- **Cause**: rusqlite 0.41 has no `json` feature (bundled SQLite 3.5x has JSON functions built in); `Connection` is not `Sync`.
- **Fix**: use the built-in JSON functions directly; share the connection through a `Mutex` (`db.lock_conn()`).

## Deadlocks and lock discipline

### `lock_conn()` nested within one expression

- **Symptom**: the process hangs, with tests stuck to timeout without an error.
- **Cause**: `db.lock_conn()` is a non-reentrant mutex; the first guard lives until **the end of the whole expression**, so a second lock in the same expression waits forever. Observed hang: 15 minutes.
- **Fix**: take the lock once per statement; split multi-step locking into separate `let` bindings and narrow scopes with blocks.

### match / if scrutinee guards

- **Symptom**: a subtler self-deadlock than the previous one; observed hanging four parallel tests.
- **Cause**: in `match get_x(&db.lock_conn(), id) { Err(_) => self.fail(...) }`, taking the lock again inside the Err arm deadlocks because the scrutinee's temporary guard lives until the match ends (Rust rule: scrutinee temporaries live to the end of the match).
- **Fix**: first `let result = get_x(&db.lock_conn(), id);` so the guard drops with the statement, then `match result`.

### A lock held across `tool.execute`

- **Symptom**: tasks in all sessions stall simultaneously, looking like the model stopped responding.
- **Cause**: `Registry::execute` held the tools lock while calling `tool.execute`, and `shell_exec` synchronously waits on a subprocess for tens of seconds to minutes — every concurrent run's `assess` / `execute` froze.
- **Fix**: inside the lock only take the `Arc<dyn Tool>` handle; execute outside it. Regression guard: `execute_runs_tool_outside_registry_lock`. Any new registry-level lock follows the same "take the handle under the lock, do the work outside".

## Agent history and requests

### Tool-call pairing breakage

- **Symptom**: OpenAI-style providers return 400 rejecting the entire history; the session locks permanently and every following message fails.
- **Cause**: an assistant message carrying tool calls must be immediately followed by **all** matching tool results — any other role message in between is a violation (once triggered by plan-review feedback inserted between the call and its results).
- **Fix**: there are four history injection points — steering, subagent reports, background command notifications, and plan-review feedback. The first three insert at the start of each iteration (the previous batch's results are already in); plan-review feedback is injected after the whole batch of tool results enters history. `repair_tool_call_integrity` (craft-model) at the request exit and history rebuild is only a defense line — **never rely on it to mask insertion-order mistakes**. The invariant and the three fix orders: `crates/craft-model/src/integrity.rs` module header.

## Paths and workspaces

### canonicalize output persisted directly

- **Symptom**: on Windows every fs / shell tool call reports "path outside the workspace, additional folders, and system temp dirs" (older copy: "path outside the workspace and additional folders"); only git tools work, and no path spelling helps.
- **Cause**: `std::fs::canonicalize` returns the verbatim form on Windows (`\\?\D:\...`). If its output is stored as the workspace root, every `root.join(rel)` path starts with `\\` and is wholesale rejected by `craft-policy::resolve`'s UNC lexical guard.
- **Fix**: pass canonicalize output through `craft_policy::resolve::strip_verbatim` before persisting or comparing (`services.rs::canon_folder`); comparison-style guards (data directory / session directory / cross-workspace ownership) must run both sides through the same helper — fixing one side silently disables the guard. Stripping at the resolution entry lives in `craft-policy::resolve` (verbatim drive forms stripped; `\\?\UNC\...` still rejected as UNC); historical stored data is normalized at read time (the RunEnv assembly point in `runner.rs`) with no DB migration.

### Temp dirs must not go into RunEnv roots

- **Symptom**: after adding the system temp directory to `RunEnv roots`, the prompt grows an "additional folders: /var/folders/..." line, Git tools start treating the temp dir as an additional folder, and skill scans scan one level more — three semantics drift at once.
- **Cause**: `roots` is not "a list of accessible directories" — it is the **workspace's source-folder order**: `roots[0]` is the primary folder (Git / environment context / worktree origin all take it), and later entries are user-mounted additional folders. Putting a non-workspace path in there forges "the user mounted this folder".
- **Fix**: temp dirs are provided only by the resolution layer (`craft_policy::resolve::resolve_existing_with_temp`, D28), and out-of-bounds judgment reads `Resolved::system_temp`; `Registry::assess` is already wired — new path-related call sites use it directly and never assemble roots themselves.

## File writes and line endings

### Write tools always fail on CRLF files

- **Symptom**: `fs_edit` succeeds on single-line edits but multi-line edits report "content to replace not found"; `apply_patch` reports "patch context does not match file content" while the content is visibly identical.
- **Cause**: on Windows with `core.autocrlf=true` (or after a Notepad save) the workspace is CRLF while `fs_read` shows LF content that returns as LF. The old implementation matched bytes exactly, and `apply_patch`'s `split('\n')` left `\r` on each line, so per-line comparison never matched. **Note that `.gitattributes`'s `text=auto` only normalizes the index, never the working-tree checkout** (`git ls-files --eol` stays `i/lf w/crlf`) — it fixes "a script rewrite wrote the index as CRLF and git diff showed a full rewrite", not this.
- **Fix**: compare on the LF form and restore `dominant_eol` when persisting (`normalize_eol` / `restore_eol` in `crates/craft-tools/src/fs.rs`, D26). New write tools reuse these three helpers and never `split('\n')` themselves; uniqueness must be counted on the normalized form (otherwise "one LF occurrence + one CRLF occurrence" is misjudged unique and edits the wrong spot).

## Models and providers

### Reintroducing an interactive terminal

- **Symptom**: someone wants PTY / xterm back to solve "command output interaction".
- **Cause**: interactive terminals were removed wholesale (D1) — eliminating the WKWebView Chinese-IME risk and node-pty cross-platform packaging obstacles together; the agent's command execution is `shell_exec`.
- **Fix**: command execution goes through `shell_exec` (own process group, env sanitizing, timeout tree-kill, background tasks). When interactive capability is needed, extend `shell_exec` and background command tasks, not the terminal form. Process groups use `tokio::process`'s `process_group(0)` and `craft-core::platform::kill_process_group` for whole-group termination (`platform.rs`).

### Hard-coded providers

- **Symptom**: a model cannot be selected, or resolves to the wrong endpoint.
- **Cause**: the app ships no built-in provider; models come only from user configuration + the catalog snapshot, and the same model ID can appear under multiple providers.
- **Fix**: remove any code branching on `openai` / `gemini` literals; resolution and selection only use `provider/model` references, with raw IDs kept solely for legacy data. No "recommended models" either.

### Hand-editing the model catalog

- **Symptom**: you edited `assets/catalog/catalog.json.gz`, the next sync overwrote it, or the Rust side never saw it.
- **Cause**: it is a generated artifact of `scripts/modelsync.mjs` and is embedded at compile time via `include_bytes!`.
- **Fix**: only change `assets/catalog/rules.json`, then run `node scripts/modelsync.mjs` and commit the snapshot, manifest, and rules, then rebuild.

## UI and state

### Same-property utility classes overriding each other

- **Symptom**: after ⌘B collapses the sidebar the whole rail is invisible and unclickable yet **still occupies 264px**; an explicitly written zero-width class does not take effect.
- **Cause**: when a conditional branch carries two classes for the same CSS property (a fixed px width class and a zeroing width class), the winner is decided by **Tailwind output order**, not source order. v4 sorts output by class name, and the fixed-px one lands after the zeroing one → the zero is overridden while `opacity-0` still applies, leaving "invisible but occupying".
- **Fix**: a property that switches by state keeps **a single source**. Dynamic values like width/height go through `style` + CSS variables (`--craft-sidebar-width`) with no hard-coded px or zeroing classes in class names; or branch-render by state (rendering nothing means no width). How to judge: after a build, look up the export order in `dist/assets/*.css` (or inspect the struck-through declaration in a web inspector).
  Also: **do not write these class-name literals in comments either** — Tailwind v4 scans source text, so a comment still makes it generate the utility class, permanently hiding the real problem.

### List rows repainting repeatedly with contradictory states (WebView2 paint ghosting)

- **Symptom**: after clicking "test connection" on the network page a few times, one row (e.g. `OpenAI / https://api.openai.com/v1`) is painted a dozen-plus times; or the button already shows "test connection" (idle) while nine "probing…" (running) rows are painted in parallel. Refreshing or re-entering the section fixes it, and the logs show nothing.
- **Cause**: these states **cannot coexist** in the data layer or frontend state — the backend manifest is 9 unique ids, the probe returns in id order, and the frontend list was unique by id with whole-block replacement. The embedded frontend bundle in the installed package (strings extracted from the exe) matched the source, ruling out a stale package. The strongest remaining suspect is "whole-subtree structural churn" combined with multi-way animation: the first version swapped an empty-state `<p>` wholesale for a 9-row list while 9 `background-clip:text` shimmer elements animated together, and WebView2 left unpainted ghosts (with button and row states visibly split on the same screen).
- **Fix**: list rows **render fixed**, updating only the single status cell inside the row — never "render the row when the result arrives" and never replacing the whole list; converge on-screen shimmer to one element (here, only the button). For similar "data cannot say that, but the UI is wrong" phenomena, suspect in this order: the state cannot hold → check whether the embedded bundle is a stale package → only then suspect the paint layer; do not change data logic based on a screenshot. Implementation: `NetworkSection` in `src/renderer/src/overlays/settings.tsx` and `probe_targets` in `services.rs`.

### The side panel has three postures; do not assume a resident 380px column

- **Symptom**: assuming the right panel starts as a resident 380px column.
- **Cause**: the side panel is not resident — `paneOpen` starts `false` (`store/app.ts`) and the closed state renders **nothing** (the old 36px icon rail was removed by D22; the open entry is the toggle at the session header's right end). The variable-width placeholder column (default 380px, draggable 280–760px) exists only while open, and maximized is a full-width drawer over the same placeholder width.
- **Fix**: width/positioning changes must first distinguish the postures (closed renders nothing / variable-width placeholder column / full-width drawer); the placeholder column and drawer share one width source.

### zustand selector infinite re-render

- **Symptom**: `The result of getSnapshot should be cached`, `Maximum update depth exceeded`, blank panel.
- **Cause**: the selector returns a new array/object (e.g. `useApp((s) => Object.values(s.x).filter(...))`), so React 19's `useSyncExternalStore` gets a fresh reference every time.
- **Fix**: selectors return only raw references or primitives from the store; derive lists inside the component with `useMemo` over stable references, or use `useShallow`.

### A hook placed after an early return

- **Symptom**: the whole window goes black with an empty DOM root (`#root` has no children) and the console reports `Rendered more hooks than during the previous render`; vite shows no compile error and every module loads — very hard to locate.
- **Cause**: App.tsx has an early return `if (!booted) return <splash/>`; a new hook (`useTitlebarActivation`) was written **after** the early return. The first render with `booted=false` takes the early return (hook not executed); after boot the re-render reaches the hook — one hook more than the previous render, and React unmounts the whole tree.
- **Fix**: every hook must be called before the early return (App.tsx's hook block, next to `useShortcuts`). A misplaced new hook does not look like "the feature does not work" — it is a whole-tree crash black screen that only explodes when something triggers a re-render.

## Images and visual input

### `Attachment.data` reverted to `String`

- **Symptom**: per-round memory and latency visibly climbing.
- **Cause**: history clones the whole batch of messages every round, and image base64 is KB–MB scale; `Arc<str>` turns clones into reference-count bumps.
- **Fix**: keep `Arc<str>` and construct with `Arc::from(...)`. Do not revert it for the convenience of calling `&mut` methods directly.

### Image token estimation semantics

- **Symptom**: assuming OpenAI's accounting is more accurate, and the context overflows early.
- **Cause**: `estimate_image_tokens` in `craft-agent/src/context.rs` (crate-private) takes Anthropic's `w*h/750` (long edge pre-shrunk to 1568) as a **conservative upper bound**; the same image counts about half under OpenAI high.
- **Fix**: keep the conservative accounting — compact earlier rather than later.

### Image dimension probing

- **Symptom**: decoding the whole image just for width/height, spiking memory or pulling in a heavy dependency.
- **Cause**: dimensions only need the header.
- **Fix**: use `craft-model/src/imageinfo.rs`'s `probe` (zero dependencies, reads only container headers by magic number); add new image formats as branches there.

### Image history bloat

- **Symptom**: old screenshots billed repeatedly, or very old images still occupying context.
- **Cause**: `history.rs` keeps only the last `KEEP_IMAGE_MESSAGES` messages with images, replacing earlier ones with placeholder text.
- **Fix**: adjusting that constant trades "old image availability" against "re-billing every round" — think it through before touching.

### Mime mismatch after image re-encoding

- **Symptom**: Anthropic rejects the whole request for a mismatched `media_type`.
- **Cause**: after converting WebP/GIF to PNG, still declaring the original type is a violation; re-encoding and updating `mime` are two separate things.
- **Fix**: pre-shrink to `VISION_MAX_EDGE` (1568) before sending to the model: **JPEG stays JPEG, everything else becomes PNG**, and update `mime` after re-encoding. Do not convert screenshots / code screenshots to JPEG — lossy compression blurs thin strokes and makes the model misread characters.

## External integrations

### Verification commands cannot see the toolchain

- **Symptom**: `pnpm` / `cargo` / `node` report `command not found` while the user's own terminal works fine; or `.zshenv` clearly exports `CARGO_HOME` but tool processes cannot read it.
- **Cause**: with no shell selected, `shell_exec` uses `/bin/sh -c` (`shellenv::default_invocation`, D25) — non-login, non-interactive, **reading no** `~/.zshenv` / `.zshrc` / `.zprofile`; the environment is also sanitized. A GUI app is launched by launchd with `launchctl getenv PATH` empty, so the process PATH is `/usr/bin:/bin:/usr/sbin:/sbin`.
- **Fix**: have the user select their shell in Settings → General → Shell Environment — environment and executor switch together (D25), with no need to hand-wrap `/bin/zsh -c`. **Do not** work around it with absolute paths like `~/.rustup/toolchains/*/bin/cargo` — that cargo lacks `CARGO_HOME`, writes the registry into `~/.cargo`, and re-downloads from the official source (measured: 400MB+ of pollution, bypassing the user's configured mirror and `rust-toolchain.toml` selection).

### Image content from MCP tools

- **Symptom**: images returned by MCP screenshot-style tools are unreadable to the model from round two on; or images are silently dropped.
- **Cause**: images are never persisted to the event store — only file names live in the `tool` message's `attachments`, and history rebuild must restore them by name. On protocol projection Anthropic requires **image-first** (images before text), and compatible gateways parse only a leading run of image blocks, so text before images loses the images.
- **Fix**: new adapters follow the existing projection rules; `caps.vision=false` never sends images, only placeholders; the budget of 5MB per image and 4 per result is enforced by `craft-mcp` (`media.rs`).

### Proxy bypass on remote calls

- **Symptom**: a proxy is configured but an MCP remote server connects directly, or gix ignores the proxy configuration.
- **Cause**: third-party libraries often build their own HTTP client (e.g. rmcp's `from_uri`) or carry their own proxy-reading logic (gix does not read the system proxy by default).
- **Fix**: resolve the proxy through `craft-core::network` and **inject it explicitly** into the other side's client or options. Confirm this first for every new outbound call.

### Python batch-operation boundaries

- **Symptom**: assuming the policy layer can understand Python script contents.
- **Cause**: after `python x.py` passes approval, writes inside the script are not path-checked — the same boundary as shell, backed by prompt constraints and audit. Only pure queries like `--version`, `-V`, `-h` classify R0; `-c` / `-m` / script paths are R2, and `pip install` is R3.
- **Fix**: batch file edits via Python use `-I` isolation; the environment already injects UTF-8 defaults (`PYTHONIOENCODING` / `PYTHONUTF8`, user values win).

## Packaging and platforms

- Signing and notarization are not enabled (decision D6): once credentials arrive, filling the config activates it; current builds are unsigned.
- Win / Linux only have the generic `bundle.targets: "all"` config; the Windows-specific config is in place (`src-tauri/tauri.windows.conf.json`, D22) while Linux has none — write cross-platform verification conclusions against this reality.

### Maximizing on a hidden window (Windows)

- **Symptom**: on-device smoke found that clicking "restore" on the Windows custom title bar collapsed the window to 14×14 (not restored to its normal size).
- **Cause**: the startup order was `maximize() → show()`. Calling maximize on a **hidden** window makes Windows record the window's then-current position as the "restore boundary"; for a frameless + shadow window (tao's undecorated shadow) the hidden position is the inset 14×14 ghost, which is what restore restores.
- **Fix**: the assembly order is fixed as `set_background_color → show → maximize` (`setup_main_window` in `src-tauri/src/lib.rs`). Show-first flashes one frame at regular size — acceptable; **never move maximize back before show** for "first sight is maximized". New startup-time window operations follow "show before geometry" alike.

### Checklist for reverting a dependency migration

- **Symptom**: after reverting a dependency migration (e.g. the custom title bar from a tauri-plugin-decoration overlay back to custom controls), dead mechanisms linger at runtime — activation requests 404, CSP allows a source that no longer exists, capabilities grant nothing, channels have no consumer.
- **Cause**: one migration touches five sites at once: dependency declarations (Cargo/pnpm), platform config (conf/CSP), capabilities, channel contracts (channels + dispatch), and components/hooks. Reverting only the core code leaves the other four behind.
- **Fix**: walk the five sites on revert: ① dependency declarations ② platform config and CSP ③ capabilities ④ channels and dispatch arms (the pairing test catches missing arms but not extra ones) ⑤ components/hooks and their consumers. Also note: reverting is not wholesale rollback — valuable fixes (e.g. the `core:window:allow-internal-toggle-maximize` double-click maximize grant) should be kept and noted in the decision.

### GUI subprocess flashing a console window (Windows)

- **Symptom**: every command execution (shell tools, MCP server startup, opening a link) flashes a console window on screen.
- **Cause**: a GUI app has no console of its own, and `std::process::Command` on Windows allocates one for each child by default.
- **Fix**: use `craft_core::platform::CREATE_NO_WINDOW` uniformly (D23); command subprocesses inject it in one place via `build_command`, while MCP stdio and the cmd carrier (`open_external`) each carry the same constant. A new subprocess path that forgets it re-triggers the bug, and tokio's `Command` has native `creation_flags` so no `CommandExt` is needed.
