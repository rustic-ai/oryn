# Action IDs are page-local; popup and opener may both legitimately use ID 1.
def positive_integer: type == "number" and . > 0 and . == floor;
([.[] | select(.kind == "structured") | .value] == [$expected]) and
([.[] | select(.kind == "trace")] | length == 1) and
([.[] | select(.kind == "observation")] | length > 0) and
(all(.[]; .kind != "error" and .kind != "unsupported")) and
([.[] | select(.kind == "action")] | length == $actions) and
(all(.[] | select(.kind == "action");
  .result.execution_domain == "native" and
  (.result.page_id | positive_integer) and
  (.result.action_id | positive_integer))) and
([.[] | select(.kind == "action") | [.result.page_id, .result.action_id]] | unique | length == $actions) and
(all(.[]; (.kind != "trace") or all(.events[];
  .kind.kind != "script_diagnostic" and
  (.kind.kind != "console" or .kind.level != "error"))))
