# SKILL.md Open Standard — Specification v1.0

> **Format version**: 1.0 (`SKILL_FORMAT_VERSION = "1.0"`)  
> **Last updated**: 2026-03-31  
> **Audience**: Skill authors, third-party integrators, and anyone building skills for TrueNorth or any SKILL.md-compatible agent runtime.

---

## Table of Contents

1. [What Is a Skill?](#1-what-is-a-skill)
2. [File Structure](#2-file-structure)
3. [YAML Frontmatter Schema](#3-yaml-frontmatter-schema)
4. [Markdown Body Sections](#4-markdown-body-sections)
5. [Trigger Matching Rules](#5-trigger-matching-rules)
6. [Required Tools Declaration](#6-required-tools-declaration)
7. [Progressive Disclosure: Load Levels](#7-progressive-disclosure-load-levels)
8. [Versioning and Compatibility](#8-versioning-and-compatibility)
9. [Complete Examples](#9-complete-examples)
10. [How to Create a Custom Skill](#10-how-to-create-a-custom-skill)
11. [Validation Rules](#11-validation-rules)
12. [Skill Marketplace and Community Registry](#12-skill-marketplace-and-community-registry)

---

## 1. What Is a Skill?

A **skill** is a structured Markdown instruction document that guides an AI agent through a complex, repeatable workflow. Skills are not executable code — they are declarative descriptions of *how* an agent should approach a category of task.

Skills follow the **SKILL.md open standard**, a format designed to be:

- **Human-readable** — any developer can open a skill file and understand what it does.
- **LLM-agnostic** — skills work with any underlying language model.
- **Interoperable** — the same skill file can run in TrueNorth, or any other SKILL.md-compatible runtime.
- **Versioned** — skills declare their version and minimum runtime requirements.
- **Progressive** — skill content is loaded incrementally to minimize token usage.

In TrueNorth, skills live in the `~/.truenorth/skills/` directory (or the path set by `skills_dir` in `config.toml`). They are discovered at startup and re-indexed when new files are added.

### What Skills Are Not

Skills are **not**:
- WASM modules (those are tools, registered separately in `truenorth-tools`)
- Prompts (those are runtime-generated per-request)
- Configuration files (use `config.toml` for system configuration)
- Plugins that modify TrueNorth's behavior — they cannot call Rust functions

---

## 2. File Structure

A skill file is a UTF-8 encoded Markdown file with a `.md` extension.

```
<skill-name>.md
```

The file has two parts:

```
---
<YAML frontmatter>
---

<Markdown body>
```

**Rules**:
- The file **must** begin with `---` on the very first line.
- The YAML frontmatter **must** be terminated by a second `---` line.
- Everything after the second `---` is the Markdown body.
- The file name should match the `name` field in frontmatter, kebab-cased (e.g., `research-assistant.md` for `name: Research Assistant`).

**Example minimal file**:

```markdown
---
name: My Skill
version: "1.0.0"
description: A one-sentence description of what this skill does.
triggers:
  - keyword
author: Your Name
permission_level: low
tools_required: []
---

## When to Use

Use this skill when...

## Workflow

1. Step one.
2. Step two.
```

---

## 3. YAML Frontmatter Schema

The frontmatter is a YAML document with the following fields. All fields are validated by `SkillValidator` at load time. Fields marked **required** will cause a `ParseError::MissingField` if absent.

### 3.1 Core Fields

#### `name` — string, **required**

The skill's canonical display name. Shown in skill listings and LLM skill indices. Should be human-friendly and specific.

```yaml
name: Research Assistant
```

- **Constraints**: 1–80 characters. No leading/trailing whitespace.
- **Used by**: `SkillMetadata.name`, skill listing in the LLM context, trigger matching.

#### `version` — string, **required**

Semantic version following [SemVer 2.0](https://semver.org/). Must be quoted to prevent YAML from interpreting it as a number.

```yaml
version: "1.0.0"
```

- **Constraints**: Must parse as a valid semver string (`major.minor.patch`).
- **Used by**: Skill registry deduplication (highest version wins if multiple files with same name exist), compatibility checks.

#### `description` — string, **required**

A single sentence describing what the skill does. This is injected into the LLM context at Level 0 so the model can select the right skill without loading the full body.

```yaml
description: Guides the agent through deep multi-source research workflows.
```

- **Constraints**: 10–200 characters. Should be a complete sentence.
- **Used by**: `SkillMetadata.description`, LLM skill selection context.

#### `triggers` — list of strings, **required**

A list of phrases that should activate this skill. At least one trigger is required.

```yaml
triggers:
  - research
  - investigate
  - deep dive into
  - find information about
  - "research*"
```

See [Section 5](#5-trigger-matching-rules) for full trigger syntax.

- **Constraints**: At least 1 trigger. Each trigger: 1–100 characters.
- **Used by**: `TriggerMatcher::match_triggers()` at context-gathering time.

#### `author` — string, **required**

The skill author's name or organization identifier.

```yaml
author: TrueNorth Team
```

- **Constraints**: 1–100 characters.
- **Used by**: Skill listings, attribution in community registry.

#### `permission_level` — string, **required**

The minimum permission level required to execute this skill's workflow. Must be one of the values below. The runtime enforces this — skills requesting `high` permission will fail to activate if the tool registry has not been configured to allow high-permission tools.

| Value | Description |
|-------|-------------|
| `none` | Read-only, no external side effects |
| `low` | External reads (web search, web fetch, memory query) |
| `medium` | Memory writes, vault modifications |
| `high` | Filesystem writes, shell execution |

```yaml
permission_level: low
```

#### `tools_required` — list of strings, **required** (may be empty)

The tool names that this skill's workflow relies on. Validated against the tool registry at load time. If any required tool is unavailable, the skill is marked inactive and an error is logged.

```yaml
tools_required:
  - web_search
  - web_fetch
  - memory_query
```

Use an empty list if the skill requires no tools:

```yaml
tools_required: []
```

---

### 3.2 Optional Fields

#### `sandboxed` — boolean, default `false`

Whether this skill should run in a WASM sandbox. Currently informational — TrueNorth does not yet sandbox pure-Markdown skills. Reserved for future runtimes that may execute embedded skill code.

```yaml
sandboxed: false
```

#### `tags` — list of strings, default `[]`

Categorization tags for skill discovery, filtering, and the community registry.

```yaml
tags:
  - research
  - information
  - web
```

**Recommended tags**: `research`, `code`, `writing`, `analysis`, `data`, `reasoning`, `security`, `devops`, `communication`, `creativity`.

#### `source_url` — string (URL), default absent

The canonical URL of this skill in the community registry or a public repository. Used by `SkillInstaller` to check for updates.

```yaml
source_url: "https://skills.truenorth.dev/research-assistant"
```

#### `min_truenorth_version` — string (semver), default absent

The minimum TrueNorth version required to run this skill. If the running version is below this value, the skill is marked inactive.

```yaml
min_truenorth_version: "0.1.0"
```

#### Full Frontmatter Example

```yaml
---
name: Research Assistant
version: "1.0.0"
description: Guides the agent through deep multi-source research workflows.
triggers:
  - research
  - investigate
  - deep dive
  - "find information about"
  - "research*"
tools_required:
  - web_search
  - web_fetch
  - memory_query
permission_level: low
author: TrueNorth Team
sandboxed: false
tags:
  - research
  - information
  - web
source_url: "https://skills.truenorth.dev/research-assistant"
min_truenorth_version: "0.1.0"
---
```

---

## 4. Markdown Body Sections

The body of a SKILL.md file is organized into named sections using `##` (H2) headings. TrueNorth's parser extracts these sections by name. Unknown sections are preserved in `SkillBody.raw` but not individually indexed.

### 4.1 `## When to Use`

**Purpose**: Describes the conditions under which this skill should be activated. Loaded at Level 1. Helps the LLM confirm that this is the right skill for the task.

**Best practices**:
- Start with "Use this skill when..."
- List 3–5 concrete scenarios
- Contrast with cases where the skill should NOT be used

```markdown
## When to Use

Use this skill when the user asks for comprehensive research on a topic that requires:
- Gathering information from multiple web sources
- Synthesizing findings across sources
- Producing a structured research report

**Do not use this skill** for:
- Quick factual lookups (use direct LLM response instead)
- Code generation tasks (use the Code Reviewer skill)
- Tasks the user has already constrained to a single source
```

### 4.2 `## Workflow`

**Purpose**: The step-by-step instructions the agent follows. This is the most important section. Loaded at Level 1.

**Best practices**:
- Use numbered steps for sequential workflows, bullet points for parallel or optional steps
- Be explicit about tool invocations (`Use web_search to...`, `Call memory_query with...`)
- Include decision points and conditional branches
- Keep each step actionable and specific

```markdown
## Workflow

1. **Clarify scope** (if the task is ambiguous):
   - Extract the core research question from the user's prompt
   - Identify 3–5 key sub-questions to answer

2. **Recall prior knowledge**:
   - Use `memory_query` with the research topic to retrieve any existing project memory
   - Note any previously established facts or sources

3. **Gather sources**:
   - Use `web_search` with 3 varied query formulations
   - Use `web_fetch` on the top 3 results per query
   - Prioritize primary sources, academic publications, and authoritative references

4. **Synthesize findings**:
   - Group findings by sub-question
   - Note conflicting information and flag it explicitly
   - Attribute every claim to a source URL

5. **Produce output**:
   - Write a structured report in Markdown
   - Include an executive summary, detailed findings, and a source list
   - Use `memory_query` (write) to store key findings in project memory
```

### 4.3 `## Best Practices`

**Purpose**: Style, quality, and domain-specific guidance. Loaded at Level 1.

```markdown
## Best Practices

- **Source quality**: Prefer peer-reviewed papers, official documentation, and
  established news organizations over personal blogs or social media.
- **Attribution**: Every factual claim must cite a URL. Use inline citation format.
- **Completeness**: Cover multiple perspectives. Explicitly note if a topic has
  contested evidence.
- **Recency**: For fast-moving topics, prefer sources from the last 12 months.
  Note the publication date of all sources.
- **Conciseness**: An executive summary should fit in 200 words. Detailed sections
  can be longer but must be organized with clear headings.
```

### 4.4 `## References`

**Purpose**: External resources, templates, and reference materials. Loaded at Level 2 (on demand). This section can be large.

```markdown
## References

### Report Template

```markdown
# Research Report: {TOPIC}

**Date**: {DATE}
**Researcher**: TrueNorth

## Executive Summary

{2-3 sentence overview}

## Key Findings

### {Sub-question 1}

{Finding with [source](URL) citation}

### {Sub-question 2}

...

## Sources

- [Source Title](URL) — {brief description}
```

### Recommended Search Query Patterns

- `site:arxiv.org {topic}` — academic papers
- `"{topic}" site:gov` — government sources
- `{topic} review 2025` — recent review articles
```

### 4.5 Additional Custom Sections

Skills may include additional `##` sections with any name. These are preserved in `SkillBody.raw` and can be referenced by the workflow instructions. Common custom sections:

- `## Constraints` — hard limits the agent must respect
- `## Output Format` — exact format requirements for the final output
- `## Examples` — concrete worked examples (useful for few-shot prompting)
- `## Anti-patterns` — explicit descriptions of things to avoid

---

## 5. Trigger Matching Rules

Triggers control when a skill is automatically activated. The `TriggerMatcher` evaluates all installed skill triggers against every incoming user prompt.

### 5.1 Matching Algorithm

1. The prompt is lowercased.
2. Each trigger phrase is lowercased.
3. For each trigger, one of two matching modes is applied (see below).
4. If any trigger matches, the skill receives a score contribution equal to the trigger's character length (longer = more specific = higher weight).
5. Results are sorted by total score descending.
6. The skill with the highest score above `SKILL_TRIGGER_CONFIDENCE_THRESHOLD` (0.80 normalized) is activated.

### 5.2 Matching Modes

#### Substring Match (default)

A trigger phrase matches if it appears as a substring anywhere in the lowercased prompt.

```yaml
triggers:
  - research          # matches "I want to research AI"
  - code review       # matches "please do a code review of this"
  - deep dive         # matches "Let's deep dive into this topic"
```

**Note**: Substring matching does not respect word boundaries. The trigger `"ode"` would match `"code"`. Use specific multi-word phrases to reduce false positives.

#### Glob Wildcard Match

If a trigger contains a `*` character, it is treated as a glob pattern. The `*` matches any sequence of non-whitespace characters within a single word token.

```yaml
triggers:
  - "research*"       # matches "researching", "researcher", "researched"
  - "debug*"          # matches "debugging", "debugged", "debugger"
  - "optim*"          # matches "optimize", "optimization", "optimizing"
```

The glob is applied per-word: the trigger `"research*"` is tested against each whitespace-separated token in the prompt. It does **not** match across word boundaries.

### 5.3 Specificity Weighting

The score contribution of each matched trigger is its character length:

| Trigger | Characters | Score contribution |
|---------|-----------|-------------------|
| `"research"` | 8 | 8 |
| `"deep dive into"` | 14 | 14 |
| `"find information about"` | 22 | 22 |

A skill with a single high-specificity trigger (`"find information about"`, score 22) outranks a skill with many low-specificity triggers (`"find"` + `"info"`, score 4 + 4 = 8).

**Best practice**: Include both a short keyword trigger and one or more multi-word specific phrases:

```yaml
triggers:
  - research               # catches broad mentions
  - "deep research"        # specific phrase → higher score
  - "comprehensive review" # specific phrase → higher score
  - "literature review"    # domain-specific phrase
```

### 5.4 Trigger Design Guidelines

- **Be specific**: Prefer `"code review"` over `"code"`. The word "code" appears in many prompts.
- **Cover synonyms**: Add multiple phrasings (`"investigate"`, `"look into"`, `"find out about"`).
- **Avoid common words alone**: Triggers like `"the"`, `"and"`, `"a"` will match everything.
- **Use globs for morphological variations**: `"deploy*"` handles `"deploying"`, `"deployment"`, `"deploys"`.
- **Test your triggers**: Use `truenorth skill test-triggers "your prompt here"` to see which skills would activate.

---

## 6. Required Tools Declaration

The `tools_required` field declares every tool that the skill's workflow will invoke. This declaration serves three purposes:

1. **Validation at load time**: If a required tool is not registered in the tool registry, the skill is marked `is_active: false` and a warning is logged. This prevents skills from activating and then failing mid-workflow.

2. **Permission enforcement**: If any required tool has a `permission_level` higher than the skill's declared `permission_level`, a validation error is raised.

3. **Documentation**: The tool list helps users understand what the skill will do and what capabilities it needs.

### Built-in Tools Available for Skills

| Tool name | Permission | Description |
|-----------|-----------|-------------|
| `web_search` | `low` | Web search via configured provider |
| `web_fetch` | `low` | HTTP GET and content extraction |
| `file_read` | `low` | Read file contents from workspace |
| `file_write` | `high` | Write file contents to workspace |
| `file_list` | `low` | List directory contents |
| `shell_exec` | `high` | Execute shell commands |
| `memory_query` | `low` | Search the memory layer |
| `mermaid_render` | `none` | Render Mermaid diagrams |

### Declaring WASM Tool Dependencies

For skills that require custom WASM tools (from the community registry or installed separately):

```yaml
tools_required:
  - web_search
  - custom-analysis-tool    # WASM tool must be installed and registered
```

If `custom-analysis-tool` is not in the tool registry when the skill is loaded, the skill will be inactive. Install the tool first with `truenorth skill install-tool <tool-url>`.

---

## 7. Progressive Disclosure: Load Levels

Skills are loaded in three progressive levels. This minimizes the token cost of the skill system while maintaining full workflow detail when needed.

### Level 0: Minimal (Always Loaded)

At startup, all installed skills are loaded to Level 0. Only `SkillMetadata` is loaded:

```
Fields loaded: name, version, description, triggers, tags, is_active
Token cost: ~80 tokens per skill
Max skills: All installed skills
```

This small amount of data is injected into the LLM context so the model can see what skills are available and help select the right one. The LLM reads the descriptions to understand each skill's domain.

### Level 1: Full (Loaded on Trigger)

When a skill's triggers match the user's prompt (above the confidence threshold), its full body is loaded:

```
Fields loaded: Level 0 + complete Markdown body
  (## When to Use, ## Workflow, ## Best Practices sections)
Token cost: 800–2000 tokens (capped at SKILL_LEVEL1_MAX_TOKENS = 2000)
Max concurrent: 5 skills (MAX_ACTIVE_SKILLS)
```

The full body is injected into the system prompt as a `<skill>` block before the user's task is processed.

### Level 2: Extended (Loaded On Demand)

During workflow execution, if the agent determines that it needs reference materials, examples, or templates from the skill, Level 2 is loaded:

```
Fields loaded: Level 1 + ## References section and any ## Examples sections
Token cost: Variable (may be large for reference-heavy skills)
Loading trigger: Explicit request within the workflow ("Load skill references")
```

Level 2 is never loaded automatically — it requires an explicit instruction in the workflow body such as:

```markdown
## Workflow

4. **Generate report from template**:
   - Load the report template from this skill's ## References section
   - Fill in each section with your synthesized findings
```

### Managing Token Budget

With 5 skills at Level 1 (max 2000 tokens each), the skill system uses at most ~10,000 tokens of context budget. This leaves substantial room for conversation history, memory context, and tool results.

The skill system respects the context budget manager's thresholds. If context utilization exceeds 70% (`compact_threshold`), the orchestrator may unload inactive Level 1 skills to free tokens.

---

## 8. Versioning and Compatibility

### Skill Versioning

Skills use [Semantic Versioning 2.0](https://semver.org/):

- **Major version** (`2.0.0`): Breaking change to the skill's interface. Old workflows may not work. Users are warned before upgrading.
- **Minor version** (`1.1.0`): New triggers, new sections, expanded workflow. Backwards compatible.
- **Patch version** (`1.0.1`): Corrections, clarifications, typo fixes.

When multiple skill files with the same `name` are found in the skills directory, TrueNorth uses the highest-versioned file and logs a warning about the duplicate.

### Runtime Compatibility

The `min_truenorth_version` field gates skill activation:

```yaml
min_truenorth_version: "0.2.0"
```

If `TRUENORTH_VERSION < min_truenorth_version`, the skill is loaded to Level 0 (so users see it in listings) but marked `is_active: false`. This prevents skills that depend on new features from causing errors on older runtimes.

The format version (`SKILL_FORMAT_VERSION = "1.0"`) is tracked separately from the TrueNorth binary version. A future SKILL.md v2.0 format will be backwards-compatible for reading but may add new required fields.

### Upgrade Path

When upgrading a skill:

1. Increment the version in frontmatter.
2. If removing or renaming triggers, bump the major version.
3. Run `truenorth skill validate my-skill.md` to verify the new version passes all checks.
4. Replace the old file in `~/.truenorth/skills/`. TrueNorth will reload automatically.

---

## 9. Complete Examples

### 9.1 Research Assistant

```markdown
---
name: Research Assistant
version: "1.0.0"
description: Guides the agent through deep multi-source research workflows.
triggers:
  - research
  - investigate
  - "deep dive"
  - "find information about"
  - "literature review"
  - "research*"
tools_required:
  - web_search
  - web_fetch
  - memory_query
permission_level: low
author: TrueNorth Team
tags:
  - research
  - information
  - web
---

## When to Use

Use this skill when the user asks for comprehensive research on a topic requiring
multiple web sources, synthesized findings, and a structured report.

Do **not** use this skill for:
- Simple factual questions answerable in one sentence
- Code generation or debugging tasks
- Requests limited to a single specific source

## Workflow

1. **Extract the research question**: Restate the user's request as a clear
   research question. Identify 3–5 key sub-questions.

2. **Recall prior context**:
   - `memory_query` with the research topic (scope: project)
   - Note existing findings to avoid redundant searches

3. **Search phase** (repeat for each sub-question):
   - `web_search` with 2–3 query variations
   - `web_fetch` on top 3 results per query
   - Prefer: arxiv.org, official docs, reputable news outlets, .gov/.edu domains

4. **Synthesize**:
   - Group findings by sub-question
   - Flag conflicts between sources explicitly
   - Every claim gets a source URL

5. **Deliver**:
   - Executive summary (≤ 200 words)
   - Detailed findings by section
   - Full source list with URLs
   - `memory_query` (write) to store key facts in project memory

## Best Practices

- Cite every claim. No unsourced assertions.
- Prefer sources from the last 24 months for fast-moving topics.
- When sources conflict, report both and note the disagreement.
- Use `web_search` with `site:arxiv.org` for scientific topics.
```

### 9.2 Code Reviewer

```markdown
---
name: Code Reviewer
version: "1.0.0"
description: Performs structured code review with security, performance, and style analysis.
triggers:
  - "code review"
  - "review this code"
  - "review my"
  - "check this code"
  - "security audit"
  - "find bugs"
tools_required:
  - file_read
  - memory_query
permission_level: low
author: TrueNorth Team
tags:
  - code
  - review
  - security
  - quality
---

## When to Use

Use this skill when the user asks for a structured review of code, a security
audit, or feedback on code quality, performance, or style.

## Workflow

1. **Load the code**:
   - If a file path is given, use `file_read` to load it
   - If code is provided inline, use it directly

2. **Recall prior review context**:
   - `memory_query` for any previous reviews of this file or related code

3. **Review pass 1 — Correctness**:
   - Logic errors, off-by-one errors, null/panic conditions
   - Error handling completeness
   - Edge cases (empty input, max values, concurrent access)

4. **Review pass 2 — Security**:
   - Injection vulnerabilities (SQL, command, path traversal)
   - Authentication and authorization checks
   - Secrets in source (API keys, passwords, tokens)
   - Input validation and sanitization

5. **Review pass 3 — Performance**:
   - Algorithmic complexity (O(n²) or worse in hot paths)
   - Unnecessary allocations or copies
   - Blocking operations in async contexts
   - Missing caching opportunities

6. **Review pass 4 — Style and Maintainability**:
   - Naming clarity
   - Function/method length (prefer ≤ 50 lines)
   - Comment quality (explains "why", not "what")
   - Test coverage assessment

7. **Deliver structured report**:
   - Severity classification: 🔴 Critical / 🟡 Warning / 🟢 Suggestion
   - Each finding: location, description, recommended fix
   - Summary score and prioritized action list

## Best Practices

- Flag critical security issues at the top, regardless of section order.
- Suggest specific fixes, not just descriptions of problems.
- Acknowledge what the code does well before the critique.
- If the code is too long to review in one pass, divide by module.
```

### 9.3 R/C/S Debate

```markdown
---
name: RCS Debate
version: "1.0.0"
description: Applies the Reason/Critic/Synthesis framework to complex decisions and arguments.
triggers:
  - "pros and cons"
  - "should I"
  - "help me decide"
  - "debate"
  - "analyze the tradeoffs"
  - "weigh the options"
  - "steelman"
tools_required: []
permission_level: none
author: TrueNorth Team
tags:
  - reasoning
  - debate
  - decision-making
  - rcs
---

## When to Use

Use this skill when the user needs to reason through a complex decision,
evaluate tradeoffs, steelman opposing positions, or arrive at a synthesized
conclusion from conflicting viewpoints.

## Workflow

This skill leverages TrueNorth's built-in R/C/S execution mode. If the
orchestrator has not already activated R/C/S mode for this task, request it.

1. **Frame the question**: Restate the decision or argument as a clear binary
   or multi-option question.

2. **Reason phase** (fresh context):
   - Present the strongest case for the primary position
   - List supporting evidence and assumptions
   - Identify the core insight driving this position

3. **Critic phase** (fresh context):
   - Challenge every assumption from the Reason phase
   - Present the strongest counterarguments
   - Identify logical fallacies, missing data, and failure modes

4. **Synthesis phase** (fresh context with Reason + Critic):
   - Acknowledge which criticisms are valid
   - Revise the original position where warranted
   - Deliver a nuanced conclusion that holds up to the best counterarguments

5. **Summarize**:
   - One-paragraph executive summary of the final position
   - Key conditions under which the conclusion might change
   - Recommended next steps or decision criteria

## Best Practices

- Steel-man the opposing position: present it as its strongest proponents would.
- Distinguish empirical claims (can be verified) from value claims (cannot).
- Be explicit when the synthesis reaches "it depends" — list what it depends on.
- For consequential decisions, note what additional information would change the answer.
```

---

## 10. How to Create a Custom Skill

### Step 1: Define the Skill's Purpose

Before writing a single line, answer these questions:
1. What category of task does this skill handle?
2. What makes this different from an existing skill?
3. What tools does the workflow need?
4. What is the expected output?

### Step 2: Create the Skill File

Create a new `.md` file in `~/.truenorth/skills/`:

```bash
touch ~/.truenorth/skills/my-skill.md
```

Use the minimal template:

```markdown
---
name: My Skill
version: "1.0.0"
description: One sentence describing what this skill does.
triggers:
  - primary keyword
  - "specific phrase"
tools_required:
  - web_search
permission_level: low
author: Your Name
tags:
  - category
---

## When to Use

Use this skill when...

## Workflow

1. First step.
2. Second step.

## Best Practices

- Guideline one.
- Guideline two.
```

### Step 3: Write the Workflow

The `## Workflow` section is the core of your skill. Write it as instructions for the agent:

- Number the steps sequentially.
- Be explicit about tool invocations: `Use web_search to find...`, not `search for...`.
- Include conditional branches: `If the result is ambiguous, ...`.
- Specify output format: `Produce a Markdown table with columns: ...`.

### Step 4: Validate the Skill

```bash
truenorth skill validate ~/.truenorth/skills/my-skill.md
```

This checks:
- YAML frontmatter is valid
- All required fields are present
- `tools_required` tools exist in the registry
- `permission_level` is a valid value
- Version is a valid semver string

### Step 5: Test Trigger Matching

```bash
truenorth skill test-triggers "your test prompt here"
```

This shows which skills would activate for the given prompt and their scores. Adjust your trigger phrases if the skill does not activate when expected, or if it activates for unintended prompts.

### Step 6: Test the Skill in Practice

```bash
# Run a task that should activate your skill
truenorth run --task "your test prompt here" --verbose

# Watch the event stream to see which skill was loaded
truenorth serve &
# open http://localhost:3000 to see the visual reasoning graph
```

### Step 7: Iterate

Refine the workflow based on actual runs. Common improvements:
- Add more specific trigger phrases if the skill isn't activating
- Add more explicit tool usage instructions if the agent isn't calling the right tools
- Add a `## Best Practices` section if quality is inconsistent
- Add a `## References` section with templates if the output format needs to be exact

### Step 8: Share (Optional)

To share your skill with the community:

1. Submit a pull request to the community skills repository (see [Section 12](#12-skill-marketplace-and-community-registry)).
2. Increment the version to `1.0.0` (if it was a draft at `0.x.x`).
3. Add `source_url` to the frontmatter pointing to the PR or the merged file URL.

---

## 11. Validation Rules

`SkillValidator` enforces the following rules at load time. Violations produce a `ValidationError` that marks the skill inactive.

### Hard Errors (skill will not activate)

| Rule | Error |
|------|-------|
| `name` missing or empty | `MissingField("name")` |
| `version` missing or not valid semver | `MissingField("version")` or `InvalidVersion` |
| `description` missing or < 10 chars | `MissingField("description")` |
| `triggers` missing or empty list | `MissingField("triggers")` |
| `author` missing | `MissingField("author")` |
| `permission_level` not in allowed values | `InvalidPermissionLevel` |
| Any `tools_required` entry not in registry | `MissingTool { name }` |
| `min_truenorth_version` > running version | `IncompatibleVersion` |

### Warnings (skill loads but warning is logged)

| Rule | Warning |
|------|---------|
| Duplicate skill name (multiple files) | Uses highest version, warns about duplicate |
| `description` > 200 chars | Truncated in LLM context |
| Any trigger < 3 chars | May cause false positives |
| `## Workflow` section missing | Skill body will have no workflow |
| Body exceeds `SKILL_LEVEL1_MAX_TOKENS` | Will be truncated at Level 1 |

---

## 12. Skill Marketplace and Community Registry

> **Status**: Planned for TrueNorth 0.2.0

The TrueNorth community skills registry will provide:

### Discovery

A searchable index of community-contributed skills, browsable by:
- Category tag
- Required tools
- Author
- Popularity (install count)
- Rating

### Installation

```bash
# Install from registry by name
truenorth skill install research-assistant

# Install from URL
truenorth skill install https://skills.truenorth.dev/code-reviewer.md

# List installed skills
truenorth skill list

# Check for updates
truenorth skill update --all

# Remove a skill
truenorth skill remove my-skill
```

### Publishing

```bash
# Publish to registry (requires account)
truenorth skill publish ~/.truenorth/skills/my-skill.md

# Update an existing published skill
truenorth skill publish --update ~/.truenorth/skills/my-skill.md
```

### Quality Standards for Registry Submissions

Published skills must:
1. Pass all validation rules (no hard errors)
2. Include at least a `## When to Use` and `## Workflow` section
3. Have a descriptive `description` (not just the skill name)
4. Specify `source_url` pointing to a public repository
5. Pass automated testing against a standard suite of test prompts

### Interoperability

Skills in the TrueNorth community registry follow the SKILL.md open standard and can be used by any SKILL.md-compatible runtime. The standard is intended to be adopted by other AI orchestration frameworks, enabling a shared ecosystem of agent skills across systems.

---

*For the runtime implementation of skill loading, see [`truenorth-skills`](../crates/truenorth-skills/src/lib.rs). For architecture context, see [ARCHITECTURE.md](ARCHITECTURE.md).*
