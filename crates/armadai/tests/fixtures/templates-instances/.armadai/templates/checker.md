---
description: "Checks {{subject}}"
tier: pro
tags: [check]
capabilities: { write: deny, shell: deny }
native: { claude: { model: "claude-{{model}}" } }
---
You check {{subject}}: you read, you never write.

## Verdict

Answer `\{{verdict}}` on the last line, with {{ subject }} named.
