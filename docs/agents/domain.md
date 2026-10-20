# Domain docs

This repo uses a single-context layout.

## Before exploring

- Read `GLOSSARY.md` at the repo root.
- Read relevant decisions in `docs/adr/`.

If these files do not exist, proceed silently. Domain modeling creates
them when terms or decisions are resolved.

## Layout

- `GLOSSARY.md`: shared domain vocabulary.
- `docs/adr/NNNN-decision-title.md`: architectural decision records.

## Use the glossary's vocabulary

Use defined terms in issue titles, proposals, hypotheses, and test names.
Follow the glossary's guidance about synonyms.

When a needed concept is missing, reconsider whether it belongs to the
project's domain; record real vocabulary gaps for `/domain-modeling`.

## Flag ADR conflicts

If a proposal contradicts an existing ADR, identify the ADR and explain
why the decision should be reconsidered.
