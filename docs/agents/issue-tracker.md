# Issue tracker: GitHub

Issues and specs live in GitHub Issues for `itsmrma/scrobbl-rs`.
Use the `gh` CLI from this repo; it infers the repository from the remote.
Outside the clone, pass `--repo itsmrma/scrobbl-rs`.

## Conventions

- Create: `gh issue create --title "..." --body-file <file>`.
- Read: `gh issue view <number> --json number,title,body,labels,comments`.
- List: `gh issue list --state open --json number,title,body,labels,assignees`.
  Apply `--label` and `--state` filters as needed.
- Comment: `gh issue comment <number> --body-file <file>`.
- Edit body: `gh issue edit <number> --body-file <file>`.
- Apply or remove labels: `gh issue edit <number> --add-label "..."` or
  `--remove-label "..."`.
- Close: `gh issue close <number> --comment "..."`.
- Create a child: `gh issue create --parent <parent> --title "..." --body-file <file>`.
- Attach an existing child: `gh issue edit <parent> --add-sub-issue <child>`.
  If sub-issues are unavailable, add the child to the parent's task list and
  put `Part of #<parent>` at the top of the child body.

Use a temporary file for multiline bodies.

## Pull requests as a triage surface

**PRs as a request surface: no.**

## Skill vocabulary

When a skill says "publish to the issue tracker", create a GitHub issue.
When it says "fetch the relevant ticket", read the GitHub issue.

## Wayfinding operations

For `/wayfinder`:

- Map: one issue labelled `wayfinder:map`, containing Notes,
  Decisions-so-far, and Fog.
- Child ticket: link it to the map using sub-issues, with the task-list
  fallback above. Label it `wayfinder:<type>`, where type is `research`,
  `prototype`, `grilling`, or `task`.
- Blocking: use native GitHub issue dependencies:
  `gh issue edit <child> --add-blocked-by <blocker>`.
  If dependencies are unavailable, put `Blocked by: #<number>` at the top
  of the child body. A ticket is unblocked when all blockers are closed.
- Frontier: inspect the map's open children in map order. Select the first
  unassigned child with no open blockers.
- Claim: `gh issue edit <number> --add-assignee @me`, as the session's first write.
- Resolve: comment with the answer, close the ticket, then append a concise
  finding and ticket link to the map's Decisions-so-far.
