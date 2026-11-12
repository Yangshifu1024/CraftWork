# Issue tracker: GitHub Issues

This repository tracks issues on GitHub Issues (origin: `github.com/Yangshifu1024/CraftWork`), operated through the `gh` CLI.

## Conventions

- **Create**: `gh issue create --title "<type: summary>" --body "<problem / acceptance / constraints>" --label needs-triage`. The body states the problem, acceptance criteria, and the decision ids (`docs/arch.md` §7) that constrain the work. Behavior contracts are still carried by code comments, not spec files.
- **Labels**: native GitHub labels using the five-role vocabulary in [triage-labels.md](triage-labels.md); a new issue starts at `needs-triage` unless it is fully specified.
- **Read**: `gh issue view <number>`; list with `gh issue list --label <label>`. Pick up work with `gh issue list --label ready-for-agent`.
- **Progress**: use GitHub comments; one issue tracks one piece of work. Reference the implementation location (`file:line` or function name) and the decision ids when closing.
- **Done**: close once the implementation is merged and verified; keep the decision ids and verification method in the body or a comment.
- **Historical note**: the three work tickets created before the GitHub Issues migration (all implemented) were removed with the migration; their implementation records live in git history and the `docs/arch.md` §7 decision table.

## When a skill says "publish to the issue tracker"

Create the issue with `gh issue create` (title + body + one of the five-role labels). If the work is a behavior change, land the conclusions in code comments first, then reference the implementation location (`file:line` or function name) and the `docs/arch.md` decision ids in the issue body — never restate rules from thin air.

## When a skill says "fetch the relevant ticket"

`gh issue view <number>`, or find claimable work with `gh issue list --label ready-for-agent`.
