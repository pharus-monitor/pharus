use crate::state::SharedState;
use crate::{billing, db};
use axum::{
    extract::{Path, Request, State},
    http::StatusCode,
    middleware::{self, Next},
    response::{IntoResponse, Json, Response},
    routing::{post, put},
    Router,
};
use chrono::TimeZone;
use pharus_common::{BillingCycle, BillingInfo, BrowserMsg, Currency, TrafficUsage};
use serde::Deserialize;

pub fn router(state: SharedState) -> Router<SharedState> {
    Router::new()
        .route("/api/admin/check", post(check))
        .route("/api/admin/agents/:id/billing", put(update_billing))
        .route_layer(middleware::from_fn_with_state(state, require_admin))
}

fn err(status: StatusCode, msg: &str) -> Response {
    (status, Json(serde_json::json!({ "error": msg }))).into_response()
}

async fn require_admin(State(state): State<SharedState>, req: Request, next: Next) -> Response {
    let Some(expected) = &state.admin_token else {
        return err(StatusCode::NOT_FOUND, "admin api disabled");
    };
    let ok = req
        .headers()
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(|t| t == expected)
        .unwrap_or(false);
    if !ok {
        return err(StatusCode::UNAUTHORIZED, "invalid admin token");
    }
    next.run(req).await
}

/// Token probe for the frontend: reaching this handler means the token is valid.
async fn check() -> StatusCode {
    StatusCode::OK
}

#[derive(Debug, Deserialize)]
pub struct BillingUpdate {
    pub reset_day: Option<u8>,
    pub quota_gb: Option<f64>,
    pub expires_on: Option<String>,
    pub price: Option<f64>,
    pub currency: Option<Currency>,
    pub cycle: Option<BillingCycle>,
}

async fn update_billing(
    State(state): State<SharedState>,
    Path(agent_id): Path<i64>,
    Json(body): Json<BillingUpdate>,
) -> Response {
    if let Some(rd) = body.reset_day {
        if !(1..=31).contains(&rd) {
            return err(StatusCode::UNPROCESSABLE_ENTITY, "reset_day must be within 1..=31");
        }
    }
    if let Some(g) = body.quota_gb {
        if !g.is_finite() || g <= 0.0 {
            return err(StatusCode::UNPROCESSABLE_ENTITY, "quota_gb must be positive");
        }
    }
    if let Some(p) = body.price {
        if !p.is_finite() || p < 0.0 {
            return err(StatusCode::UNPROCESSABLE_ENTITY, "price must be >= 0");
        }
    }
    let expires_at = match &body.expires_on {
        Some(s) => {
            let Ok(date) = chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d") else {
                return err(StatusCode::UNPROCESSABLE_ENTITY, "expires_on must be YYYY-MM-DD");
            };
            let Some(dt) = date.and_hms_opt(0, 0, 0) else {
                return err(StatusCode::UNPROCESSABLE_ENTITY, "invalid expires_on");
            };
            let Some(local) = chrono::Local.from_local_datetime(&dt).earliest() else {
                return err(StatusCode::UNPROCESSABLE_ENTITY, "invalid expires_on");
            };
            Some(local.timestamp())
        }
        None => None,
    };

    let billing = BillingInfo {
        reset_day: body.reset_day,
        quota_bytes: body.quota_gb.map(|g| (g * 1_073_741_824.0) as u64),
        expires_at,
        price: body.price,
        currency: body.currency,
        cycle: body.cycle,
    };

    let rows = {
        let conn = state.db.lock().unwrap();
        match db::set_billing(&conn, agent_id, &billing) {
            Ok(r) => r,
            Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
        }
    };
    if rows == 0 {
        return err(StatusCode::NOT_FOUND, "agent not found");
    }

    let traffic = {
        let mut agents = state.agents.write().unwrap();
        agents.get_mut(&agent_id).map(|a| {
            let old_reset = a.billing.as_ref().and_then(|b| b.reset_day);
            if billing.reset_day != old_reset {
                if let Some(rd) = billing.reset_day {
                    // changing the reset day starts a fresh cycle
                    a.traffic.cycle_start = billing::cycle_start_for(rd, chrono::Local::now());
                    a.traffic.rx_bytes = 0;
                    a.traffic.tx_bytes = 0;
                    a.traffic.last_rx_total = None;
                    a.traffic.last_tx_total = None;
                }
            }
            a.billing = Some(billing.clone());
            (
                TrafficUsage {
                    cycle_start: a.traffic.cycle_start,
                    rx_bytes: a.traffic.rx_bytes,
                    tx_bytes: a.traffic.tx_bytes,
                },
                db::TrafficRow {
                    cycle_start: a.traffic.cycle_start,
                    rx_bytes: a.traffic.rx_bytes,
                    tx_bytes: a.traffic.tx_bytes,
                    last_rx_total: a.traffic.last_rx_total,
                    last_tx_total: a.traffic.last_tx_total,
                },
            )
        })
    };

    let (traffic_usage, row) = match traffic {
        Some((u, r)) => (Some(u), Some(r)),
        None => (None, None),
    };
    if let Some(r) = row {
        let conn = state.db.lock().unwrap();
        if let Err(e) = db::upsert_traffic(&conn, agent_id, &r) {
            tracing::warn!(agent_id, error = %e, "traffic flush after billing update failed");
        }
    }

    state.broadcast(BrowserMsg::Billing {
        agent_id,
        billing: Some(billing.clone()),
        traffic: traffic_usage.clone(),
    });

    Json(serde_json::json!({
        "billing": billing,
        "traffic": traffic_usage,
    }))
    .into_response()
}
