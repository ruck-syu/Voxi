# Guard review resolution — before candidate freeze

The review's example is the exact shared-unit shorthand documented in
`website/commands.html`: “Ranges work too: between 10 and 50 MB”. The existing
`parse::phrase::extract_filters` also explicitly inherits either endpoint's unit
when only one unit is supplied. The proposed candidate preserves both values
and the documented MB unit; the unit is not invented by the model.

The parent therefore rejects the proposed P1 behavior change to require two MB
tokens. It would incorrectly reject a documented supported form and remove F070's
useful recovery. This is a documentation ambiguity in the earlier contract
description, which has been clarified in the source and README.

The upper MB unit is required by this bounded certificate. Added regression
checks prove that shared-unit and repeated-unit notation produce the same intent,
while a wholly unitless range and either mixed-unit order are rejected. No
independent-bank case or outcome was inspected to make this decision.

The review found no additional confirmed wrong acceptance in the bounded domains.
Its caution about limited coverage and external scheduling/planning requirements
remains applicable. Native packaging checks were separately fixed and tested.
