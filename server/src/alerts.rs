//! Alert rule evaluation.

use crate::db::{AlertRuleRow, AlertStateRow, TaskRow};
use crate::notify::Notification;
use crate::state::SharedState;
use anyhow::{anyhow, Result};
use pharus_common::Metrics;
use std::collections::{HashMap, HashSet, VecDeque};
use std::time::Duration;
use tokio::time::MissedTickBehavior;

const EVALUATION_INTERVAL_SECS: u64 = 30;
const MAX_SAMPLE_GAP_SECS: i64 = 65;

type AlertKey = (i64, i64);
type LatestTaskResults = HashMap<(i64, i64), CompactTaskResult>;

/// Start the background evaluation loop.
pub fn spawn(state: SharedState) {
    tokio::spawn(async move {
        let persisted = loop {
            match load_persisted_states(&state) {
                Ok(states) => break states,
                Err(error) => {
                    tracing::warn!(%error, "alert state load failed; evaluation is paused");
                    tokio::time::sleep(Duration::from_secs(EVALUATION_INTERVAL_SECS)).await;
                }
            }
        };

        let mut evaluator = Evaluator::new(persisted);
        let mut interval = tokio::time::interval(Duration::from_secs(EVALUATION_INTERVAL_SECS));
        interval.set_missed_tick_behavior(MissedTickBehavior::Skip);

        loop {
            interval.tick().await;
            if let Err(error) = evaluator.tick(&state).await {
                tracing::warn!(%error, "alert evaluation tick failed");
            }
        }
    });
}

fn load_persisted_states(state: &SharedState) -> Result<HashMap<AlertKey, AlertStateRow>> {
    let conn = state
        .db
        .lock()
        .map_err(|_| anyhow!("database lock is poisoned"))?;
    crate::db::load_alert_state(&conn)
}

struct Evaluator {
    windows: HashMap<AlertKey, SampleWindow>,
    states: HashMap<AlertKey, AlertStateRow>,
    offline_since: HashMap<AlertKey, i64>,
    task_failures: HashMap<AlertKey, u32>,
    signatures: HashMap<i64, RuleSignature>,
}

impl Evaluator {
    fn new(states: HashMap<AlertKey, AlertStateRow>) -> Self {
        Self {
            windows: HashMap::new(),
            states,
            offline_since: HashMap::new(),
            task_failures: HashMap::new(),
            signatures: HashMap::new(),
        }
    }

    async fn tick(&mut self, state: &SharedState) -> Result<()> {
        let now = chrono::Utc::now().timestamp();
        let agents = snapshot_agents(state)?;
        let rules = load_rules(state)?;
        self.sync_rule_signatures(&rules);

        let task_observations = if rules.iter().any(|rule| rule.enabled && rule.kind == "task") {
            match load_task_data(state) {
                Ok((tasks, results)) => build_task_observations(&agents, &tasks, &results),
                Err(error) => {
                    tracing::warn!(%error, "task alert inputs could not be loaded");
                    HashMap::new()
                }
            }
        } else {
            HashMap::new()
        };

        let mut active_pairs = HashSet::new();
        for rule in rules.iter().filter(|rule| rule.enabled) {
            if !matches!(rule.kind.as_str(), "metric" | "offline" | "task") {
                tracing::warn!(rule_id = rule.id, "unknown alert rule kind");
                continue;
            }
            if rule.kind == "metric" && !matches!(rule.op.as_str(), ">" | "<") {
                tracing::warn!(rule_id = rule.id, "unknown metric comparison operator");
                continue;
            }

            for agent in agents.iter().filter(|agent| {
                rule.agent_id
                    .is_none_or(|configured_id| configured_id == agent.id)
            }) {
                let key = (rule.id, agent.id);
                active_pairs.insert(key);

                let observation = match rule.kind.as_str() {
                    "metric" => metric_observation(rule, agent),
                    "offline" => Some(self.offline_observation(rule, agent, now)),
                    "task" => {
                        let mut obs = if let Some(tid) = rule.task_id {
                            task_observations.get(&(agent.id, tid)).cloned()
                        } else {
                            task_observations
                                .iter()
                                .find(|((a, _), _)| *a == agent.id)
                                .map(|(_, o)| o.clone())
                        };
                        if let Some(o) = obs.as_mut() {
                            let key = (rule.id, agent.id);
                            let count = self.task_failures.entry(key).or_insert(0);
                            if o.breached {
                                *count += 1;
                                let required = rule.consecutive.max(1) as u32;
                                o.breached = *count >= required;
                                o.observed = format!("连续失败 {count} 次（要求 {required} 次）");
                            } else {
                                *count = 0;
                            }
                        }
                        obs
                    }
                    _ => None,
                };
                let Some(observation) = observation else {
                    continue;
                };

                let status = self.windows.entry(key).or_default().record(
                    now,
                    rule.duration.max(0),
                    normalized_ratio(rule.ratio),
                    observation.breached,
                );
                if let Some(status) = status {
                    self.apply_transition(state, rule, agent, &observation, status, now)
                        .await;
                }
            }
        }

        self.windows.retain(|key, _| active_pairs.contains(key));
        self.offline_since
            .retain(|key, _| active_pairs.contains(key));
        self.task_failures
            .retain(|key, _| active_pairs.contains(key));
        Ok(())
    }

    fn sync_rule_signatures(&mut self, rules: &[AlertRuleRow]) {
        let active_ids: HashSet<i64> = rules
            .iter()
            .filter(|rule| rule.enabled)
            .map(|rule| rule.id)
            .collect();

        for rule in rules.iter().filter(|rule| rule.enabled) {
            let signature = RuleSignature::from(rule);
            if self.signatures.get(&rule.id) != Some(&signature) {
                self.windows.retain(|(rule_id, _), _| *rule_id != rule.id);
                self.offline_since
                    .retain(|(rule_id, _), _| *rule_id != rule.id);
                self.signatures.insert(rule.id, signature);
            }
        }
        self.signatures
            .retain(|rule_id, _| active_ids.contains(rule_id));
        self.windows
            .retain(|(rule_id, _), _| active_ids.contains(rule_id));
        self.offline_since
            .retain(|(rule_id, _), _| active_ids.contains(rule_id));
    }

    fn offline_observation(
        &mut self,
        rule: &AlertRuleRow,
        agent: &AgentSnapshot,
        now: i64,
    ) -> Observation {
        let key = (rule.id, agent.id);
        let grace = grace_seconds(rule.threshold);
        if agent.online {
            self.offline_since.remove(&key);
            return Observation {
                observed: "当前在线（离线 0 秒）".to_string(),
                threshold: format!("离线时长 ≥ {grace} 秒"),
                breached: false,
            };
        }

        let persisted_state = self.states.get(&key);
        let was_firing = persisted_state.is_some_and(|state| state.firing);
        let firing_since = persisted_state.map_or(0, |state| state.since);
        let started_at = self.offline_since.entry(key).or_insert_with(|| {
            // A persisted firing state means the grace period had already elapsed
            // before restart; seed it so a restart cannot produce a false recovery.
            if was_firing && firing_since > 0 && firing_since <= now {
                firing_since.saturating_sub(grace)
            } else {
                now.saturating_sub(if was_firing { grace } else { 0 })
            }
        });
        let elapsed = now.saturating_sub(*started_at);
        Observation {
            observed: if was_firing {
                format!("已离线至少 {elapsed} 秒")
            } else {
                format!("已离线 {elapsed} 秒")
            },
            threshold: format!("离线时长 ≥ {grace} 秒"),
            breached: elapsed >= grace,
        }
    }

    async fn apply_transition(
        &mut self,
        state: &SharedState,
        rule: &AlertRuleRow,
        agent: &AgentSnapshot,
        observation: &Observation,
        status: WindowStatus,
        now: i64,
    ) {
        let key = (rule.id, agent.id);
        let previous = self.states.get(&key).is_some_and(|state| state.firing);
        if previous == status.firing {
            return;
        }
        tracing::info!(
            rule_id = rule.id,
            agent_id = agent.id,
            firing = status.firing,
            "alert transition"
        );

        // Recovery always notifies; repeated firings respect the cooldown so a
        // flapping host does not re-page on every transition.
        let last_notify = self.states.get(&key).map(|s| s.last_notify).unwrap_or(0);
        let notify = !status.firing || now.saturating_sub(last_notify) >= rule.cooldown.max(0);
        let persisted = AlertStateRow {
            firing: status.firing,
            since: if status.firing { now } else { 0 },
            last_notify: if notify { now } else { last_notify },
        };
        self.states.insert(key, persisted.clone());

        let saved = {
            match state.db.lock() {
                Ok(conn) => crate::db::save_alert_state(&conn, rule.id, agent.id, &persisted),
                Err(_) => Err(anyhow!("database lock is poisoned")),
            }
        };
        if let Err(error) = saved {
            tracing::warn!(
                rule_id = rule.id,
                agent_id = agent.id,
                %error,
                "alert state save failed"
            );
        }

        if notify {
            let action = if status.firing { "触发" } else { "恢复" };
            let notification = Notification {
                title: format!("Pharus 告警{action}：{}", rule.name),
                body: format!(
                    "主机：{}\n规则：{}\n观测值：{}\n阈值：{}\n窗口：{} 秒\n窗口超限比例：{:.1}%（要求 ≥ {:.1}%）",
                    agent.name,
                    rule.name,
                    observation.observed,
                    observation.threshold,
                    rule.duration.max(0),
                    status.breach_ratio * 100.0,
                    normalized_ratio(rule.ratio) * 100.0,
                ),
            };
            crate::notify::dispatch(state, &rule.channels, &notification).await;
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
struct RuleSignature {
    kind: String,
    agent_id: Option<i64>,
    metric: Option<String>,
    op: String,
    threshold: u64,
    duration: i64,
    ratio: u64,
}

impl From<&AlertRuleRow> for RuleSignature {
    fn from(rule: &AlertRuleRow) -> Self {
        Self {
            kind: rule.kind.clone(),
            agent_id: rule.agent_id,
            metric: rule.metric.clone(),
            op: rule.op.clone(),
            threshold: rule.threshold.to_bits(),
            duration: rule.duration,
            ratio: rule.ratio.to_bits(),
        }
    }
}

#[derive(Clone)]
struct AgentSnapshot {
    id: i64,
    name: String,
    online: bool,
    data: Option<Metrics>,
    traffic_used: u64,
    traffic_quota: Option<u64>,
    /// Average ping loss across all tasks, percent 0..=100.
    ping_loss: Option<f64>,
}

fn snapshot_agents(state: &SharedState) -> Result<Vec<AgentSnapshot>> {
    let agents = state
        .agents
        .read()
        .map_err(|_| anyhow!("agent state lock is poisoned"))?;
    let mut snapshots: Vec<_> = agents
        .iter()
        .map(|(&id, agent)| AgentSnapshot {
            id,
            name: agent.name.clone(),
            online: agent.online,
            data: agent.data.clone(),
            traffic_used: {
                let traffic = &agent.traffic;
                match agent.billing.as_ref().and_then(|b| b.traffic_mode.as_deref()) {
                    Some("uni") => match agent.billing.as_ref().and_then(|b| b.traffic_dir.as_deref()) {
                        Some("up") => traffic.tx_bytes,
                        Some("max") => traffic.rx_bytes.max(traffic.tx_bytes),
                        _ => traffic.rx_bytes,
                    },
                    _ => traffic.rx_bytes.saturating_add(traffic.tx_bytes),
                }
            },
            traffic_quota: agent
                .billing
                .as_ref()
                .and_then(|billing| billing.quota_bytes),
            ping_loss: {
                let losses: Vec<f64> = agent
                    .pings
                    .iter()
                    .map(|p| p.loss)
                    .filter(|l| *l >= 0.0 && *l <= 1.0)
                    .collect();
                if losses.is_empty() {
                    None
                } else {
                    Some(losses.iter().sum::<f64>() / losses.len() as f64 * 100.0)
                }
            },
        })
        .collect();
    snapshots.sort_unstable_by_key(|agent| agent.id);
    Ok(snapshots)
}

fn load_rules(state: &SharedState) -> Result<Vec<AlertRuleRow>> {
    let conn = state
        .db
        .lock()
        .map_err(|_| anyhow!("database lock is poisoned"))?;
    crate::db::list_alert_rules(&conn)
}

#[derive(Clone, Copy)]
struct CompactTaskResult {
    #[allow(dead_code)]
    ts: i64,
    exit_code: i32,
}

fn load_task_data(state: &SharedState) -> Result<(Vec<TaskRow>, LatestTaskResults)> {
    let conn = state
        .db
        .lock()
        .map_err(|_| anyhow!("database lock is poisoned"))?;
    let tasks: Vec<_> = crate::db::list_tasks(&conn)?
        .into_iter()
        .filter(|task| task.enabled)
        .collect();
    let latest = crate::db::latest_task_results(&conn)?
        .into_iter()
        .map(|(key, (ts, exit_code))| (key, CompactTaskResult { ts, exit_code }))
        .collect();
    Ok((tasks, latest))
}

#[derive(Clone)]
struct Observation {
    observed: String,
    threshold: String,
    breached: bool,
}

fn metric_observation(rule: &AlertRuleRow, agent: &AgentSnapshot) -> Option<Observation> {
    if !agent.online {
        return None;
    }
    let data = agent.data.as_ref()?;
    let metric = rule.metric.as_deref()?;
    let (label, value, percent) = match metric {
        "cpu" => ("CPU", f64::from(data.cpu_usage), true),
        "mem" => ("内存", percentage(data.mem_used, data.mem_total)?, true),
        "disk" => ("磁盘", percentage(data.disk_used, data.disk_total)?, true),
        "load" => ("1 分钟负载", data.load1, false),
        "traffic" => (
            "流量配额使用率",
            percentage(agent.traffic_used, agent.traffic_quota?)?,
            true,
        ),
        "loss" => ("丢包率", agent.ping_loss?, true),
        _ => return None,
    };
    let breached = compare(rule.op.as_str(), value, rule.threshold)?;
    let (observed, threshold) = if percent {
        (
            format!("{label} {value:.2}%"),
            format!("{label} {} {:.2}%", rule.op, rule.threshold),
        )
    } else {
        (
            format!("{label} {value:.2}"),
            format!("{label} {} {:.2}", rule.op, rule.threshold),
        )
    };
    Some(Observation {
        observed,
        threshold,
        breached,
    })
}

fn build_task_observations(
    agents: &[AgentSnapshot],
    tasks: &[TaskRow],
    latest: &LatestTaskResults,
) -> HashMap<(i64, i64), Observation> {
    let mut observations = HashMap::new();
    for agent in agents {
        for task in tasks.iter().filter(|task| {
            task.agent_id
                .is_none_or(|configured_id| configured_id == agent.id)
        }) {
            let Some(&result) = latest.get(&(task.id, agent.id)) else {
                continue;
            };
            let breached = result.exit_code != 0;
            observations.insert(
                (agent.id, task.id),
                Observation {
                    observed: if breached {
                        format!("任务“{}”退出码 {}", task.name, result.exit_code)
                    } else {
                        format!("任务“{}”最近退出码 0", task.name)
                    },
                    threshold: "任务退出码 ≠ 0".to_string(),
                    breached,
                },
            );
        }
    }
    observations
}

fn percentage(used: u64, total: u64) -> Option<f64> {
    if total == 0 {
        None
    } else {
        Some(used as f64 / total as f64 * 100.0)
    }
}

fn compare(op: &str, value: f64, threshold: f64) -> Option<bool> {
    match op {
        ">" => Some(value > threshold),
        "<" => Some(value < threshold),
        _ => None,
    }
}

fn grace_seconds(threshold: f64) -> i64 {
    if !threshold.is_finite() || threshold <= 0.0 {
        0
    } else if threshold >= i64::MAX as f64 {
        i64::MAX
    } else {
        threshold.ceil() as i64
    }
}

fn normalized_ratio(ratio: f64) -> f64 {
    if ratio.is_finite() {
        ratio.clamp(0.0, 1.0)
    } else {
        1.0
    }
}

#[derive(Debug, Clone, Copy)]
struct TimedSample {
    ts: i64,
    breached: bool,
}

#[derive(Debug, Default)]
struct SampleWindow {
    started_at: Option<i64>,
    samples: VecDeque<TimedSample>,
}

#[derive(Debug, Clone, Copy)]
struct WindowStatus {
    breach_ratio: f64,
    firing: bool,
}

impl SampleWindow {
    fn record(
        &mut self,
        now: i64,
        duration: i64,
        required_ratio: f64,
        breached: bool,
    ) -> Option<WindowStatus> {
        if self
            .samples
            .back()
            .is_some_and(|last| last.ts > now || now.saturating_sub(last.ts) > MAX_SAMPLE_GAP_SECS)
        {
            self.samples.clear();
            self.started_at = None;
        }

        self.started_at.get_or_insert(now);
        if let Some(last) = self.samples.back_mut().filter(|last| last.ts == now) {
            last.breached = breached;
        } else {
            self.samples.push_back(TimedSample { ts: now, breached });
        }

        let cutoff = now.saturating_sub(duration);
        while self
            .samples
            .front()
            .is_some_and(|sample| sample.ts < cutoff)
        {
            self.samples.pop_front();
        }

        if now.saturating_sub(self.started_at?) < duration || self.samples.is_empty() {
            return None;
        }
        let breach_count = self.samples.iter().filter(|sample| sample.breached).count();
        let breach_ratio = breach_count as f64 / self.samples.len() as f64;
        Some(WindowStatus {
            breach_ratio,
            // A zero ratio still requires at least one actual breach; otherwise a
            // healthy rule would mathematically satisfy a threshold of zero.
            firing: breach_count > 0 && breach_ratio + f64::EPSILON >= required_ratio,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::SampleWindow;

    #[test]
    fn waits_for_the_complete_observation_window() {
        let mut window = SampleWindow::default();
        assert!(window.record(0, 60, 1.0, true).is_none());
        assert!(window.record(30, 60, 1.0, true).is_none());

        let status = window.record(60, 60, 1.0, true).unwrap();
        assert!(status.firing);
        assert_eq!(status.breach_ratio, 1.0);
    }

    #[test]
    fn ratio_one_requires_every_sample_to_breach() {
        let mut window = SampleWindow::default();
        assert!(window.record(0, 60, 1.0, true).is_none());
        assert!(window.record(30, 60, 1.0, false).is_none());

        let status = window.record(60, 60, 1.0, true).unwrap();
        assert!(!status.firing);
        assert!((status.breach_ratio - (2.0 / 3.0)).abs() < f64::EPSILON);
    }

    #[test]
    fn half_ratio_fires_then_recovers_as_samples_expire() {
        let mut window = SampleWindow::default();
        assert!(window.record(0, 60, 0.5, true).is_none());
        assert!(window.record(30, 60, 0.5, false).is_none());

        let firing = window.record(60, 60, 0.5, true).unwrap();
        assert!(firing.firing);

        let recovered = window.record(90, 60, 0.5, false).unwrap();
        assert!(!recovered.firing);
        assert!((recovered.breach_ratio - (1.0 / 3.0)).abs() < f64::EPSILON);
    }

    #[test]
    fn zero_ratio_still_requires_a_real_breach() {
        let mut window = SampleWindow::default();
        assert!(window.record(0, 30, 0.0, false).is_none());
        let healthy = window.record(30, 30, 0.0, false).unwrap();
        assert!(!healthy.firing);

        let breached = window.record(60, 30, 0.0, true).unwrap();
        assert!(breached.firing);
    }

    #[test]
    fn a_sampling_gap_restarts_window_maturity() {
        let mut window = SampleWindow::default();
        assert!(window.record(0, 60, 1.0, true).is_none());
        assert!(window.record(30, 60, 1.0, true).is_none());
        assert!(window.record(120, 60, 1.0, true).is_none());
        assert!(window.record(150, 60, 1.0, true).is_none());
        assert!(window.record(180, 60, 1.0, true).unwrap().firing);
    }
}
