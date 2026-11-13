# Contributing to CraftWork

Thank you for wanting to improve CraftWork. This document covers preparing the environment, changing code, running checks, and committing your work.

Read [AGENTS.md](AGENTS.md) before writing code: it lists the repository's layering, approval, security, UI, and data constraints, plus the agent workflow conventions (issue tracker / triage labels / domain docs). Behavior contracts live in code-side comments (module-level `//!` and constant comments); trade-offs and boundaries live in the decision table of [docs/arch.md](docs/arch.md); terminology follows the root [GLOSSARY.md](GLOSSARY.md).

## Environment

| Dependency | Version | Notes |
| --- | --- | --- |
| Rust | 1.99.1 (pinned by `rust-toolchain.toml`; Tauri 2.12's MSRV is 1.90) | Domain kernel and Tauri shell |
| Node.js | ≥ 20 | Renderer build and directory-sync scripts |
| pnpm | latest stable | The only package manager (renderer) |
| macOS | — | The currently tested platform; Windows / Linux packaging configs exist but await on-device verification |

- Use only pnpm for package management (renderer); Rust dependencies are managed by the cargo workspace, and build-script permissions are maintained in `pnpm-workspace.yaml`.
- Do not install or upgrade tools and dependencies on your own; open an issue describing the situation before adding a tool or a major upgrade.

## Local Development

```bash
pnpm install

# desktop development mode
pnpm dev

# isolated data directory, avoids polluting ~/.craftwork
CRAFTWORK_HOME=/tmp/craftwork-dev pnpm dev

# re-sync the models.dev model catalog (generated files must be committed)
node scripts/modelsync.mjs
node scripts/modelsync.mjs --check   # check-only against upstream
```

The default data directory is `~/.craftwork` (database, keys, attachments, and the built-in scratch session directory); `CRAFTWORK_HOME` isolates it entirely. Tests and smoke runs must use an isolated directory. Multiple instances have no mutual exclusion — clean up leftover processes before testing.

## Pre-Commit Checks

Run on every commit:

```bash
pnpm typecheck    # renderer type check
pnpm lint:rust    # cargo clippy --workspace --all-targets -- -D warnings
pnpm test:rust    # cargo test --workspace --no-fail-fast (domain kernel)
pnpm check:docs   # verifies paths / tool names / decision ids / script names in docs
```

The same set is documented in [AGENTS.md](AGENTS.md) §8 (Testing and Verification). Faster subsets while iterating: `cargo test -p <crate> [test-name-fragment]`; the full suite is the gate before commit.

Local release builds default to **thin LTO** (root `Cargo.toml`) with parallel final linking; release CI overrides it back to fat via `CARGO_PROFILE_RELEASE_LTO=fat`, so the shipped artifact optimization is unchanged. To verify only the packaging pipeline, use `pnpm tauri build --no-bundle` to skip installer generation; the Windows linker is set to the toolchain's own rust-lld by `.cargo/config.toml` — no extra LLVM install needed.

UI changes additionally require:

- Isolated-directory smoke: `CRAFTWORK_HOME=/tmp/cw-<topic> pnpm dev`.
- Confirm the three-column layout and light/dark themes at a 1440×900 window.
- Walk the full chain: first-run empty state → add a provider from the model catalog → configure the key → send a message → tool call → approve / deny → create-project dialog → data panel → theme switch.

## What CI Checks

Pull Requests run `.github/workflows/ci.yml`, which includes:

| Job | Content |
| --- | --- |
| Renderer type check | `pnpm typecheck` (macOS runner) |
| Rust check matrix | `cargo check / test / clippy` on macOS / Ubuntu / Windows (non-macOS allowed to fail) |

Do not merge while CI is red.

## Code Conventions

The complete constraints live in [AGENTS.md](AGENTS.md) §7 (Domain Conventions); the most commonly tripped ones:

- **Layering**: domain crates (`craft-core`, `craft-store`, `craft-policy`, `craft-agent`, `craft-model`, `craft-tools`, `craft-doc`, `craft-git`, `craft-marketplace`, `craft-mcp`) stay plain Rust modules without Tauri (so `cargo test` works standalone); `src-tauri/src/services.rs` only does argument validation, orchestration, and error translation. Only `src-tauri` may touch Tauri APIs.
- **Events**: event writes and broadcasts go through `craft-store`'s `recorder::Recorder` (`seq` allocation and insert in one transaction); the renderer state shell only does "apply events + dispatch actions", and `applyBatch` is the single event entry.
- **Channels**: every renderer system operation goes through `window.craftwork` (the central channel table); a new capability is wired in three places in sync — `src/shared/channels.ts` (signatures), `src/renderer/src/lib/tauri-bridge.ts` (invoke implementation), and `src-tauri/src/lib.rs` (dispatch branch) — and the renderer never uses Node APIs.
- **Approvals**: approvals are decided explicitly by the user; no silent-allow path on the backend.
- **Security**: path validation (lexical + symlink real-path), command grading, key reads, and the external-link allow-list all happen in the Rust kernel; never trust paths or commands from the renderer. Keys live only in the Rust process and `~/.craftwork/keys.json` (0600).
- **Data**: SQLite is the source of truth; table changes add a new migration version and never modify existing ones; back up before file writes; rusqlite uses bundled static compilation.
- **UI**: visual values use only the semantic color tokens in `styles.css` (both light and dark themes together), no hard-coded colors; animations respect `prefers-reduced-motion`; icons use lucide-react; lists render with stable `key`s.
- **Errors**: the backend returns readable Chinese error strings shown as toasts; model errors map into the existing error codes; background async failures write logs or emit an `error` event, never exiting silently.
- **Tests**: core logic in policy, path, diff, storage, agent loop, tools, and model layers must have `cargo test` coverage; bug fixes come with a regression test.

For the steps to add UI, tools, or provider integrations, see `docs/agents/recipes.md` linked from the routing table in [AGENTS.md](AGENTS.md) §3.

## Branches and Commits

1. Fork the repository and branch from `main`, e.g. `feat/anthropic-adapter`, `fix/approval-timeout`.
2. Commit messages follow **Conventional Commits**: `type: description` — lowercase English type (`feat` / `fix` / `docs` / ...), one description line. Do not mix unrelated formatting or refactors into feature commits.
3. **Merge commits are allowed** when several independent pieces of logic land together, but the body must break down each piece (with its decision id or scope); see [AGENTS.md](AGENTS.md) §9.
4. Contributions use the DCO (Developer Certificate of Origin); sign off when committing:

```bash
git commit -s -m "feat: add Anthropic message adapter"
```

The sign-off writes `Signed-off-by: Your Name <your@email>` at the end of the message, confirming you have the right to submit the change and agree to release it under this repository's license.

## Pull Request

A PR description includes at least:

- What changed and why, linked to the related GitHub issue (e.g. `Closes #12`).
- How it was verified: which commands ran, which scenarios were covered.
- Affected docs; for UI changes, a screenshot or recording at a 1440×900 window.
- Whether new dependencies, migrations, or configuration entries were introduced.

Never commit these build artifacts:

- `dist/`
- `target/`
- `node_modules/`
- `.DS_Store`

## Documentation Sync

| Change | Sync to |
| --- | --- |
| A feature's behavior change | Module-level comments and constant comments at the implementation (the single spec carrier) |
| Overturned or new architecture / technology decisions | The architecture chapters and decision summary of `docs/arch.md` |
| Terms or concept definitions | Root `GLOSSARY.md` |
| Collaboration conventions (issue tracker / triage labels / domain docs) | The matching file under `docs/agents/` |
| Usage, features, architecture, contribution flow | `README.md`, this file |
| Development rules, commands, pitfalls | `AGENTS.md` |

## License

CraftWork is licensed under [Apache-2.0](LICENSE). Submitting a contribution means you agree to release it under the same license.

## Reporting Issues

- Issues are tracked on GitHub Issues (origin: https://github.com/Yangshifu1024/CraftWork), operated through the `gh` CLI; a new issue starts with the `needs-triage` label, and the five-role label vocabulary is in [docs/agents/triage-labels.md](docs/agents/triage-labels.md). The full convention is in [docs/agents/issue-tracker.md](docs/agents/issue-tracker.md).
- When reporting a problem, include the OS, version, reproduction steps, and relevant logs.
- For key leaks or security vulnerabilities, never paste keys, tokens, or the full data directory in public channels; contact the maintainers privately first and rotate any leaked key immediately.
