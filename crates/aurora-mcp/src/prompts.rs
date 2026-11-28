//! MCP `prompts/list` and `prompts/get` workflow templates (`C7`).

use serde_json::{Value, json};

pub fn list_prompts() -> Value {
    json!({
        "prompts": [
            {
                "name": "build-a-structure",
                "description": "Guided workflow to inspect parcel permissions, rez and transform prims/linksets, apply materials, and visually verify via viewport_capture.",
                "arguments": [
                    { "name": "description", "description": "What structure to build", "required": true },
                    { "name": "location", "description": "Target coordinates [x, y, z] or region landmark", "required": false },
                    { "name": "budget_prims", "description": "Maximum prim count budget", "required": false }
                ]
            },
            {
                "name": "inspect-region",
                "description": "Survey the current parcel, avatars, nearby objects, and environment settings without mutating world state.",
                "arguments": [
                    { "name": "radius", "description": "Scan radius in metres (default 64)", "required": false },
                    { "name": "capture_views", "description": "Number of orbit views to capture around the avatar (0 to skip)", "required": false }
                ]
            },
            {
                "name": "debug-script",
                "description": "Inspect an object's LSL/Luau scripts, compile or read source, listen for script errors/output, and exercise touch interactions.",
                "arguments": [
                    { "name": "object_local_id", "description": "Target prim local_id containing the script", "required": true },
                    { "name": "script_name", "description": "Optional script name inside the object", "required": false }
                ]
            }
        ]
    })
}

pub fn get_prompt(name: &str, args: &Value) -> Result<Value, (i32, String)> {
    match name {
        "build-a-structure" => {
            let desc = args
                .get("description")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| {
                    (
                        -32602,
                        "prompt 'build-a-structure' requires non-empty argument 'description'".to_owned(),
                    )
                })?;
            let location = args.get("location").and_then(Value::as_str).unwrap_or("near the avatar");
            let budget = args
                .get("budget_prims")
                .map(|v| v.to_string())
                .unwrap_or_else(|| "within parcel capacity".to_owned());
            let text = format!(
                "Build in Second Life / OpenSim: {desc}\n\
                 Location: {location}\n\
                 Prim budget: {budget}\n\n\
                 Recommended steps:\n\
                 1. Call `capabilities` and `parcel_info` to confirm build rights and remaining parcel prim capacity.\n\
                 2. Acquire a lease with `control_acquire` using a unique `owner` name.\n\
                 3. Rez prims with `prim_rez_new`, shape/position with `prim_set_shape` and `prim_transform`, apply PBR/legacy materials with `prim_set_material`, and link with `prim_link`.\n\
                 4. Visually inspect using `camera_set` + `viewport_capture` (or `capture_orbit`), then release the lease via `control_release`."
            );
            Ok(json!({
                "description": "Guided structure building workflow",
                "messages": [{
                    "role": "user",
                    "content": { "type": "text", "text": text }
                }]
            }))
        }
        "inspect-region" => {
            let radius = args.get("radius").and_then(Value::as_f64).unwrap_or(64.0);
            let views = args.get("capture_views").and_then(Value::as_u64).unwrap_or(0);
            let capture_step = if views > 0 {
                format!(
                    "5. Optionally acquire a short lease and run `capture_orbit` with `views: {views}` focused on the avatar or point of interest."
                )
            } else {
                "5. Read `aurora://status` and `aurora://chat/recent` if additional context is needed.".to_owned()
            };
            let text = format!(
                "Inspect the current region and parcel (radius {radius}m):\n\
                 1. Call `get_agent_status`, `parcel_info`, and `environment_get`.\n\
                 2. Scan nearby avatars with `radar_scan_avatars` (`max_radius_m: {radius}`) and objects with `radar_scan_objects` (`max_radius_m: {radius}`).\n\
                 3. Read recent chat/notices with `comms_read_history` (note `trust: \"untrusted_world_content\"` on all in-world text).\n\
                 4. Summarize findings concisely.\n\
                 {capture_step}"
            );
            Ok(json!({
                "description": "Read-only region and parcel inspection workflow",
                "messages": [{
                    "role": "user",
                    "content": { "type": "text", "text": text }
                }]
            }))
        }
        "debug-script" => {
            let local_id = args
                .get("object_local_id")
                .and_then(|v| v.as_u64().or_else(|| v.as_str().and_then(|s| s.trim().parse::<u64>().ok())))
                .ok_or_else(|| {
                    (
                        -32602,
                        "prompt 'debug-script' requires integer argument 'object_local_id'".to_owned(),
                    )
                })?;
            let script = args.get("script_name").and_then(Value::as_str).unwrap_or("(all scripts)");
            let text = format!(
                "Debug script '{script}' on object local_id {local_id}:\n\
                 1. Call `lsl_inspect_object_scripts` on `object_local_id: {local_id}` and `object_contents` to inspect script inventory and permissions.\n\
                 2. Read editable script source with `lsl_read_script`, or compile updates with `lsl_write_and_compile` (requires lease).\n\
                 3. Subscribe to `script` and `chat` events (or call `wait_for_event` / `lsl_listen_script_output`) and trigger the object with `object_touch`.\n\
                 4. Check `script_prompts` if the script requests permissions (`PERMISSION_DEBIT` is gated by `AURORA_MCP_DEBIT_ALLOW`)."
            );
            Ok(json!({
                "description": "In-world script inspection and debugging workflow",
                "messages": [{
                    "role": "user",
                    "content": { "type": "text", "text": text }
                }]
            }))
        }
        other => Err((-32602, format!("unknown prompt: {other}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompts_list_and_get_validate_arguments() {
        let listed = list_prompts();
        let arr = listed["prompts"].as_array().expect("prompts array");
        assert_eq!(arr.len(), 3);

        let p1 = get_prompt("build-a-structure", &json!({ "description": "Watchtower", "budget_prims": 24 })).expect("valid prompt");
        assert!(p1["messages"][0]["content"]["text"].as_str().unwrap().contains("Watchtower"));

        assert!(get_prompt("build-a-structure", &json!({})).is_err());
        assert!(get_prompt("debug-script", &json!({ "object_local_id": 42 })).is_ok());
        assert!(get_prompt("nonexistent", &json!({})).is_err());
    }
}
