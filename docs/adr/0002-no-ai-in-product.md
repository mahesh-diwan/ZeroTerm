# ADR-0002: No AI features in the product

- **Status:** Accepted
- **Date:** 2026-08-11
- **Deciders:** Project owner

## Context and Problem Statement

ZeroTerm previously shipped AI functionality: a `zeroterm-ai` crate (LLM
client) and an AI overlay/completion surface in the app. The owner directed
that all AI be removed from the product and the focus shift to core terminal
excellence — *"remove all ai from our product and look for what more features
can be added rather than what existing features can be made better."* AI
features are a popular re-suggestion; without a recorded reason, a future
contributor could re-add them without knowing why they were removed.

## Decision Drivers

- **Owner's product direction** — ZeroTerm is a terminal, not an AI
  assistant; differentiation comes from terminal UX and robustness.
- **Scope discipline** — AI overlays consumed UI chrome, keybindings, and
  maintenance effort that belongs to the terminal itself.
- **Re-suggestion prevention** — a written record stops future "why isn't
  there AI?" churn.

## Considered Options

1. **Keep AI as an optional feature** (config-gated, opt-in).
   - Rejected: the owner explicitly removed it; optionality still spends
     maintenance and UI surface on it.
2. **Remove AI entirely.**
   - Accepted: the `zeroterm-ai` crate and the AI overlay/completion were
     deleted; the product ships with no AI client, no AI keybindings, no AI
     config surface. The line editor's ghost-suffix rendering was retained
     only as a tested display helper (production uses it with an empty
     suffix).

## Decision Outcome

ZeroTerm ships **without** AI: no LLM client, no AI overlay, no AI
completion, no AI configuration. The product's roadmap is terminal features
(shell integration, exit-code awareness, rendering, TUI compatibility, UX).

### Consequences

- **Positive:** leaner app, fewer keybinding/overlay conflicts, clearer
  product identity, all engineering effort on terminal quality.
- **Negative:** no AI-assisted workflows for users who might want them; the
  editor ghost-suffix display helper remains but is not user-facing.
- **Follow-up:** if AI ever returns, it must be a deliberate product decision
  that revises this ADR — not a drive-by re-add.
