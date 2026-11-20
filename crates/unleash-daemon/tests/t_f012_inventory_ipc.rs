//! T-F012-013 / REQ-MODEL-013: passive inventory through the actual UDS service.
//! Synthetic loopback endpoint only; no live provider, vault or model execution.

#[path = "support/provider_probe_inventory_fixture.rs"]
mod fixture;
#[path = "support/provider_probe_inventory_oracles.rs"]
mod oracles;

use fixture::{InventoryFixture, TestResult, require};
use serde_json::json;
use std::time::Duration;
use unleash_client::{ClientError, UnleashClient};
use unleash_protocol::{ErrorCode, ReceiptState, SubscribeEventsRequest};
use uuid::Uuid;

#[tokio::test]
async fn t_f012_013_inventory_active_mandate_gets_one_durable_list() -> TestResult {
    let (fixture, mut admin) = InventoryFixture::start().await?;
    let result = nominal_inventory(&fixture, &mut admin).await;
    drop(admin);
    fixture.stop().await?;
    result
}

async fn nominal_inventory(fixture: &InventoryFixture, admin: &mut UnleashClient) -> TestResult {
    oracles::validate_network_query(&fixture.db()?)?;
    let profile = Uuid::from_u128(0xf012_1a01);
    let configured = fixture.configure(admin, profile, 1).await?;
    let mandate = fixture
        .issue(admin, profile, &configured, "inventory", 2, 0)
        .await?;
    fixture.assert_no_probe_effect()?;
    let mut events = admin
        .subscribe_events(&SubscribeEventsRequest {
            stream_id: mandate.event_cursor.stream_id,
            after_sequence: mandate.event_cursor.sequence,
            project_id: None,
            aggregate_id: None,
            batch_limit: 64,
        })
        .await?;
    let request = fixture::probe(admin, profile, 3, &mandate, "inventory");
    let capture = fixture.capture(oracles::NetworkExpectation {
        request: request.clone(),
        configured: configured.clone(),
        mandate: mandate.clone(),
        profile,
    })?;
    let receipt = match admin.send_command(&request).await {
        Ok(receipt) => receipt,
        Err(ClientError::Remote(remote)) if remote.code == ErrorCode::InvalidArgument => {
            fixture.assert_no_probe_effect()?;
            return Err(
                "inventory route still closed: InvalidArgument after real profile and mandate"
                    .into(),
            );
        }
        Err(error) => return Err(error.into()),
    };
    require(
        receipt.state == ReceiptState::Accepted,
        "inventory receipt must be accepted",
    )?;
    require(
        receipt.result_ref == Some(format!("provider-probe:{}", request.aggregate_id)),
        "inventory result must refer to this probe",
    )?;
    let captured = capture.finish().await?;
    captured.assert_one_get(fixture.endpoint_addr()?)?;
    let view = oracles::wait_for_inventory_view(
        admin,
        profile,
        request.aggregate_id,
        &receipt,
        &mut events,
    )
    .await?;
    require(
        view.result.get("qualification") == Some(&json!("required")),
        "a native model list does not qualify inference",
    )?;
    let proof =
        oracles::assert_committed_inventory(fixture.data_root(), &request, &receipt, &captured)?;
    let cursor = view
        .watermark
        .ok_or("inventory view must carry a durable watermark")?;
    require(
        cursor.stream_id.to_string() == proof.stream_id
            && cursor.sequence.as_u64() >= proof.event_sequence,
        "view watermark must cover the inventory commit",
    )?;
    let before = fixture.probe_counts(request.aggregate_id)?;
    let replay =
        tokio::time::timeout(Duration::from_secs(3), admin.send_command(&request)).await??;
    require(
        replay == receipt,
        "exact inventory replay must return the original receipt",
    )?;
    require(
        fixture.probe_counts(request.aggregate_id)? == before,
        "inventory replay must not create another effect or durable row",
    )?;
    fixture.assert_no_pending_connection()?;
    oracles::assert_no_model_or_secret_rows(&fixture.db()?)?;
    Ok(())
}
