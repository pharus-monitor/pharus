//! Two-level diagnostic feature toggles: a global default per feature, plus an
//! optional per-agent override. The effective set is mirrored into `AgentState`
//! so request handlers can re-validate without touching the database.

use crate::state::SharedState;
use anyhow::Result;
use pharus_common::BrowserMsg;
use rusqlite::Connection;
use std::collections::HashMap;

/// Looking glass (ping / traceroute), MTR, streaming-unlock checks, ping tasks,
/// custom script tasks.
pub const FEATURES: &[&str] = &["lg", "mtr", "streaming", "ping", "tasks"];

const PREFIX: &str = "feature_";

pub fn is_valid(name: &str) -> bool {
    FEATURES.contains(&name)
}

fn parse(value: &str) -> bool {
    matches!(value, "1" | "true" | "on" | "yes")
}

/// Global defaults. Unset features default to enabled so that turning the
/// toggles on for the first time does not silently disable a working install.
pub fn global_defaults(conn: &Connection) -> Result<HashMap<String, bool>> {
    let mut out = HashMap::new();
    for f in FEATURES {
        let stored = crate::db::get_setting(conn, &format!("{PREFIX}{f}"))?;
        out.insert((*f).to_string(), stored.as_deref().map(parse).unwrap_or(true));
    }
    Ok(out)
}

pub fn set_global(conn: &Connection, name: &str, value: bool) -> Result<()> {
    crate::db::set_setting(conn, &format!("{PREFIX}{name}"), if value { "1" } else { "0" })
}

fn overrides_from(settings: &HashMap<String, String>) -> HashMap<String, bool> {
    settings
        .iter()
        .filter_map(|(k, v)| {
            let f = k.strip_prefix(PREFIX)?;
            is_valid(f).then(|| (f.to_string(), parse(v)))
        })
        .collect()
}

pub fn overrides_for(conn: &Connection, agent_id: i64) -> Result<HashMap<String, bool>> {
    Ok(overrides_from(&crate::db::agent_settings(conn, agent_id)?))
}

/// `None` clears the override so the agent falls back to the global default.
pub fn set_override(conn: &Connection, agent_id: i64, name: &str, value: Option<bool>) -> Result<()> {
    let key = format!("{PREFIX}{name}");
    match value {
        Some(v) => crate::db::set_agent_setting(conn, agent_id, &key, if v { "1" } else { "0" }),
        None => crate::db::clear_agent_setting(conn, agent_id, &key).map(|_| ()),
    }
}

pub fn merge(global: &HashMap<String, bool>, overrides: &HashMap<String, bool>) -> Vec<String> {
    FEATURES
        .iter()
        .filter(|f| {
            *overrides
                .get(**f)
                .or_else(|| global.get(**f))
                .unwrap_or(&true)
        })
        .map(|f| (*f).to_string())
        .collect()
}

pub fn effective_for(conn: &Connection, agent_id: i64) -> Result<Vec<String>> {
    Ok(merge(&global_defaults(conn)?, &overrides_for(conn, agent_id)?))
}

/// Recompute every agent's effective set and broadcast the ones that changed.
/// Called after a global toggle flips.
pub fn refresh_all(state: &SharedState) {
    let computed = {
        let conn = state.db.lock().unwrap();
        let Ok(global) = global_defaults(&conn) else { return };
        let Ok(all) = crate::db::all_agent_settings(&conn) else { return };
        let ids: Vec<i64> = state.agents.read().unwrap().keys().copied().collect();
        ids.into_iter()
            .map(|id| {
                let over = all.get(&id).map(overrides_from).unwrap_or_default();
                (id, merge(&global, &over))
            })
            .collect::<Vec<_>>()
    };

    let changed: Vec<(i64, Vec<String>)> = {
        let mut agents = state.agents.write().unwrap();
        computed
            .into_iter()
            .filter_map(|(id, feats)| {
                let a = agents.get_mut(&id)?;
                (a.features != feats).then(|| {
                    a.features = feats.clone();
                    (id, feats)
                })
            })
            .collect()
    };

    for (agent_id, features) in changed {
        state.broadcast(BrowserMsg::FeaturesUpdate { agent_id, features });
        // Re-sync so an agent that just lost `ping`/`tasks` stops running them
        // instead of waiting for its next reconnect.
        state.push_tasks(agent_id);
    }
}

/// Recompute one agent's effective set, store it and broadcast it.
pub fn refresh_agent(state: &SharedState, agent_id: i64) -> Vec<String> {
    let features = {
        let conn = state.db.lock().unwrap();
        effective_for(&conn, agent_id).unwrap_or_default()
    };
    {
        let mut agents = state.agents.write().unwrap();
        if let Some(a) = agents.get_mut(&agent_id) {
            a.features = features.clone();
        }
    }
    state.broadcast(BrowserMsg::FeaturesUpdate {
        agent_id,
        features: features.clone(),
    });
    state.push_tasks(agent_id);
    features
}

/// Server-side re-validation. The frontend hides disabled controls, but every
/// request that acts on a feature must check here as well.
pub fn enabled(state: &SharedState, agent_id: i64, feature: &str) -> bool {
    state
        .agents
        .read()
        .unwrap()
        .get(&agent_id)
        .is_some_and(|a| a.features.iter().any(|f| f == feature))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(pairs: &[(&str, bool)]) -> HashMap<String, bool> {
        pairs.iter().map(|(k, v)| ((*k).to_string(), *v)).collect()
    }

    #[test]
    fn unset_features_default_to_enabled() {
        assert_eq!(merge(&HashMap::new(), &HashMap::new()), FEATURES.to_vec());
    }

    #[test]
    fn override_beats_global_in_both_directions() {
        let global = map(&[("lg", false), ("mtr", true)]);
        let over = map(&[("lg", true), ("mtr", false)]);
        let got = merge(&global, &over);
        assert!(got.contains(&"lg".to_string()));
        assert!(!got.contains(&"mtr".to_string()));
    }

    #[test]
    fn absent_override_falls_back_to_global() {
        let got = merge(&map(&[("lg", false)]), &HashMap::new());
        assert!(!got.contains(&"lg".to_string()));
        assert!(got.contains(&"mtr".to_string()));
    }

    #[test]
    fn unknown_override_keys_are_ignored() {
        let settings: HashMap<String, String> = [
            ("feature_lg".to_string(), "0".to_string()),
            ("feature_bogus".to_string(), "1".to_string()),
            ("unrelated".to_string(), "1".to_string()),
        ]
        .into_iter()
        .collect();
        let over = overrides_from(&settings);
        assert_eq!(over.len(), 1);
        assert_eq!(over.get("lg"), Some(&false));
    }
}
