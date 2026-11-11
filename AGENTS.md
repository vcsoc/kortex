# Engineering policy

Read the specification and progress documents before changes. Never execute indexed code. Keep source evidence and explicit uncertainty. Test production behavior, not fabricated successes. Run cargo fmt, clippy and workspace tests. Preserve user work. No public release until owner license approval.

## Kortex code intelligence

Read [docs/AI_AGENT_GUIDE.md](docs/AI_AGENT_GUIDE.md) before using Kortex for code investigation or changes. It defines setup, CLI/MCP usage, freshness checks, pre-edit impact assessment, post-edit validation and safe fallback when the index is unavailable or incomplete. Verify graph findings against source and tests; missing edges are not proof of safety. This guide supplements, and does not replace, the specification and progress documents.
