---
name: vkdg-copy
description: "Write sharp, human developer copy for VKDG — READMEs, docs, community files, release notes. No AI tells, no emojis, no em-dashes, no generic boilerplate."
---

# Writing Copy for VKDG

Reference projects: uv, Ruff, ripgrep, Turso, Bun. Study them before writing anything.

## The register

Confident technical prose. Written by someone who built the thing and has opinions about it. Not a product team. Not a docs system. A developer explaining a tool they use.

Short paragraphs. Present tense. Active voice. No exclamation points in body copy. Enthusiasm comes from proof, not punctuation.

## Opening rule

First sentence: `[subject] is a [one specific modifier] [noun] that [verb] [object].`

If written in a specific language, say so inline — 'written in Rust' earns its place by explaining the speed claim and filtering the right audience.

No scene-setting. No 'designed to'. No 'in today's landscape'. No 'modern'. Start with what the thing is.

## Proof before claims

Every speed, reliability, or scale claim needs a benchmark, a chart, or a testimonial with exact numbers and methodology. No claim without evidence. 'warm cache', 'Linux kernel source tree' — that level of specificity.

## Vocabulary filter

Run this before publishing anything:

| Replace | With |
|---|---|
| leverage | use |
| utilize | use |
| facilitate | help / let / allow |
| seamless | delete, or describe what friction it removes |
| comprehensive | list what's actually included |
| robust | say what failure modes it handles |
| empower | say what it lets the user do |
| delve | delete |
| navigate the complexities | explain the hard part |
| cutting-edge / state-of-the-art | name what it's better than and by how much |
| It's worth noting that | just note it |
| Designed to empower | say what it does |

## No emojis. No em-dashes.

Emojis in documentation signal template. Em-dashes signal AI. Neither belongs here.

Use a comma or a period instead of an em-dash. If the sentence needs an em-dash, split it into two sentences.

## Bullet rhythm

Vary length. No more than 2 bullets in the same syntactic pattern in a row. At least one bullet per list should be a fragment. At least one should contain a number.

Forbidden pattern: `Adjective noun — enables seamless X through automated Y.`

## Honest tradeoffs

Include a 'When not to use this' or 'Limitations' section. This is the highest-trust signal in any README. A model cannot write it authentically because it requires knowing what the tool is bad at.

ripgrep has 'Why shouldn't I use ripgrep?' — this is why it reads like a person wrote it.

## Persona specificity

When describing who the tool is for, name a real situation.

Bad: 'teams that need scalable AI infrastructure'
Good: 'teams running Claude Code or Codex against a single upstream account who want multi-account fallback without changing their CLI config'

## Code block comments

Every code block gets inline comments on non-obvious lines. These are often the first thing a skimming reader reads — make them do work.

## Structure for READMEs

1. One-sentence description
2. Proof of core claim (benchmark, chart, or concrete number)
3. Short factual bullets (mechanism-first, not benefit-first)
4. Installation — copy-pasteable, platform-labeled, mention one gotcha
5. Feature sections with real CLI output
6. Limitations / when not to use this
7. Contributing (specific to actual process, not generic)
8. License

## No conclusions

Do not write a conclusion section. End with the next step, a specific problem being worked on, or a link. Never summarize.

## The personality budget

One concrete personality moment per section is enough. An analogy ('Linux for payments'). An admission. An absurd proof ('general enough to run Doom on it'). A specific gotcha you actually hit.

Do not distribute personality uniformly across the text — it reads as performed. One flash per section, then back to dry technical prose.

## Anti-patterns to catch

- The landscape opener: 'In today's rapidly evolving...'
- Exhaustive hedging: 'While there are certainly valid use cases...'
- Perfectly parallel bullet lists (every bullet same syntactic form)
- Benefit-first features: 'Maximize developer productivity with...'
- The frictionless README: pristine, complete, reads like nobody in particular
- Generic contributing: 'We welcome contributors from around the world...'
- Feature lists that list abstractions, not mechanisms
- The conclusion that concludes

## Quality check

Before finishing any document, ask:
1. Could a model have written this sentence? If yes, rewrite it with a specific number, a tradeoff, or a mechanism.
2. Is there proof for every claim?
3. Does the opening say what the thing is in one sentence?
4. Are there any words from the vocabulary filter?
5. Is there at least one honest limitation?
