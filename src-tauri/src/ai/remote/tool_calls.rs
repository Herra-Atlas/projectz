use std::collections::BTreeMap;

#[derive(Debug, Default)]
pub struct StreamedToolCalls {
    calls: BTreeMap<usize, ToolCallBuffer>,
}

#[derive(Debug, Default)]
struct ToolCallBuffer {
    id: Option<String>,
    name: String,
    arguments: String,
}

#[derive(Debug)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: String,
}

impl StreamedToolCalls {
    pub fn append(&mut self, deltas: &[serde_json::Value]) {
        for delta in deltas {
            let Some(index) = delta.get("index").and_then(serde_json::Value::as_u64) else {
                continue;
            };
            let call = self.calls.entry(index as usize).or_default();
            if let Some(id) = delta.get("id").and_then(serde_json::Value::as_str) {
                call.id = Some(id.to_string());
            }
            if let Some(name) = delta
                .pointer("/function/name")
                .and_then(serde_json::Value::as_str)
            {
                call.name.push_str(name);
            }
            if let Some(arguments) = delta
                .pointer("/function/arguments")
                .and_then(serde_json::Value::as_str)
            {
                call.arguments.push_str(arguments);
            }
        }
    }

    pub fn finish(self) -> Vec<ToolCall> {
        self.calls
            .into_values()
            .filter_map(|call| {
                Some(ToolCall {
                    id: call.id?,
                    name: call.name,
                    arguments: call.arguments,
                })
            })
            .collect()
    }
}
