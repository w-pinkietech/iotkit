# CLAUDE.md

This file is the Claude Code entrypoint for this repository.

**Read and follow [`AGENTS.md`](./AGENTS.md).** It is the index for project
context, documentation authority, change map, issue-driven workflow, commands,
source placement, review, and verification. Detailed rules are linked from
there under [`.agents/`](./.agents/).

Do not redefine those rules here. Claude-specific plugins, skills, or defaults
are optional aids only; they never override the user request or `AGENTS.md`.
If an instruction conflicts with a current contract or repository invariant in
`AGENTS.md` / `.agents/`, stop and report the conflict.
