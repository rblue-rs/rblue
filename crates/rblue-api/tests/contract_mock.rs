//! Runs the cross-backend contract suite against the `rblue-mock` backend.
//!
//! This is the default CI fixture. `cargo test --workspace` and
//! `cargo test -p rblue-api` both run it; `--features contract` remains a
//! no-op compatibility alias.
//!
//! The factory builds the scripted fixture documented on
//! [`rblue_api::contract::run_contract_suite`]: a connectable peripheral
//! advertising the contract service with a `READ | WRITE | NOTIFY` echo
//! characteristic.

use std::sync::Arc;

use rblue_api::Adapter;

#[tokio::test]
async fn mock_backend_passes_contract_suite() {
    let result = rblue_api::contract::run_contract_suite(|| async {
        let adapter: Arc<dyn Adapter> = Arc::new(rblue_mock::MockAdapter::with_contract_fixture());
        Ok(adapter)
    })
    .await;
    result.expect("rblue-mock must pass the contract suite unchanged");
}
