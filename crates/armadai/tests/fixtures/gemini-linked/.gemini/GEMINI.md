# nested

See [agents/nested.md](agents/nested.md) for the full agent prompt.


## ArmadAI Response Protocol

Follow this protocol for all responses:

1. When finished responding, end with this marker on its own line:
   <!--ARMADAI_END-->

2. When delegating to a sub-agent, prefix with:
   <!--ARMADAI_DELEGATE:agent-name-->

3. Before the END marker, include metadata:
   <!--ARMADAI_META:status=complete-->
