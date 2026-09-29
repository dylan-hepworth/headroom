// Agent plans: the teams laid out in the planner (see Planner.tsx), saved as templates to start on any chat. One file
// each, in the plans folder in Headroom's folder, named by the plan's ID, never its name, which is only in the file.
// "My company", an example team, is put there the first time, and left alone after that.

use std::{
    fs,
    path::{Path, PathBuf},
};

use serde_json::Value;

const EXAMPLE: &str = include_str!("../../src/my-company.json");

fn dir(config: &Path) -> PathBuf {
    config.join("plans")
}

/// An ID that's safe as a file name: letters, digits, and dashes, not too long.
fn safe(id: &str) -> Option<&str> {
    let ok = !id.is_empty() && id.len() <= 64 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
    ok.then_some(id)
}

/// Does it look like a plan the planner can open?
fn valid(plan: &Value) -> bool {
    plan["id"].as_str().and_then(safe).is_some()
        && plan["name"].is_string()
        && plan["agents"].as_array().is_some_and(|a| a.iter().all(|a| a["id"].is_string() && a["name"].is_string()))
        && plan["edges"].is_array()
}

/// The saved plans, in the order they were made. The first time, the example's put there to start from.
pub fn list(config: &Path) -> Vec<Value> {
    let dir = dir(config);
    if !dir.exists() && fs::create_dir_all(&dir).is_ok() {
        if let Ok(example) = serde_json::from_str::<Value>(EXAMPLE) {
            let _ = save(config, &example);
        }
    }
    let mut plans: Vec<(std::time::SystemTime, Value)> = fs::read_dir(&dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .filter_map(|e| {
            let plan: Value = serde_json::from_str(&fs::read_to_string(e.path()).ok()?).ok()?;
            let made = e.metadata().and_then(|m| m.created()).unwrap_or(std::time::UNIX_EPOCH);
            valid(&plan).then_some((made, plan))
        })
        .collect();
    plans.sort_by_key(|(made, _)| *made);
    plans.into_iter().map(|(_, plan)| plan).collect()
}

/// Save a plan, over the one with its ID if there is one. Written under another name and renamed into place, so a
/// half-written file is never read.
pub fn save(config: &Path, plan: &Value) -> Result<(), String> {
    if !valid(plan) {
        return Err("That isn't a plan Headroom can save".into());
    }
    let id = plan["id"].as_str().and_then(safe).ok_or("That plan's ID won't do as a file name")?;
    let dir = dir(config);
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let temp = dir.join(format!(".{id}.tmp"));
    fs::write(&temp, serde_json::to_string_pretty(plan).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    fs::rename(&temp, dir.join(format!("{id}.json"))).map_err(|e| e.to_string())
}

/// Where a plan's file is, for a chat to change it while planning with the user (see Together.tsx).
pub fn path(config: &Path, id: &str) -> Option<PathBuf> {
    Some(dir(config).join(format!("{}.json", safe(id)?)))
}

/// A plan's file as it is now, whoever last wrote it.
pub fn read(config: &Path, id: &str) -> Option<String> {
    fs::read_to_string(path(config, id)?).ok()
}

pub fn delete(config: &Path, id: &str) -> Result<(), String> {
    let id = safe(id).ok_or("That plan's ID won't do as a file name")?;
    fs::remove_file(dir(config).join(format!("{id}.json"))).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn plans_are_saved_by_id_with_the_example_to_start() {
        let config = std::env::temp_dir().join(format!("headroom-plans-{}", std::process::id()));
        let first = list(&config);
        assert_eq!(first.len(), 1);
        assert_eq!(first[0]["name"], "My company");

        let plan = json!({ "id": "a1", "name": "../../escape", "agents": [], "edges": [] });
        save(&config, &plan).unwrap();
        assert!(config.join("plans/a1.json").exists(), "named by its ID, not its name");
        assert_eq!(list(&config).len(), 2);

        // The example isn't put back once it's gone
        delete(&config, "my-company").unwrap();
        assert_eq!(list(&config).len(), 1);
        assert!(save(&config, &json!({ "id": "../x", "name": "x", "agents": [], "edges": [] })).is_err());
        assert!(delete(&config, "../settings").is_err());
        let _ = fs::remove_dir_all(config);
    }
}
