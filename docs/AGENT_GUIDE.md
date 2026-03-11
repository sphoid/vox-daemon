# Agent Orchestration Guide

## How This Setup Works

This project uses Claude Code's **custom subagent** system to parallelize development with specialist agents. Here's the mental model:

```
┌──────────────────────────────────────────────────────┐
│                   YOU (Human)                         │
│              Interact with main session               │
└────────────────────┬─────────────────────────────────┘
                     │
                     ▼
┌──────────────────────────────────────────────────────┐
│             ORCHESTRATOR (Main Session)               │
│                  Model: Opus                          │
│          Reads CLAUDE.md for instructions             │
│     Plans work, dispatches agents, tracks progress    │
└──┬──────────┬──────────┬──────────┬──────────────────┘
   │          │          │          │
   ▼          ▼          ▼          ▼
┌────────┐┌────────┐┌────────┐┌────────┐
│ audio- ││  ai-   ││ gui-   ││  qa-   │
│special-││special-││special-││review- │
│  ist   ││  ist   ││  ist   ││   er   │
│        ││        ││        ││        │
│Sonnet  ││Sonnet  ││Sonnet  ││ Opus   │
│        ││        ││        ││        │
│PipeWire││Whisper ││iced    ││Reviews │
│audio   ││LLM API ││tray    ││PRD     │
│capture ││diarize ││notify  ││checks  │
└────────┘└────────┘└────────┘└────────┘
```

### Key Concepts

**Custom subagents** are defined as Markdown files in `.claude/agents/`. Claude Code discovers them automatically and can spawn them via the Task tool. Each agent:
- Has its own **context window** (isolated from the orchestrator)
- Has a scoped **system prompt** defining its expertise
- Can be restricted to specific **tools** (e.g., QA reviewer can't write production code)
- Runs on a configurable **model** (Sonnet for speed, Opus for depth)
- **Inherits** the project's `CLAUDE.md` context automatically

**The orchestrator** (your main Claude Code session) reads `CLAUDE.md` and decides when to dispatch which agent, tracks progress, and coordinates the implement → review cycle.

---

## Getting Started

### 1. Project Setup

```bash
# Create the project repository
mkdir vox-daemon && cd vox-daemon
git init

# Copy in the provided files
# CLAUDE.md          → ./CLAUDE.md
# PRD.md             → ./PRD.md
# .claude/agents/*   → ./.claude/agents/

# Verify agent structure
ls -la .claude/agents/
# audio-specialist.md
# ai-specialist.md
# gui-specialist.md
# qa-reviewer.md

# Create progress tracking
mkdir -p docs
touch docs/progress.md docs/qa-log.md
```

### 2. Start Claude Code

```bash
# Start Claude Code with Opus for the orchestrator
claude --model opus
```

### 3. Verify Agents Are Loaded

Inside Claude Code, run:
```
/agents
```

You should see all four agents listed: `audio-specialist`, `ai-specialist`, `gui-specialist`, and `qa-reviewer`.

### 4. Begin Implementation

Start by telling the orchestrator what to build. For example:

```
Let's begin Phase 1 of the PRD. Start by setting up the Cargo workspace 
with all the crate stubs, then implement vox-core with the shared types, 
config handling, and XDG path resolution. After that, dispatch the 
audio-specialist to implement vox-capture.
```

The orchestrator will:
1. Set up the workspace itself (simple scaffolding)
2. Dispatch `audio-specialist` for the PipeWire capture work
3. After completion, dispatch `qa-reviewer` to validate against the PRD

---

## Orchestration Patterns

### Pattern 1: Sequential (Feature Implementation)

For features with dependencies, use sequential dispatch:

```
You → Orchestrator: "Implement the Whisper transcription pipeline"
  Orchestrator → ai-specialist: "Implement vox-transcribe with whisper-rs..."
  ai-specialist completes → reports to orchestrator
  Orchestrator → qa-reviewer: "Review vox-transcribe against PRD Section 4.2..."
  qa-reviewer completes → reports issues
  Orchestrator → ai-specialist: "Fix these issues: [list from QA]..."
  Repeat until qa-reviewer approves
```

### Pattern 2: Parallel (Independent Features)

When working on features in different crates, dispatch in parallel:

```
You → Orchestrator: "Implement the system tray and notifications 
                     while also adding Markdown export to vox-storage"
  Orchestrator → gui-specialist: "Implement vox-tray and vox-notify..."
  Orchestrator → ai-specialist: "Add Markdown export to vox-storage..."
  (Both run simultaneously)
  Both complete → orchestrator dispatches qa-reviewer for each
```

To background a subagent while it works, press **Ctrl+B** during its execution. Then use `/tasks` to monitor progress.

### Pattern 3: Feedback Loop (QA-Driven Iteration)

```
qa-reviewer reports: "NEEDS REVISION — 3 issues found"
  Orchestrator reads the QA log
  Orchestrator → [appropriate specialist]: "Fix issues #1, #2, #3 from QA review"
  Specialist fixes → orchestrator re-dispatches qa-reviewer
  qa-reviewer: "APPROVED"
  Orchestrator updates docs/progress.md
```

---

## Tips for Effective Orchestration

### Give the orchestrator clear instructions

**Good:**
> Implement vox-capture per PRD Section 4.1. Focus on PipeWire stream 
> enumeration and dual-stream capture. Don't worry about resampling yet — 
> we'll add that in a follow-up.

**Bad:**
> Build the audio stuff.

### Scope subagent tasks tightly

Each subagent dispatch should be a single, well-defined task. Don't ask an agent to implement an entire crate in one shot — break it into 2-3 focused tasks.

### Use the QA agent liberally

Run `qa-reviewer` after every significant feature completion. It's cheaper to catch issues early. The QA agent runs on Opus for thorough analysis, but it's read-only so it won't accidentally break things.

### Monitor progress

Check `docs/progress.md` and `docs/qa-log.md` regularly to see what's been implemented and what issues remain.

### Keep agents focused

If you find a specialist agent trying to work outside its scope (e.g., `audio-specialist` trying to modify the GUI), redirect it. The CLAUDE.md rules and the agent's system prompt should prevent this, but it's worth watching for.

---

## Customizing Agents

The agent definitions in `.claude/agents/` are just Markdown files. Feel free to modify them as the project evolves. Common customizations:

- **Add more context** to the system prompt as architectural decisions solidify
- **Restrict tools further** if an agent is doing things it shouldn't
- **Change the model** if you find Sonnet isn't capable enough for a particular specialist
- **Add new agents** — for example, a `docs-specialist` for Phase 4

To reload agents after editing, restart your Claude Code session or run `/agents`.

---

## Cost Considerations

| Agent | Model | Usage Pattern | Relative Cost |
|-------|-------|--------------|---------------|
| Orchestrator | Opus | Always active, light reasoning | Medium |
| audio-specialist | Sonnet | Focused coding bursts | Low |
| ai-specialist | Sonnet | Focused coding bursts | Low |
| gui-specialist | Sonnet | Focused coding bursts | Low |
| qa-reviewer | Opus | Thorough analysis, read-heavy | Medium |

**Cost optimization tips:**
- Sonnet for implementation agents is the sweet spot — capable enough for focused Rust coding, 3-4x cheaper than Opus
- Opus for QA is worth it — the reviewer needs deeper reasoning to catch subtle spec violations
- Background long-running agents (Ctrl+B) so you're not blocked waiting
- Max 3-4 specialists — more agents means more coordination overhead than benefit
