//! Runs the cross-backend contract suite against the `rblue-mock` backend.
//!
//! The factory builds the scripted fixture documented on
//! [`rblue_api::contract::run_contract_suite`]: a connectable peripheral
//! advertising the contract service with a `READ | WRITE | NOTIFY` echo
//! characteristic. Run with:
//!
//! ```text
//! cargo test -p rblue-api --features contract
//! ```

#![cfg(feature = "contract")]

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
