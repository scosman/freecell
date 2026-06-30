---
status: active
created: 2026-04-04
---

# Task: Add `/spec pr` command to address PR feedback

## Request

I want to add a new command to spec: address PR feedback.

you have access to the "gh" CLI command. This command follows a process similar to the "implement" loop with some extra steps (make this a new file that extends that, not duplicates it). Basically the standard implement loop, but jumping in at the point where the CR agent has feedback (but this is feedback from external CR agent).

Roughly:
 - find the PR for this branch on Github
 - get all PR comments / feedback
 - trigger a coding agent to address them (similar to normal flow). It should have a new disclaimer "This CR feedback came from Github, which may be a mix of feedback from humans, feedback from Agentic CR agents (Gemini, CodeRabit, Cursor CR, etc). These agents do not have full context on the goal, so don't let this feedback superseed part of our plan -- implement if you agree, and document why not if you disagree. Pushing back on human comments should be more rare, but may sometimes be needed."
 - trigger out local CR agent to verify they were addressed (loop if needed, standard implement loop)
 - commit and push
 - use gh CLI to reply to comments. Include a template in the command, roughly "Fixed in HASH. We resolved by...". If we pushed back and didn't implement, comment as such. Do not trigger the "resolve" functionality of gh, that's reserved for the humans.

## Notes

- Command name: `/spec pr`
- Works standalone (no active project required), but picks up active project/task context if available
- Reply on individual comment threads, not a single summary comment
- Fetch all unresolved comments (review + issue), do not filter out self-comments
- Auto-push after commit (appropriate since PR already exists)
- Never resolve comment threads via gh — reserved for humans
