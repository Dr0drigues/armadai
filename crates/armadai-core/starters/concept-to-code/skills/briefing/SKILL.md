---
name: briefing
description: "Shape of the project brief (.armadai/docs/brief.md) and of good success criteria. Use when an idea must be turned into a brief, when the user types /brief, when the brief is refined after a scope change, and before judging work against the brief."
---
# Briefing

A brief says what problem is solved, for whom, how success is recognised and what is out of scope. It is the reference every step is judged against: a vague brief makes every later step a guess.

## Where

- `.armadai/docs/brief.md`: the working brief of the team. Refine it in place; never start a second one.
- Once the user confirmed it: `docs/functional/<domain>/<feature>.md`, the human version (what a user can do, the rules, examples), in the user's language. No working notes there.

## Shape of `.armadai/docs/brief.md`

```markdown
# Brief: <name>

Updated: YYYY-MM-DD

## Problem
Two to four sentences: who has what problem today, and what it costs them.

## Users
- <kind of user>: what they need, in what context.

## Success criteria
- C1: <an observable behaviour, checkable by yes or no>
- C2: …

## Non-goals
- <what this iteration deliberately does not do>

## Constraints
- <deadline, platform, data, budget, compliance… only those that exist>

## Open questions
- <what the user has not settled yet>

## Changes
- YYYY-MM-DD: <what changed in the scope, and why>
```

## Good success criteria

- Observable and checkable: someone who did not write the code can say yes or no. "Fast" is not a criterion; "the list of 1,000 projects shows in under one second" is.
- One behaviour each, numbered `C1`, `C2`…: the validator reports per criterion.
- Written from the user's side (what they can do, see, get), not from the code's.
- Few: five to ten for a first iteration. More means the scope should be split.

## Asking the questions

Ask one question at a time, the one whose answer changes the brief the most, until every section is filled:

1. the problem and who has it;
2. what success looks like for them (it gives the criteria);
3. what is out of scope for now (non-goals are as useful as goals);
4. the constraints that really exist.

Offer 2 to 4 concrete options whenever the answer can be a choice, with your recommendation, and always leave room for another answer. Never fill a gap with a guess: write it under "Open questions" and ask it.

- In the main conversation (the `/brief` command), ask with your question tool, one question per call.
- As a subagent (`analyst`), you cannot ask: return the question to `lead` under "Questions for the user"; `lead` asks and sends the answer back.

Finish by showing the user the success criteria and non-goals, and asking them to confirm or correct. Record the confirmation date in "Updated".

## Refining after a scope change

Start from what changed (the validator's report, the user's words). Update the sections concerned, add a dated line under "Changes", and say which steps already done are affected (mockups, design, code).

Speak the user's language, in the questions and in the brief.
