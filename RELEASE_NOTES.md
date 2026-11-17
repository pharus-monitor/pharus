# Release Notes

## v0.8.1

### Bug Fixes
- **Billing edits apply without restart**: Expiry dates, quotas and other billing fields are now read live from the database instead of being cached at startup. Out-of-band DB edits (bulk migrations, manual maintenance) now take effect on the panel immediately; previously they were invisible until the server restarted.
- **Nodes report IP changes automatically**: Agents now re-report system info when their interface addresses change, so a node that gains, loses or replaces an IP updates the panel within one reporting interval — no agent restart needed. The server already accepted repeated system-info updates; only the agent side needed fixing.

### Changes
- Version bump to 0.8.1

## v0.7.2

### Bug Fixes
- **Fix alert traffic calculation**: Alert rules now correctly account for `traffic_mode` (uni/bidirectional) and `traffic_dir` (up/down/max) settings, matching the billing panel's traffic calculation logic. Previously, alerts used a simple `rx + tx` sum, which could cause false alerts when uni-directional mode was configured.

### Code Quality
- **Extract shared functions**: Moved `platform_string()` and `sha256_hex()` to `pharus-common` crate, eliminating duplication between agent and server.
- **Unify timestamp functions**: Consolidated three separate `now()`/`now_ts()` implementations in `db.rs`, `diag.rs`, and `api.rs` into a single shared `state::now()` function.
- **Remove duplicate code**: Extracted `cmd_output_finish()` helper in agent, replacing three identical `finish` closures in `stream_task`, `stream_iperf3`, and `stream_mtr`.
- **Reuse existing helpers**: `login()` now calls `request_is_secure()` instead of duplicating HTTPS detection logic.

### Changes
- Version bump to 0.7.2
